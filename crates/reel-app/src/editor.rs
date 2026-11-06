//! The editor's shell around the preview. Milestone 6 builds this out.

use std::path::{Path, PathBuf};
use std::time::Duration;

use dioxus::desktop::tao::event::{Event, WindowEvent};
use dioxus::desktop::use_wry_event_handler;
use dioxus::prelude::*;
use reel_core::format;
use reel_core::project::{Look, MediaRef, Project};
use reel_core::recordly::{self, ImportOptions};
use reel_core::time::{Micros, TimelineTime};
use reel_core::timeline::Timeline;
use reel_media::macos::probe;

use crate::Home;
use crate::preview::{self, PixelRect, SHARED};
use crate::ui::button::{Button, ButtonSize, ButtonVariant};
use crate::ui::slider::Slider;

/// Reports where the clear area is whenever it moves or changes size.
const OBSERVE_HOLE: &str = r"
const start = () => {
    const hole = document.getElementById('preview-hole');
    if (!hole) { setTimeout(start, 20); return; }
    const send = () => {
        const r = hole.getBoundingClientRect();
        dioxus.send({ x: r.x, y: r.y, w: r.width, h: r.height, dpr: devicePixelRatio });
    };
    new ResizeObserver(send).observe(hole);
    addEventListener('resize', send);
    send();
};
start();
await new Promise(() => {});
";

fn pixel_rect(value: &serde_json::Value) -> PixelRect {
    let number = |key: &str| value[key].as_f64().unwrap_or(0.0);
    let scale = number("dpr").max(1.0);
    let physical = |css: f64| (css * scale).round().max(0.0) as u32;
    PixelRect {
        x: physical(number("x")),
        y: physical(number("y")),
        width: physical(number("w")),
        height: physical(number("h")),
    }
}

/// Opening options from the command line: `reel-app [file] [--look look.json]`.
struct Launch {
    file: Option<PathBuf>,
    look: Option<PathBuf>,
}

fn launch_options() -> Launch {
    let mut launch = Launch {
        file: None,
        look: None,
    };
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--look" {
            launch.look = args.next().map(PathBuf::from);
        } else if !arg.starts_with('-') {
            launch.file = Some(PathBuf::from(arg));
        }
    }
    launch
}

/// Reads a file into a project, whichever kind of file it is.
fn load_project(path: &Path, look: Option<&Path>) -> Result<(Project, Option<PathBuf>), String> {
    let extension = path
        .extension()
        .map(|ext| ext.to_string_lossy().to_ascii_lowercase());
    let folder = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .map(Path::to_path_buf);
    let read = || {
        std::fs::read_to_string(path)
            .map_err(|error| format!("Couldn't read {}: {error}", path.display()))
    };
    let (mut project, folder) = match extension.as_deref() {
        Some(format::EXTENSION) => (
            format::from_str(&read()?).map_err(|error| error.to_string())?,
            folder,
        ),
        Some("recordly" | "openscreen") => {
            let options = ImportOptions {
                source_duration: None,
                find_sidecars: true,
            };
            let imported =
                recordly::import(&read()?, options).map_err(|error| error.to_string())?;
            (imported.project, folder)
        }
        _ => {
            // A plain video, opened as a new project with one clip.
            let info = probe(path).map_err(|error| error.0)?;
            let absolute = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
            let mut project = Project {
                id: recordly::new_id(),
                timeline: Timeline::for_recording(info.duration),
                ..Project::default()
            };
            project.media.video = MediaRef::absolute(absolute);
            (project, None)
        }
    };
    if let Some(look) = look {
        let text = std::fs::read_to_string(look)
            .map_err(|error| format!("Couldn't read {}: {error}", look.display()))?;
        project.look = serde_json::from_str::<Look>(&text)
            .map_err(|error| format!("{}: {error}", look.display()))?;
    }
    Ok((project, folder))
}

/// Opens a file without tying up the UI thread, since sound has to be decoded first.
async fn open_file(path: PathBuf, look: Option<PathBuf>) -> Result<(), String> {
    let prepared = tokio::task::spawn_blocking(move || {
        let (project, folder) = load_project(&path, look.as_deref())?;
        preview::prepare(project, folder.as_deref())
    })
    .await
    .map_err(|error| error.to_string())??;
    preview::show(prepared);
    Ok(())
}

fn clock_text(time: Micros) -> String {
    let total = time.secs().max(0.0);
    let minutes = (total / 60.0) as u64;
    let seconds = total - minutes as f64 * 60.0;
    format!("{minutes}:{seconds:05.2}")
}

#[component]
pub fn Editor(dark: bool, toggle_theme: EventHandler<()>) -> Element {
    let mut project_open = use_signal(|| false);
    let mut message = use_signal(|| None::<String>);
    let mut playing = use_signal(|| false);
    let mut position = use_signal(|| Micros::ZERO);
    let mut duration = use_signal(|| Micros::ZERO);

    use_wry_event_handler(|event, _| {
        if let Event::WindowEvent {
            event: WindowEvent::Resized(size),
            ..
        } = event
        {
            SHARED.set_surface_size(size.width, size.height);
        }
    });

    // Opens anything given on the command line.
    use_future(move || async move {
        let launch = launch_options();
        if let Some(file) = launch.file {
            match open_file(file, launch.look).await {
                Ok(()) => {
                    duration.set(preview::duration());
                    project_open.set(true);
                }
                Err(error) => message.set(Some(error)),
            }
        }
        #[cfg(feature = "self-test")]
        if std::env::var_os("REEL_SELF_TEST").is_some() {
            crate::self_test::run().await;
        }
    });

    use_future(move || async move {
        let mut reports = document::eval(OBSERVE_HOLE);
        while let Ok(value) = reports.recv::<serde_json::Value>().await {
            SHARED.set_hole(pixel_rect(&value));
        }
    });

    // Keeps the readout in step. A sixtieth of a second is plenty, and T4 showed it costs little.
    use_future(move || async move {
        let mut clock = tokio::time::interval(Duration::from_secs_f64(1.0 / 60.0));
        loop {
            clock.tick().await;
            let now = SHARED.position() - TimelineTime::ZERO;
            if position() != now {
                position.set(now);
            }
            let is_playing = SHARED.is_playing();
            if playing() != is_playing {
                playing.set(is_playing);
            }
        }
    });

    let choose_file = move |_| {
        let picked = rfd::FileDialog::new()
            .set_title("Open a video or project")
            .add_filter(
                "Videos and projects",
                &["mp4", "mov", "m4v", "reel", "recordly", "openscreen"],
            )
            .pick_file();
        if let Some(path) = picked {
            spawn(async move {
                message.set(None);
                match open_file(path, None).await {
                    Ok(()) => {
                        duration.set(preview::duration());
                        project_open.set(true);
                    }
                    Err(error) => message.set(Some(error)),
                }
            });
        }
    };

    let toggle_playback = move |_| {
        if playing() {
            preview::pause();
        } else {
            preview::play();
        }
    };

    if !project_open() {
        return rsx! {
            Home { dark, toggle_theme,
                if let Some(text) = message() {
                    p { class: "home-message", role: "alert", "{text}" }
                }
                Button { id: "open-file", onclick: choose_file, "Open a video or project" }
            }
        };
    }

    let total = duration();
    let total_ms = total.ms().max(1.0);
    let label = if playing() { "Pause" } else { "Play" };

    rsx! {
        div {
            class: "shell",
            tabindex: "0",
            onkeydown: move |event| {
                if event.key() == Key::Character(" ".to_string()) {
                    event.prevent_default();
                    if SHARED.is_playing() {
                        preview::pause();
                    } else {
                        preview::play();
                    }
                }
            },
            header { class: "shell-header",
                span { class: "shell-title", "Reel" }
                div { class: "shell-actions",
                    Button { variant: ButtonVariant::Outline, size: ButtonSize::Sm, onclick: choose_file, "Open" }
                    Button {
                        variant: ButtonVariant::Outline,
                        size: ButtonSize::Sm,
                        onclick: move |_| toggle_theme.call(()),
                        if dark { "Light" } else { "Dark" }
                    }
                }
            }
            div { id: "preview-hole", class: "shell-preview" }
            footer { class: "shell-transport",
                Button { id: "play", size: ButtonSize::Sm, onclick: toggle_playback, "{label}" }
                span { id: "position", class: "shell-time", "{clock_text(position())}" }
                div { class: "shell-scrub",
                    Slider {
                        min: 0.0,
                        max: total_ms,
                        step: 1.0,
                        value: position().ms(),
                        oninput: move |event: FormEvent| {
                            if let Ok(ms) = event.value().parse::<f64>() {
                                preview::seek(TimelineTime::from_ms(ms));
                            }
                        },
                    }
                }
                span { class: "shell-time", "{clock_text(total)}" }
                div { class: "shell-volume",
                    Slider {
                        min: 0.0,
                        max: 1.0,
                        step: 0.01,
                        value: 1.0,
                        oninput: move |event: FormEvent| {
                            if let Ok(volume) = event.value().parse::<f64>() {
                                preview::set_volume(volume);
                            }
                        },
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times_read_as_minutes_and_seconds() {
        assert_eq!(clock_text(Micros::from_secs(0)), "0:00.00");
        assert_eq!(clock_text(Micros::from_ms(83_250.0)), "1:23.25");
    }

    #[test]
    fn the_holes_position_scales_with_the_display() {
        let rect = pixel_rect(
            &serde_json::json!({ "x": 10.0, "y": 44.0, "w": 100.5, "h": 50.0, "dpr": 2.0 }),
        );
        assert_eq!(
            rect,
            PixelRect {
                x: 20,
                y: 88,
                width: 201,
                height: 100
            }
        );
    }

    #[test]
    fn a_look_file_applies_to_a_project() {
        let dir = std::env::temp_dir().join(format!("reel-look-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let reel = dir.join("a.reel");
        std::fs::write(&reel, format::to_string(&Project::default())).unwrap();
        let look = dir.join("look.json");
        std::fs::write(&look, r#"{ "shadow": 0.1, "corner_radius": 30.0 }"#).unwrap();
        let (project, _) = load_project(&reel, Some(&look)).unwrap();
        assert_eq!(project.look.shadow, 0.1);
        assert_eq!(project.look.corner_radius, 30.0);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
