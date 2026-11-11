//! Generating captions, and the edits on them that aren't single timeline operations.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU64, Ordering};
use std::time::Duration;

use dioxus::prelude::*;
use reel_captions::editing::{captions_from_cues, merge_caption as merged, split_caption as split};
use reel_captions::models::{self, SMALL};
use reel_captions::transcribe::transcribe;
use reel_core::edit::Edit;
use reel_core::project::SourceTrackKind;
use reel_core::settings::SettingsStore;
use reel_core::timeline::ItemId;
use reel_media::macos::read_pcm;

use super::controls::{ChoiceRow, Section};
use super::state::{Ed, use_ed};
use crate::recording::support_folder;

const LANGUAGES: [(&str, &str); 10] = [
    ("auto", "Detect"),
    ("en", "English"),
    ("es", "Spanish"),
    ("fr", "French"),
    ("de", "German"),
    ("it", "Italian"),
    ("pt", "Portuguese"),
    ("zh", "Chinese"),
    ("ja", "Japanese"),
    ("ko", "Korean"),
];

fn settings_path() -> PathBuf {
    support_folder().join("settings.json")
}

fn default_model_path() -> PathBuf {
    SMALL.path_in(&support_folder().join("models"))
}

fn model_path() -> Option<PathBuf> {
    SettingsStore::open(&settings_path())
        .get()
        .whisper_model
        .clone()
        .filter(|path| path.exists())
        .or_else(|| Some(default_model_path()).filter(|path| path.exists()))
}

/// A sound track that could be transcribed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AudioChoice {
    pub key: String,
    pub label: String,
    pub path: PathBuf,
}

/// What can be transcribed. The mic goes first, since that's almost always the speech. Looking only
/// at the recording often finds the wrong track, or nothing (BUILD_PLAN 9.2).
#[must_use]
pub fn audio_choices(ed: &Ed) -> Vec<AudioChoice> {
    let Some(project) = ed.project() else {
        return Vec::new();
    };
    let folder = ed.folder();
    let mut choices = Vec::new();
    for track in &project.media.source_tracks {
        let Some(file) = &track.file else { continue };
        let label = match track.kind {
            SourceTrackKind::Mic => "Microphone",
            SourceTrackKind::System => "System audio",
            SourceTrackKind::Mixed => "Recording",
        };
        choices.push((
            matches!(track.kind, SourceTrackKind::Mic),
            AudioChoice {
                key: label.to_lowercase(),
                label: label.to_string(),
                path: file.resolve(folder.as_deref()),
            },
        ));
    }
    choices.sort_by_key(|(is_mic, _)| !is_mic);
    let mut result: Vec<AudioChoice> = choices.into_iter().map(|(_, choice)| choice).collect();
    let video = project.media.video.resolve(folder.as_deref());
    result.push(AudioChoice {
        key: "video".to_string(),
        label: "Video's own sound".to_string(),
        path: video,
    });
    result
}

#[derive(Clone, Debug, PartialEq)]
enum Work {
    Idle,
    Downloading,
    Transcribing,
}

#[component]
pub fn GeneratePanel(language: String, has_captions: bool) -> Element {
    let ed = use_ed();
    let mut work = use_signal(|| Work::Idle);
    let mut percent = use_signal(|| 0_i32);
    let mut message = use_signal(|| None::<String>);
    let mut model = use_signal(model_path);
    let choices = audio_choices(&ed);
    let mut chosen_audio = use_signal(|| None::<String>);
    let cancel = use_hook(|| Arc::new(AtomicBool::new(false)));

    let selected_audio = chosen_audio()
        .filter(|key| choices.iter().any(|choice| choice.key == *key))
        .or_else(|| choices.first().map(|choice| choice.key.clone()))
        .unwrap_or_default();

    let cancel_download = cancel.clone();
    let start_download = move |_| {
        message.set(None);
        work.set(Work::Downloading);
        percent.set(0);
        cancel_download.store(false, Ordering::Relaxed);
        let flag = cancel_download.clone();
        let progress = Arc::new(AtomicI32::new(0));
        let shown = progress.clone();
        spawn(async move {
            let finished = Arc::new(AtomicU64::new(0));
            let done = finished.clone();
            let task = tokio::task::spawn_blocking(move || {
                let result = models::download(&SMALL, &default_model_path(), |got, total| {
                    if let Some(total) = total {
                        progress.store((got * 100 / total.max(1)) as i32, Ordering::Relaxed);
                    }
                    !flag.load(Ordering::Relaxed)
                });
                done.store(1, Ordering::Relaxed);
                result
            });
            while finished.load(Ordering::Relaxed) == 0 {
                tokio::time::sleep(Duration::from_millis(100)).await;
                percent.set(shown.load(Ordering::Relaxed));
            }
            match task.await {
                Ok(Ok(())) => model.set(model_path()),
                Ok(Err(error)) => message.set(Some(error.to_string())),
                Err(error) => message.set(Some(error.to_string())),
            }
            work.set(Work::Idle);
        });
    };
    let cancel_flag = cancel.clone();

    let audio_for_generate = choices.clone();
    let audio_key = selected_audio.clone();
    let generate = move |_| {
        let Some(path) = model() else { return };
        let Some(project) = ed.project() else { return };
        let Some(source) = audio_for_generate
            .iter()
            .find(|choice| choice.key == audio_key)
            .cloned()
        else {
            return;
        };
        let language = project.caption_style.language.clone();
        let timeline = project.timeline.clone();
        message.set(None);
        work.set(Work::Transcribing);
        percent.set(0);
        spawn(async move {
            let progress = Arc::new(AtomicI32::new(0));
            let (shown, reported) = (progress.clone(), progress);
            let finished = Arc::new(AtomicU64::new(0));
            let done = finished.clone();
            let task = tokio::task::spawn_blocking(move || {
                let result = (|| {
                    let pcm = read_pcm(&source.path)
                        .map_err(|error| error.0)?
                        .ok_or_else(|| format!("{} has no sound to transcribe.", source.label))?;
                    let cues = transcribe(&path, &pcm.samples, &language, move |value| {
                        reported.store(value, Ordering::Relaxed);
                    })?;
                    Ok::<_, String>(captions_from_cues(&timeline, &cues))
                })();
                done.store(1, Ordering::Relaxed);
                result
            });
            while finished.load(Ordering::Relaxed) == 0 {
                tokio::time::sleep(Duration::from_millis(100)).await;
                percent.set(shown.load(Ordering::Relaxed));
            }
            match task.await {
                Ok(Ok(captions)) if captions.is_empty() => {
                    message.set(Some("No speech was heard in that track.".to_string()));
                }
                Ok(Ok(captions)) => {
                    ed.apply(&Edit::SetCaptions { captions });
                    ed.change("captions-show", |p| p.caption_style.show = true);
                    ed.end_gesture();
                }
                Ok(Err(error)) => message.set(Some(error)),
                Err(error) => message.set(Some(error.to_string())),
            }
            work.set(Work::Idle);
        });
    };

    let language_options: Vec<(String, String)> = LANGUAGES
        .iter()
        .map(|(key, name)| ((*key).to_string(), (*name).to_string()))
        .collect();
    let busy = work() != Work::Idle;
    rsx! {
        Section { title: "Generate",
            if let Some(path) = model() {
                p { class: "panel-note", title: "{path.display()}", "Runs on this Mac. Nothing is uploaded." }
            } else {
                p { class: "panel-note", "Captions need a speech model ({SMALL.megabytes} MB). It stays on this Mac, and nothing you record is uploaded." }
            }
            div { class: "row-buttons",
                if model().is_none() {
                    button {
                        class: "row-button",
                        id: "download-model",
                        disabled: busy,
                        onclick: start_download,
                        "Download model"
                    }
                } else {
                    button {
                        class: "row-button",
                        id: "delete-model",
                        disabled: busy,
                        onclick: move |_| {
                            let _ = std::fs::remove_file(default_model_path());
                            let mut store = SettingsStore::open(&settings_path());
                            let _ = store.update(|settings| settings.whisper_model = None);
                            model.set(model_path());
                        },
                        "Delete model"
                    }
                }
                button {
                    class: "row-button",
                    id: "select-model",
                    disabled: busy,
                    onclick: move |_| {
                        let picked = rfd::FileDialog::new()
                            .set_title("Choose a Whisper model")
                            .add_filter("Whisper models", &["bin"])
                            .pick_file();
                        if let Some(path) = picked {
                            let mut store = SettingsStore::open(&settings_path());
                            let chosen = path.clone();
                            let _ = store.update(|settings| settings.whisper_model = Some(chosen));
                            model.set(Some(path));
                        }
                    },
                    "Use my own…"
                }
            }
            ChoiceRow {
                label: "Sound to use",
                options: choices.iter().map(|choice| (choice.key.clone(), choice.label.clone())).collect::<Vec<_>>(),
                value: selected_audio.clone(),
                onchange: move |key| chosen_audio.set(Some(key)),
            }
            div { class: "row-choice",
                span { class: "row-label", "Language" }
                select {
                    "aria-label": "Language",
                    onchange: move |event| {
                        let value = event.value();
                        ed.change("captions-language", |p| p.caption_style.language = value);
                        ed.end_gesture();
                    },
                    for (key, name) in language_options {
                        option { value: "{key}", selected: key == language, "{name}" }
                    }
                }
            }
            if busy {
                p { class: "panel-note", role: "status",
                    if work() == Work::Downloading { "Downloading… {percent()}%" } else { "Transcribing… {percent()}%" }
                }
                if work() == Work::Downloading {
                    div { class: "row-buttons",
                        button {
                            class: "row-button",
                            onclick: move |_| cancel_flag.store(true, Ordering::Relaxed),
                            "Cancel"
                        }
                    }
                }
            }
            if let Some(text) = message() {
                p { class: "panel-error", role: "alert", "{text}" }
            }
            div { class: "row-buttons",
                button {
                    class: "row-button row-primary",
                    id: "generate-captions",
                    disabled: busy || model().is_none(),
                    onclick: generate,
                    if has_captions { "Regenerate" } else { "Generate" }
                }
                if has_captions {
                    button {
                        class: "row-button",
                        id: "clear-captions",
                        disabled: busy,
                        onclick: move |_| { ed.apply(&Edit::SetCaptions { captions: Vec::new() }); },
                        "Clear"
                    }
                }
            }
        }
    }
}

/// Splits a caption at the playhead, at the nearest word.
pub fn split_caption(mut ed: Ed, id: &ItemId) {
    let Some(project) = ed.project() else { return };
    match split(&project.timeline.captions, id, ed.now()) {
        Some(captions) => {
            ed.apply(&Edit::SetCaptions { captions });
        }
        None => ed.notice.set(Some(
            "Move the playhead into the caption, away from its ends, to split it.".to_string(),
        )),
    }
}

/// Joins a caption with the one after it.
pub fn merge_caption(ed: Ed, id: &ItemId) {
    let Some(project) = ed.project() else { return };
    if let Some(captions) = merged(&project.timeline.captions, id) {
        ed.apply(&Edit::SetCaptions { captions });
    }
}
