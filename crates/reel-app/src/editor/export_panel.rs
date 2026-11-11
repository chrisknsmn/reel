//! The export panel. It renders first and asks where to save afterwards, so that cancelling the
//! save dialog doesn't throw the render away: Save Again works without rendering again.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use dioxus::prelude::*;
use reel_captions::subtitles::{to_srt, to_vtt};
use reel_capture::disk;
use reel_core::project::{EncodingMode, ExportFormat, ExportQuality, GifSize, Project};
use reel_render::export::{Scene, export_gif, export_mp4, gif_size};

use super::controls::{ChoiceRow, ToggleRow};
use super::state::{Overlay, use_ed};
use crate::recording::support_folder;

/// Every effect the project uses, and whether this export will include it, so nothing is left out
/// without a word (BUILD_PLAN 8.7).
#[must_use]
pub fn left_out(project: &Project, format: ExportFormat) -> Vec<String> {
    let mut notes = Vec::new();
    if format == ExportFormat::Gif
        && (!project.timeline.audio.is_empty() || !project.media.source_tracks.is_empty())
    {
        notes.push("GIFs have no sound.".to_string());
    }
    notes
}

/// A rough size for an MP4, for the disk check.
#[must_use]
pub fn estimated_bytes(bitrate: u64, seconds: f64) -> u64 {
    // Sound and overhead on top of the video.
    ((bitrate as f64 + 200_000.0) / 8.0 * seconds * 1.1) as u64
}

/// How much free space an export of this size needs: its own size twice, once for the temporary file
/// and once for the move if that crosses disks.
#[must_use]
pub fn has_room(free: Option<u64>, needed: u64) -> bool {
    free.is_none_or(|free| free >= needed.saturating_mul(2))
}

fn default_name(format: ExportFormat) -> String {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_secs());
    let extension = if format == ExportFormat::Gif {
        "gif"
    } else {
        "mp4"
    };
    format!("export-{stamp}.{extension}")
}

#[derive(Clone, Debug, PartialEq)]
enum Stage {
    Settings,
    Rendering,
    /// Rendered. The file waits here until it's been saved somewhere.
    Rendered {
        file: PathBuf,
        seconds: f64,
        saved: Option<PathBuf>,
    },
    Failed(String),
}

#[component]
pub fn ExportPanel() -> Element {
    let mut ed = use_ed();
    let mut stage = use_signal(|| Stage::Settings);
    let mut done_frames = use_signal(|| 0_u64);
    let mut total_frames = use_signal(|| 0_u64);
    let mut speed = use_signal(|| 0.0_f64);
    let mut captions_file = use_signal(|| false);
    let cancel = use_hook(|| Arc::new(AtomicBool::new(false)));
    let Some(project) = ed.project() else {
        return rsx! {};
    };
    let export = project.export.clone();
    let format = export.format;
    let stage_size = crate::preview::stage_and_frame().map_or((0, 0), |(size, _)| size);
    let has_captions = project.caption_style.show && !project.timeline.captions.is_empty();

    let final_size = |quality: ExportQuality| {
        let (width, height) = quality.scaled_size(stage_size);
        format!("{width}×{height}")
    };
    let quality = match export.quality {
        ExportQuality::Low => "low",
        ExportQuality::Medium => "medium",
        ExportQuality::Good => "good",
        ExportQuality::Source => "source",
    };
    let encoding = match export.encoding {
        EncodingMode::Fast => "fast",
        EncodingMode::Balanced => "balanced",
        EncodingMode::Quality => "quality",
    };
    let gif_choice = match export.gif_size {
        GifSize::Small => "small",
        GifSize::Medium => "medium",
        GifSize::Large => "large",
        GifSize::Original => "original",
    };
    let notes = left_out(&project, format);

    let cancel_start = cancel.clone();
    let project_for_start = project.clone();
    let start = move |_| {
        let project = project_for_start.clone();
        let folder = ed.folder();
        let duration = project.timeline.duration().secs();
        let needed = estimated_bytes(40_000_000, duration);
        let temp_folder = support_folder().join("exports");
        if let Err(error) = std::fs::create_dir_all(&temp_folder) {
            stage.set(Stage::Failed(format!(
                "{} can't be made: {error}",
                temp_folder.display()
            )));
            return;
        }
        if !has_room(disk::free_bytes(&temp_folder), needed.min(4_000_000_000)) {
            stage.set(Stage::Failed(
                "There isn't enough disk space for this export.".to_string(),
            ));
            return;
        }
        let flag = cancel_start.clone();
        flag.store(false, Ordering::Relaxed);
        stage.set(Stage::Rendering);
        done_frames.set(0);
        total_frames.set(0);
        speed.set(0.0);
        spawn(async move {
            let (done, total) = (Arc::new(AtomicU64::new(0)), Arc::new(AtomicU64::new(0)));
            let (done_in, total_in, flag_in) = (done.clone(), total.clone(), flag.clone());
            let name = default_name(project.export.format);
            let file = temp_folder.join(name);
            let target = file.clone();
            let finished = Arc::new(AtomicBool::new(false));
            let finished_in = finished.clone();
            let task = tokio::task::spawn_blocking(move || {
                let result = (|| {
                    let scene = Scene::new(project, folder.as_deref()).map_err(|error| error.0)?;
                    let progress = |frame: u64, frames: u64| {
                        done_in.store(frame, Ordering::Relaxed);
                        total_in.store(frames, Ordering::Relaxed);
                        !flag_in.load(Ordering::Relaxed)
                    };
                    let report = if scene.project.export.format == ExportFormat::Gif {
                        export_gif(&scene, &target, progress)
                    } else {
                        export_mp4(&scene, &target, progress)
                    };
                    report.map_err(|error| error.0)
                })();
                finished_in.store(true, Ordering::Release);
                result
            });
            let began = Instant::now();
            while !finished.load(Ordering::Acquire) {
                tokio::time::sleep(Duration::from_millis(100)).await;
                let frames = done.load(Ordering::Relaxed);
                done_frames.set(frames);
                total_frames.set(total.load(Ordering::Relaxed));
                speed.set(frames as f64 / began.elapsed().as_secs_f64().max(0.001));
            }
            match task.await {
                Ok(Ok(report)) => {
                    crate::logs::write("export", &format!("{report:?}"));
                    stage.set(Stage::Rendered {
                        file,
                        seconds: report.elapsed_seconds,
                        saved: None,
                    });
                }
                // A cancel isn't an error, so it goes back to the settings quietly.
                Ok(Err(_)) if flag.load(Ordering::Relaxed) => stage.set(Stage::Settings),
                Ok(Err(error)) => {
                    crate::logs::write("export", &error);
                    stage.set(Stage::Failed(error));
                }
                Err(error) => stage.set(Stage::Failed(error.to_string())),
            }
        });
    };

    let cancel_render = cancel.clone();
    let project_for_save = project.clone();
    let write_captions = captions_file();
    let mut save = move |file: PathBuf, seconds: f64| {
        let extension = if format == ExportFormat::Gif {
            "gif"
        } else {
            "mp4"
        };
        let name = file.file_name().map_or_else(
            || format!("export.{extension}"),
            |name| name.to_string_lossy().into_owned(),
        );
        let downloads = std::env::var_os("HOME").map(|home| Path::new(&home).join("Downloads"));
        let mut dialog = rfd::FileDialog::new()
            .set_title("Save the export")
            .set_file_name(name)
            .add_filter(extension, &[extension]);
        if let Some(folder) = downloads {
            dialog = dialog.set_directory(folder);
        }
        let Some(destination) = dialog.save_file() else {
            return;
        };
        // Copied rather than moved, so that Save Again still works for another copy.
        if let Err(error) = std::fs::copy(&file, &destination) {
            stage.set(Stage::Failed(format!(
                "{} can't be written: {error}",
                destination.display()
            )));
            return;
        }
        if write_captions && format == ExportFormat::Mp4 {
            let timeline = &project_for_save.timeline;
            let written = std::fs::write(destination.with_extension("srt"), to_srt(timeline))
                .and_then(|()| std::fs::write(destination.with_extension("vtt"), to_vtt(timeline)));
            if let Err(error) = written {
                // Say so, rather than report the export as complete.
                ed.notice.set(Some(format!(
                    "The caption files couldn't be written: {error}"
                )));
            }
        }
        stage.set(Stage::Rendered {
            file,
            seconds,
            saved: Some(destination),
        });
    };

    let close = move |_| {
        if let Stage::Rendered { file, .. } = stage() {
            let _ = std::fs::remove_file(file);
        }
        ed.overlay.set(Overlay::None);
    };

    rsx! {
        div { class: "ex-panel", id: "export-panel", role: "dialog", "aria-label": "Export",
            header {
                h2 { "Export" }
                button { class: "row-button", id: "export-close", onclick: close, "Close" }
            }
            match stage() {
                Stage::Settings => rsx! {
                    ChoiceRow {
                        label: "Format",
                        options: vec![("mp4".to_string(), "MP4".to_string()), ("gif".to_string(), "GIF".to_string())],
                        value: if format == ExportFormat::Gif { "gif".to_string() } else { "mp4".to_string() },
                        onchange: move |choice: String| {
                            ed.change("export-format", |p| p.export.format = if choice == "gif" { ExportFormat::Gif } else { ExportFormat::Mp4 });
                            ed.end_gesture();
                        },
                    }
                    if format == ExportFormat::Mp4 {
                        ChoiceRow {
                            label: "Quality",
                            options: vec![
                                ("low".to_string(), format!("Low {}", final_size(ExportQuality::Low))),
                                ("medium".to_string(), format!("Medium {}", final_size(ExportQuality::Medium))),
                                ("good".to_string(), format!("High {}", final_size(ExportQuality::Good))),
                                ("source".to_string(), format!("Original {}", final_size(ExportQuality::Source))),
                            ],
                            value: quality.to_string(),
                            onchange: move |choice: String| {
                                ed.change("export-quality", |p| p.export.quality = match choice.as_str() {
                                    "low" => ExportQuality::Low,
                                    "medium" => ExportQuality::Medium,
                                    "good" => ExportQuality::Good,
                                    _ => ExportQuality::Source,
                                });
                                ed.end_gesture();
                            },
                        }
                        ChoiceRow {
                            label: "Encoding",
                            options: vec![("fast".to_string(), "Fast".to_string()), ("balanced".to_string(), "Balanced".to_string()), ("quality".to_string(), "Quality".to_string())],
                            value: encoding.to_string(),
                            onchange: move |choice: String| {
                                ed.change("export-encoding", |p| p.export.encoding = match choice.as_str() {
                                    "fast" => EncodingMode::Fast,
                                    "quality" => EncodingMode::Quality,
                                    _ => EncodingMode::Balanced,
                                });
                                ed.end_gesture();
                            },
                        }
                        ChoiceRow {
                            label: "Frame rate",
                            options: vec![("24".to_string(), "24".to_string()), ("30".to_string(), "30".to_string()), ("60".to_string(), "60".to_string())],
                            value: export.mp4_frame_rate.to_string(),
                            onchange: move |choice: String| {
                                if let Ok(rate) = choice.parse::<u32>() {
                                    ed.change("export-rate", |p| p.export.mp4_frame_rate = rate);
                                    ed.end_gesture();
                                }
                            },
                        }
                        if has_captions {
                            ToggleRow {
                                label: "Also save .srt and .vtt files",
                                value: captions_file(),
                                onchange: move |on| captions_file.set(on),
                            }
                        }
                    } else {
                        ChoiceRow {
                            label: "Frame rate",
                            options: vec![("15".to_string(), "15".to_string()), ("20".to_string(), "20".to_string()), ("25".to_string(), "25".to_string()), ("30".to_string(), "30".to_string())],
                            value: export.gif_frame_rate.to_string(),
                            onchange: move |choice: String| {
                                if let Ok(rate) = choice.parse::<u32>() {
                                    ed.change("export-gif-rate", |p| p.export.gif_frame_rate = rate);
                                    ed.end_gesture();
                                }
                            },
                        }
                        ChoiceRow {
                            label: "Size",
                            options: [("medium", GifSize::Medium, "Medium"), ("large", GifSize::Large, "Large"), ("original", GifSize::Original, "Original")]
                                .iter()
                                .map(|(key, size, name)| {
                                    let (width, height) = gif_size(stage_size, *size);
                                    ((*key).to_string(), format!("{name} {width}×{height}"))
                                })
                                .collect::<Vec<_>>(),
                            value: gif_choice.to_string(),
                            onchange: move |choice: String| {
                                ed.change("export-gif-size", |p| p.export.gif_size = match choice.as_str() {
                                    "large" => GifSize::Large,
                                    "original" => GifSize::Original,
                                    _ => GifSize::Medium,
                                });
                                ed.end_gesture();
                            },
                        }
                        ToggleRow {
                            label: "Loop",
                            value: export.gif_loop,
                            onchange: move |on| {
                                ed.change("export-gif-loop", |p| p.export.gif_loop = on);
                                ed.end_gesture();
                            },
                        }
                    }
                    for note in notes.clone() {
                        p { class: "panel-note", "{note}" }
                    }
                    div { class: "row-buttons",
                        button { class: "row-button row-primary", id: "export-start", onclick: start.clone(), "Export" }
                    }
                },
                Stage::Rendering => rsx! {
                    p { class: "panel-note", role: "status", id: "export-status",
                        "Rendering frame {done_frames()} of {total_frames()}, at {speed():.0} frames a second"
                    }
                    progress {
                        class: "ex-progress",
                        id: "export-progress",
                        max: "{total_frames().max(1)}",
                        value: "{done_frames()}",
                    }
                    div { class: "row-buttons",
                        button {
                            class: "row-button",
                            id: "export-cancel",
                            onclick: {
                                let flag = cancel_render.clone();
                                move |_| flag.store(true, Ordering::Relaxed)
                            },
                            "Cancel"
                        }
                    }
                },
                Stage::Rendered { file, seconds, saved } => {
                    let (file_save, file_show) = (file.clone(), saved.clone());
                    rsx! {
                        p { class: "panel-note", id: "export-done", "Rendered in {seconds:.1} s." }
                        if let Some(path) = saved.clone() {
                            p { class: "panel-note", title: "{path.display()}", "Saved." }
                        }
                        div { class: "row-buttons",
                            button {
                                class: "row-button row-primary",
                                id: "export-save",
                                onclick: move |_| save(file_save.clone(), seconds),
                                if saved.is_some() { "Save another copy…" } else { "Save…" }
                            }
                            if let Some(path) = file_show {
                                button {
                                    class: "row-button",
                                    id: "export-reveal",
                                    onclick: move |_| {
                                        let _ = std::process::Command::new("open").arg("-R").arg(&path).status();
                                    },
                                    "Show in Finder"
                                }
                            }
                        }
                    }
                },
                Stage::Failed(reason) => rsx! {
                    p { class: "panel-error", role: "alert", id: "export-error", "{reason}" }
                    div { class: "row-buttons",
                        button {
                            class: "row-button",
                            id: "export-back",
                            onclick: move |_| stage.set(Stage::Settings),
                            "Back"
                        }
                    }
                },
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_gif_says_it_has_no_sound_only_if_theres_sound_to_lose() {
        let mut project = Project::default();
        assert!(left_out(&project, ExportFormat::Gif).is_empty());
        project
            .media
            .source_tracks
            .push(reel_core::project::SourceTrack {
                kind: reel_core::project::SourceTrackKind::Mic,
                file: None,
                volume: 1.0,
                boost: false,
            });
        assert_eq!(left_out(&project, ExportFormat::Gif).len(), 1);
        assert!(left_out(&project, ExportFormat::Mp4).is_empty());
    }

    #[test]
    fn size_estimates_are_in_the_right_range() {
        let bytes = estimated_bytes(10_000_000, 60.0);
        assert!((70_000_000..100_000_000).contains(&bytes), "{bytes}");
    }

    #[test]
    fn room_is_checked_for_twice_the_size() {
        assert!(has_room(Some(200), 100));
        assert!(!has_room(Some(199), 100));
        assert!(
            has_room(None, 100),
            "if it can't be found out, don't stand in the way"
        );
    }
}
