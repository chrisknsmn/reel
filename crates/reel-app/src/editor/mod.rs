//! The editor (§7): a header, an icon rail, a settings panel, the preview, and the timeline. Everything
//! drawn in the export is drawn by wgpu, showing through a hole in this page.

mod captions;
mod controls;
mod dialogs;
mod export_panel;
mod keys;
mod math;
mod overlay;
mod panels;
mod state;
mod timeline;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use dioxus::desktop::tao::event::{Event, WindowEvent};
use dioxus::desktop::{use_muda_event_handler, use_wry_event_handler};
use dioxus::prelude::*;
use icons::{
    Captions, Download, FolderOpen, Image, MousePointer2, Pause, Play, Plus, Redo2, Scissors,
    Settings as SettingsIcon, SkipBack, SkipForward, Undo2, Video, Volume2, VolumeX, Wand, ZoomIn,
};
use reel_core::edit::Edit;
use reel_core::format;
use reel_core::history::Selection;
use reel_core::project::{Crop, Look, MediaRef, Project};
use reel_core::recordly::{self, ImportOptions};
use reel_core::session::Session;
use reel_core::settings::SettingsStore;
use reel_core::time::{Micros, TimelineTime};
use reel_core::timeline::{AnnotationContent, ArrowDirection, AudioItem, Timeline, ZoomMode};
use reel_media::macos::{probe, read_pcm};
use reel_motion::suggest::suggest_zooms;

use crate::links::{self, ToBar, ToEditor};
use crate::preview::{self, PixelRect, SHARED};
use crate::recording::support_folder;
use crate::ui::button::Button;
use crate::ui::slider::Slider;

use self::keys::Action;
use self::math::{View, clock_text, skip_target};
pub use self::state::Ed;
use self::state::{Overlay as OpenOverlay, Rail};
use self::timeline::add_zoom_edit;

/// No dialog or side panel open, for tests to go back to.
#[cfg(feature = "self-test")]
pub const CLOSED: OpenOverlay = OpenOverlay::None;

#[cfg(feature = "self-test")]
pub use self::math::SMALLEST_VIEW;

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
fn load_session(path: &Path, look: Option<&Path>) -> Result<(Session, Option<PathBuf>), String> {
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
    let (mut session, folder) = match extension.as_deref() {
        Some(format::EXTENSION) => (
            Session::open(path).map_err(|error| error.to_string())?,
            folder,
        ),
        Some("recordly" | "openscreen") => {
            let options = ImportOptions {
                source_duration: None,
                find_sidecars: true,
            };
            let imported =
                recordly::import(&read()?, options).map_err(|error| error.to_string())?;
            (Session::new(imported.project), folder)
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
            (Session::new(project), None)
        }
    };
    if let Some(look) = look {
        let text = std::fs::read_to_string(look)
            .map_err(|error| format!("Couldn't read {}: {error}", look.display()))?;
        let look: Look =
            serde_json::from_str(&text).map_err(|error| format!("{}: {error}", look.display()))?;
        session.editor.change(None, |project| project.look = look);
    }
    Ok((session, folder))
}

/// A video that wasn't where the project said, and the session waiting for it (§59).
#[derive(Clone)]
struct MissingVideo {
    session: Session,
    folder: Option<PathBuf>,
    path: PathBuf,
}

enum Opened {
    Ready(Box<Session>, preview::Prepared),
    Missing(Box<MissingVideo>),
}

/// Opens a file without tying up the UI thread, since sound has to be decoded first.
async fn open_file(path: PathBuf, look: Option<PathBuf>) -> Result<Opened, String> {
    tokio::task::spawn_blocking(move || {
        let (session, folder) = load_session(&path, look.as_deref())?;
        prepare_session(session, folder)
    })
    .await
    .map_err(|error| error.to_string())?
}

fn prepare_session(session: Session, folder: Option<PathBuf>) -> Result<Opened, String> {
    let project = session.editor.project().clone();
    let video = project.media.video.resolve(folder.as_deref());
    if !video.exists() {
        return Ok(Opened::Missing(Box::new(MissingVideo {
            session,
            folder,
            path: video,
        })));
    }
    let prepared = preview::prepare(project, folder.as_deref())?;
    Ok(Opened::Ready(Box::new(session), prepared))
}

fn settings_path() -> PathBuf {
    support_folder().join("settings.json")
}

/// Makes a change the toolbar and the keyboard share.
fn delete_selected(ed: Ed) {
    let edit = match ed.selection() {
        Selection::None => return,
        Selection::Clip(id) => Edit::DeleteClip { id },
        Selection::Zoom(id) => Edit::DeleteZoom { id },
        Selection::AllZooms => Edit::DeleteAllZooms,
        Selection::Annotation(id) => Edit::DeleteAnnotation { id },
        Selection::Audio(id) => Edit::DeleteAudio { id },
        Selection::Caption(id) => Edit::DeleteCaption { id },
    };
    if ed.apply(&edit).is_some() {
        ed.select(Selection::None);
    }
}

fn add_annotation(ed: Ed, content: AnnotationContent) {
    let Some(project) = ed.project() else { return };
    let id = project.timeline.new_id("annotation");
    let annotation = panels::new_annotation(id, ed.now(), content);
    if let Some(outcome) = ed.apply(&Edit::AddAnnotation { annotation })
        && let Some(created) = outcome.created
    {
        ed.select(Selection::Annotation(created));
    }
}

fn add_zoom(ed: Ed) {
    if let Some(outcome) = ed.apply(&add_zoom_edit(ed.now()))
        && let Some(created) = outcome.created
    {
        ed.select(Selection::Zoom(created));
    }
}

/// Picks a sound file and puts it at the playhead, on a row of its own (§16).
fn add_audio(ed: Ed) {
    let picked = rfd::FileDialog::new()
        .set_title("Add audio")
        .add_filter(
            "Audio",
            &["mp3", "wav", "aac", "m4a", "flac", "ogg", "aiff"],
        )
        .pick_file();
    let Some(path) = picked else { return };
    let mut notice = ed.notice;
    spawn(async move {
        let file = path.clone();
        let length = tokio::task::spawn_blocking(move || {
            read_pcm(&file).map(|pcm| {
                pcm.map(|pcm| Micros((pcm.frames() as f64 / 48_000.0 * 1_000_000.0) as i64))
            })
        })
        .await;
        let Ok(Ok(Some(length))) = length else {
            notice.set(Some(
                "That file has no sound that Reel can read.".to_string(),
            ));
            return;
        };
        let Some(project) = ed.project() else { return };
        let start = ed.now();
        let end = (start + length).min(TimelineTime::ZERO + project.timeline.duration());
        let track = project
            .timeline
            .audio
            .iter()
            .map(|item| item.track + 1)
            .max()
            .unwrap_or(0);
        let item = AudioItem {
            id: project.timeline.new_id("audio"),
            start,
            end,
            file: MediaRef::absolute(path),
            file_duration: length,
            volume: 1.0,
            boost: false,
            track,
        };
        if let Some(outcome) = ed.apply(&Edit::AddAudio { item })
            && let Some(created) = outcome.created
        {
            ed.select(Selection::Audio(created));
        }
    });
}

/// Adds zooms around clusters of clicks. Recordly disables this when auto-apply is off, and blames
/// cursor looping (§58). All it really needs is cursor data.
fn suggest(mut ed: Ed) {
    let Some(track) = preview::cursor_track() else {
        ed.notice.set(Some(
            "This video has no cursor data to suggest zooms from.".to_string(),
        ));
        return;
    };
    let Some(project) = ed.project() else { return };
    let suggestions = suggest_zooms(&track, &project.timeline);
    if suggestions.is_empty() {
        ed.notice.set(Some(
            "There are no clusters of clicks to zoom in on.".to_string(),
        ));
        return;
    }
    ed.change("suggest-zooms", |p| {
        for suggestion in &suggestions {
            let _ = p.timeline.apply(&Edit::AddZoom {
                start: suggestion.start,
                depth: 1,
                focus: suggestion.focus,
                mode: ZoomMode::Auto,
            });
        }
    });
    ed.end_gesture();
}

fn save_as(mut ed: Ed) {
    let name = dialogs::project_name(ed.session.read().as_ref().and_then(Session::path));
    let picked = rfd::FileDialog::new()
        .set_title("Save the project")
        .set_directory(dialogs::projects_folder())
        .set_file_name(format!("{name}.{}", format::EXTENSION))
        .add_filter("Reel projects", &[format::EXTENSION])
        .save_file();
    let Some(path) = picked else { return };
    let result = ed
        .session
        .write()
        .as_mut()
        .map(|session| session.save_as(&path));
    if let Some(Err(error)) = result {
        ed.notice.set(Some(error.to_string()));
    } else {
        remember_project(&path);
        ed.notice.set(Some("Saved.".to_string()));
    }
}

fn remember_project(path: &Path) {
    let mut store = SettingsStore::open(&settings_path());
    let _ = store.update(|settings| settings.remember_project(path));
}

fn save(mut ed: Ed) {
    let has_path = ed
        .session
        .read()
        .as_ref()
        .is_some_and(|session| session.path().is_some());
    if !has_path {
        save_as(ed);
        return;
    }
    let result = ed.session.write().as_mut().map(Session::save);
    match result {
        Some(Err(error)) => ed.notice.set(Some(error.to_string())),
        _ => ed.notice.set(Some("Saved.".to_string())),
    }
}

/// Does what a key means.
fn run_action(ed: Ed, action: Action) {
    let mut overlay = ed.overlay;
    match action {
        Action::AddZoom => add_zoom(ed),
        Action::Split => {
            ed.apply(&Edit::SplitClip { at: ed.now() });
        }
        Action::AddAnnotation => add_annotation(
            ed,
            AnnotationContent::Text {
                text: "Text".to_string(),
                style: reel_core::timeline::TextStyle::default(),
            },
        ),
        Action::AddKeyframe => {
            let now = ed.now();
            let mut keyframes = ed.keyframes;
            if !keyframes().contains(&now) {
                keyframes.write().push(now);
            }
        }
        Action::DeleteSelected => delete_selected(ed),
        Action::PlayPause => {
            if SHARED.is_playing() {
                preview::pause();
            } else {
                preview::play();
            }
        }
        Action::Undo => ed.undo(),
        Action::Redo => ed.redo(),
        Action::SelectAllZooms => ed.select(Selection::AllZooms),
        Action::NextAnnotation | Action::PreviousAnnotation => {
            let Some(project) = ed.project() else { return };
            let selection = ed.selection();
            let selected = match &selection {
                Selection::Annotation(id) => Some(id),
                _ => None,
            };
            let backward = action == Action::PreviousAnnotation;
            if let Some(next) = overlay::cycle_annotation(
                &project.timeline.annotations,
                ed.now(),
                selected,
                backward,
            ) {
                ed.select(Selection::Annotation(next));
            }
        }
        Action::Cancel => {
            if overlay() == OpenOverlay::None {
                ed.select(Selection::None);
            } else {
                overlay.set(OpenOverlay::None);
            }
        }
        Action::OpenProjects => overlay.set(OpenOverlay::Projects),
        Action::Save => save(ed),
        Action::SaveAs => save_as(ed),
    }
}

/// Reads the theme setting, which System leaves to the OS.
fn apply_theme(mut dark: Signal<bool>) {
    let theme = SettingsStore::open(&settings_path()).get().theme;
    dark.set(crate::theme_is_dark(theme));
}

/// Puts the editor on screen. This only happens with a project to show, so the window never appears
/// empty (§48).
fn show_editor() {
    let window = dioxus::desktop::window();
    if !crate::editor_is_showing() {
        crate::native::size_as_editor(&window.window);
    }
    crate::set_editor_showing(true);
    window.set_visible(true);
    crate::native::bring_forward(&window.window);
}

#[component]
pub fn Editor(dark: Signal<bool>) -> Element {
    let session = use_signal(|| None::<Session>);
    let mut ed = use_context_provider(|| Ed {
        session,
        view: Signal::new(View::whole(Micros::from_secs(1))),
        position: Signal::new(Micros::ZERO),
        playing: Signal::new(false),
        notice: Signal::new(None),
        rail: Signal::new(Rail::Scene),
        overlay: Signal::new(OpenOverlay::None),
        keyframes: Signal::new(Vec::new()),
        hole: Signal::new((0.0, 0.0, 1.0, 1.0)),
        dark,
        dragging: Signal::new(false),
    });
    let mut missing = use_signal(|| None::<MissingVideo>);
    let shortcuts = use_signal(|| {
        SettingsStore::open(&settings_path())
            .get()
            .shortcuts
            .clone()
    });
    let mut add_menu = use_signal(|| false);
    let mut volume = use_signal(|| 1.0_f64);
    let mut renaming = use_signal(|| None::<String>);
    use_hook(move || apply_theme(dark));

    let mut finish_open = move |opened: Result<Opened, String>| match opened {
        Ok(Opened::Ready(new_session, prepared)) => {
            show_editor();
            links::tell_bar(ToBar::EditorOpened);
            preview::show(prepared);
            let total = new_session.editor.project().timeline.duration();
            if let Some(path) = new_session.path() {
                remember_project(path);
            }
            let mut slot = ed.session;
            slot.set(Some(*new_session));
            ed.view.set(View::whole(total));
            ed.keyframes.set(Vec::new());
            ed.position.set(Micros::ZERO);
            ed.overlay.set(OpenOverlay::None);
            missing.set(None);
        }
        Ok(Opened::Missing(found)) => {
            show_editor();
            links::tell_bar(ToBar::EditorOpened);
            missing.set(Some(*found));
        }
        Err(error) if crate::editor_is_showing() => {
            ed.notice
                .set(Some(format!("That couldn't be opened: {error}")));
        }
        // With no editor on screen, the bar is what's there to say so.
        Err(error) => links::tell_bar(ToBar::OpenFailed(error)),
    };

    use_wry_event_handler(move |event, _| {
        let own_id = dioxus::desktop::window().id();
        match event {
            Event::WindowEvent {
                event: WindowEvent::Resized(size),
                window_id,
                ..
            } if *window_id == own_id => {
                SHARED.set_surface_size(size.width, size.height);
            }
            // Closing hides the window, so that its surface stays ready. The bar comes back (§48).
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                window_id,
                ..
            } if *window_id == own_id => {
                preview::pause();
                crate::set_editor_showing(false);
                links::tell_bar(ToBar::EditorClosed);
            }
            // A .reel file opened from Finder (§20).
            Event::Opened { urls } => {
                for url in urls {
                    if let Ok(path) = url.to_file_path() {
                        links::tell_editor(ToEditor::Open(path));
                    }
                }
            }
            _ => {}
        }
    });

    use_muda_event_handler(move |event| {
        if !crate::editor_is_showing() {
            return;
        }
        match event.id.as_ref() {
            crate::menu::OPEN => run_action(ed, Action::OpenProjects),
            crate::menu::SAVE => run_action(ed, Action::Save),
            crate::menu::SAVE_AS => run_action(ed, Action::SaveAs),
            crate::menu::UNDO => run_action(ed, Action::Undo),
            crate::menu::REDO => run_action(ed, Action::Redo),
            // In a text box this means its text, and everywhere else it means the zooms.
            crate::menu::SELECT_ALL => {
                spawn(async move {
                    let in_text = document::eval(
                        "const e = document.activeElement; if (e && (e.tagName === 'INPUT' || e.tagName === 'TEXTAREA')) { e.select(); return true; } return false;",
                    )
                    .await;
                    if in_text.ok().and_then(|value| value.as_bool()) != Some(true) {
                        run_action(ed, Action::SelectAllZooms);
                    }
                });
            }
            _ => {}
        }
    });

    // Opens what the launch bar sends over, such as a recording that's just finished.
    use_future(move || async move {
        let mut inbox = links::listen_as_editor();
        while let Some(ToEditor::Open(path)) = inbox.recv().await {
            // What's open is saved first, so opening something else can't lose it (§20).
            let saved = ed
                .session
                .write()
                .as_mut()
                .filter(|session| session.is_dirty() && session.path().is_some())
                .map(Session::save);
            if let Some(Err(error)) = saved {
                show_editor();
                ed.notice
                    .set(Some(format!("The open project couldn't be saved: {error}")));
                continue;
            }
            finish_open(open_file(path, None).await);
        }
    });

    // Opens anything given on the command line.
    use_future(move || async move {
        let launch = launch_options();
        if let Some(file) = launch.file {
            finish_open(open_file(file, launch.look).await);
        }
        #[cfg(feature = "self-test")]
        if std::env::var("REEL_SELF_TEST").as_deref() == Ok("1") {
            crate::self_test::run().await;
        }
        #[cfg(feature = "self-test")]
        if std::env::var("REEL_SELF_TEST").as_deref() == Ok("editor") {
            crate::editor_test::run(ed).await;
        }
    });

    use_future(move || async move {
        let mut reports = document::eval(OBSERVE_HOLE);
        while let Ok(value) = reports.recv::<serde_json::Value>().await {
            SHARED.set_hole(pixel_rect(&value));
            let number = |key: &str| value[key].as_f64().unwrap_or(0.0);
            ed.hole
                .set((number("x"), number("y"), number("w"), number("h")));
        }
    });

    // Keeps the readout in step. A sixtieth of a second is plenty, and T4 showed it costs little.
    use_future(move || async move {
        let mut clock = tokio::time::interval(Duration::from_secs_f64(1.0 / 60.0));
        loop {
            clock.tick().await;
            let now = SHARED.position() - TimelineTime::ZERO;
            if *ed.position.peek() != now {
                ed.position.set(now);
            }
            let is_playing = SHARED.is_playing();
            if *ed.playing.peek() != is_playing {
                ed.playing.set(is_playing);
            }
        }
    });

    // Autosave: a second after a change, and new recordings too, which Recordly never saves (§20).
    use_future(move || async move {
        let mut tick = tokio::time::interval(Duration::from_millis(300));
        loop {
            tick.tick().await;
            let now = Instant::now();
            let due = ed
                .session
                .peek()
                .as_ref()
                .is_some_and(|session| session.autosave_due(now));
            if !due {
                continue;
            }
            let folder = dialogs::projects_folder();
            let name = ed
                .session
                .peek()
                .as_ref()
                .map_or_else(String::new, |session| {
                    session
                        .editor
                        .project()
                        .media
                        .video
                        .absolute
                        .file_stem()
                        .map_or_else(
                            || "Untitled".to_string(),
                            |stem| stem.to_string_lossy().into_owned(),
                        )
                });
            let mut slot = ed.session;
            let result = slot
                .write()
                .as_mut()
                .map(|session| session.autosave(&folder, &name));
            if let Some(Err(error)) = result {
                ed.notice.set(Some(format!("Couldn't save: {error}")));
            }
        }
    });

    // The view resets when the timeline's length changes (§14), but not during a drag: rescaling it
    // under the mouse would move the thing being dragged, which would change the length again.
    let mut seen_total = use_signal(|| Micros::ZERO);
    use_effect(move || {
        let total = ed.total();
        if !(ed.dragging)() && total != *seen_total.peek() {
            seen_total.set(total);
            ed.view.set(View::whole(total));
        }
    });

    // Notices clear themselves.
    use_effect(move || {
        if (ed.notice)().is_some() {
            spawn(async move {
                tokio::time::sleep(Duration::from_secs(4)).await;
                ed.notice.set(None);
            });
        }
    });

    // Dropping a video or project anywhere opens it (§20).
    let on_drop = move |event: DragEvent| {
        event.prevent_default();
        if let Some(file) = event.data_transfer().files().first() {
            links::tell_editor(ToEditor::Open(file.path()));
        }
    };

    if let Some(lost) = missing() {
        return rsx! {
            main { class: "home",
                p { class: "home-message", role: "alert", "This project's video isn't where it was: {lost.path.display()}" }
                if let Some(text) = (ed.notice)() {
                    p { class: "home-message", role: "status", id: "notice", "{text}" }
                }
                Button {
                    id: "relink-video",
                    onclick: move |_| {
                        let picked = rfd::FileDialog::new()
                            .set_title("Find the video")
                            .add_filter("Videos", &["mp4", "mov", "m4v"])
                            .pick_file();
                        let Some(found) = picked else { return };
                        let mut waiting = lost.clone();
                        waiting.session.editor.change(None, |p| p.media.video = MediaRef::absolute(found));
                        spawn(async move {
                            let opened = tokio::task::spawn_blocking(move || prepare_session(waiting.session, waiting.folder)).await;
                            finish_open(opened.unwrap_or_else(|error| Err(error.to_string())));
                        });
                    },
                    "Find the video…"
                }
            }
        };
    }

    let (can_undo, can_redo, dirty, title, path) = {
        let guard = ed.session.read();
        let current = guard.as_ref();
        (
            current.is_some_and(|session| session.editor.can_undo()),
            current.is_some_and(|session| session.editor.can_redo()),
            current.is_some_and(Session::is_dirty),
            dialogs::project_name(current.and_then(Session::path)),
            current.and_then(|session| session.path().map(Path::to_path_buf)),
        )
    };
    let Some(project) = ed.project() else {
        return rsx! {};
    };
    let total = project.timeline.duration();

    let current_overlay = (ed.overlay)();
    let is_cropped = project.look.crop != Crop::default();
    let rail = (ed.rail)();
    let rail_button = move |which: Rail, label: &'static str, icon: Element| {
        rsx! {
            button {
                class: "ed-rail-button",
                id: "rail-{label}",
                title: "{label}",
                "aria-label": "{label}",
                "data-selected": "{rail == which}",
                onclick: move |_| {
                    ed.select(Selection::None);
                    ed.rail.set(which);
                },
                {icon}
            }
        }
    };
    let aspects = [
        "native", "16:9", "9:16", "1:1", "4:3", "4:5", "16:10", "10:16",
    ];

    rsx! {
        div {
            class: "shell",
            id: "editor-root",
            tabindex: "0",
            ondragover: move |event| event.prevent_default(),
            ondrop: on_drop,
            onkeydown: move |event| {
                let key = event.key().to_string();
                if keys::is_modifier(&key) {
                    return;
                }
                let modifiers = event.modifiers();
                let pressed = keys::describe(&key, modifiers.meta(), modifiers.ctrl(), modifiers.alt(), modifiers.shift());
                let bound: BTreeMap<String, String> = shortcuts();
                if let Some(action) = keys::action_for(&pressed, &bound) {
                    event.prevent_default();
                    run_action(ed, action);
                }
            },
            header { class: "shell-header",
                div { class: "shell-actions",
                    button { class: "ed-icon", id: "open-projects", title: "Projects", "aria-label": "Projects", onclick: move |_| ed.overlay.set(OpenOverlay::Projects), FolderOpen {} }
                    button { class: "ed-icon", id: "undo", title: "Undo", "aria-label": "Undo", disabled: !can_undo, onclick: move |_| ed.undo(), Undo2 {} }
                    button { class: "ed-icon", id: "redo", title: "Redo", "aria-label": "Redo", disabled: !can_redo, onclick: move |_| ed.redo(), Redo2 {} }
                }
                div { class: "shell-title",
                    if let Some(text) = renaming() {
                        input {
                            id: "rename",
                            value: "{text}",
                            autofocus: true,
                            onmounted: move |event| async move { let _ = event.data().set_focus(true).await; },
                            oninput: move |event| renaming.set(Some(event.value())),
                            onkeydown: move |event| {
                                event.stop_propagation();
                                if event.key() == Key::Enter {
                                    if let Some(name) = renaming() {
                                        let mut slot = ed.session;
                                        let result = slot.write().as_mut().map(|session| session.rename(name.trim()));
                                        if let Some(Err(error)) = result {
                                            ed.notice.set(Some(error.to_string()));
                                        }
                                    }
                                    renaming.set(None);
                                } else if event.key() == Key::Escape {
                                    renaming.set(None);
                                }
                            },
                            onblur: move |_| renaming.set(None),
                        }
                    } else {
                        button {
                            class: "shell-name",
                            id: "project-title",
                            title: "Rename",
                            onclick: {
                                let title = title.clone();
                                move |_| renaming.set(Some(title.clone()))
                            },
                            "{title}"
                            if dirty { span { class: "shell-dirty", "aria-label": "Unsaved changes" } }
                        }
                    }
                }
                div { class: "shell-actions",
                    if let Some(text) = (ed.notice)() {
                        span { class: "shell-notice", role: "status", id: "notice", "{text}" }
                    }
                    button { class: "row-button", id: "open-presets", onclick: move |_| ed.overlay.set(OpenOverlay::Presets), "Presets" }
                    button { class: "row-button row-primary", id: "open-export", onclick: move |_| ed.overlay.set(OpenOverlay::Export), Download {} "Export" }
                }
            }
            div { class: "ed-main",
                nav { class: "ed-rail", "aria-label": "Panels",
                    {rail_button(Rail::Scene, "Scene", rsx! { Image {} })}
                    {rail_button(Rail::Cursor, "Cursor", rsx! { MousePointer2 {} })}
                    {rail_button(Rail::Webcam, "Webcam", rsx! { Video {} })}
                    {rail_button(Rail::Captions, "Captions", rsx! { Captions {} })}
                    {rail_button(Rail::Settings, "Settings", rsx! { SettingsIcon {} })}
                }
                panels::SidePanel {}
                div { class: "ed-stage",
                    div { class: "ed-toolbar",
                        select {
                            id: "aspect-ratio",
                            "aria-label": "Aspect ratio",
                            onchange: move |event| {
                                let value = event.value();
                                ed.change("aspect-ratio", |p| p.look.aspect_ratio = value);
                                ed.end_gesture();
                            },
                            for aspect in aspects {
                                option { value: "{aspect}", selected: project.look.aspect_ratio == aspect, if aspect == "native" { "Native" } else { "{aspect}" } }
                            }
                        }
                        button {
                            class: "row-button",
                            id: "open-crop",
                            "data-active": "{is_cropped}",
                            onclick: move |_| ed.overlay.set(OpenOverlay::Crop),
                            "Crop"
                            if is_cropped { span { class: "shell-dirty" } }
                        }
                    }
                    div { id: "preview-hole", class: "shell-preview",
                        overlay::PreviewOverlay {}
                    }
                    footer { class: "shell-transport",
                        div { class: "ed-add",
                            button {
                                class: "row-button",
                                id: "add-layer",
                                "aria-expanded": "{add_menu()}",
                                onclick: move |_| add_menu.toggle(),
                                Plus {} "Add"
                            }
                            if add_menu() {
                                div { class: "ed-add-menu", role: "menu",
                                    button { id: "add-text", role: "menuitem", onclick: move |_| { add_menu.set(false); run_action(ed, Action::AddAnnotation); }, "Text" }
                                    button { id: "add-arrow", role: "menuitem", onclick: move |_| {
                                        add_menu.set(false);
                                        add_annotation(ed, AnnotationContent::Arrow { direction: ArrowDirection::Right, color: "#2563EB".to_string(), stroke_width: 4.0 });
                                    }, "Arrow" }
                                    button { id: "add-blur", role: "menuitem", onclick: move |_| {
                                        add_menu.set(false);
                                        add_annotation(ed, AnnotationContent::Blur { strength: 20.0, fill: None });
                                    }, "Blur" }
                                    button { id: "add-image", role: "menuitem", onclick: move |_| {
                                        add_menu.set(false);
                                        add_image(ed);
                                    }, "Image…" }
                                    button { id: "add-audio", role: "menuitem", onclick: move |_| { add_menu.set(false); add_audio(ed); }, "Audio…" }
                                }
                            }
                        }
                        button { class: "ed-icon", id: "add-zoom", title: "Add zoom", "aria-label": "Add zoom", onclick: move |_| add_zoom(ed), ZoomIn {} }
                        button { class: "ed-icon", id: "suggest-zooms", title: "Suggest zooms", "aria-label": "Suggest zooms", onclick: move |_| suggest(ed), Wand {} }
                        button { class: "ed-icon", id: "split", title: "Split", "aria-label": "Split", onclick: move |_| { ed.apply(&Edit::SplitClip { at: ed.now() }); }, Scissors {} }
                        div { class: "ed-transport-middle",
                            span { id: "position", class: "shell-time", "{clock_text((ed.position)())}" }
                            button {
                                class: "ed-icon",
                                id: "skip-back",
                                "aria-label": "Skip back",
                                onclick: move |_| ed.seek(skip_target(ed.now(), &(ed.keyframes)(), false, total)),
                                SkipBack {}
                            }
                            button {
                                class: "ed-play",
                                id: "play",
                                "aria-label": if (ed.playing)() { "Pause" } else { "Play" },
                                onclick: move |_| run_action(ed, Action::PlayPause),
                                if (ed.playing)() { Pause {} } else { Play {} }
                            }
                            button {
                                class: "ed-icon",
                                id: "skip-forward",
                                "aria-label": "Skip forward",
                                onclick: move |_| ed.seek(skip_target(ed.now(), &(ed.keyframes)(), true, total)),
                                SkipForward {}
                            }
                            span { class: "shell-time", "{clock_text(total)}" }
                        }
                        button {
                            class: "ed-icon",
                            id: "mute",
                            "aria-label": if volume() > 0.0 { "Mute" } else { "Unmute" },
                            onclick: move |_| {
                                let next = if volume() > 0.0 { 0.0 } else { 1.0 };
                                volume.set(next);
                                preview::set_volume(next);
                            },
                            if volume() > 0.0 { Volume2 {} } else { VolumeX {} }
                        }
                        label { class: "shell-volume",
                            span { class: "sr-only", "Volume" }
                            Slider {
                                min: 0.0,
                                max: 1.0,
                                step: 0.01,
                                value: volume(),
                                oninput: move |event: FormEvent| {
                                    if let Ok(level) = event.value().parse::<f64>() {
                                        volume.set(level);
                                        preview::set_volume(level);
                                    }
                                },
                            }
                        }
                    }
                }
            }
            timeline::TimelinePanel {}
            match current_overlay {
                OpenOverlay::Export => rsx! { export_panel::ExportPanel {} },
                OpenOverlay::Projects => rsx! { dialogs::ProjectBrowser { current: path } },
                OpenOverlay::Presets => rsx! { dialogs::PresetsMenu {} },
                OpenOverlay::Shortcuts => rsx! { dialogs::ShortcutsDialog { bound: shortcuts } },
                _ => rsx! {},
            }
        }
    }
}

/// Picks an image and embeds it in the project (§13).
fn add_image(mut ed: Ed) {
    let picked = rfd::FileDialog::new()
        .set_title("Add an image")
        .add_filter("Images", &["jpg", "jpeg", "png", "gif", "webp"])
        .pick_file();
    let Some(path) = picked else { return };
    let Ok(bytes) = std::fs::read(&path) else {
        ed.notice
            .set(Some("That image couldn't be read.".to_string()));
        return;
    };
    let kind = match path
        .extension()
        .map(|ext| ext.to_string_lossy().to_ascii_lowercase())
        .as_deref()
    {
        Some("png") => "png",
        Some("gif") => "gif",
        Some("webp") => "webp",
        _ => "jpeg",
    };
    let data_url = format!("data:image/{kind};base64,{}", crate::base64(&bytes));
    add_annotation(ed, AnnotationContent::Image { data_url });
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let (session, _) = load_session(&reel, Some(&look)).unwrap();
        assert_eq!(session.editor.project().look.shadow, 0.1);
        assert_eq!(session.editor.project().look.corner_radius, 30.0);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
