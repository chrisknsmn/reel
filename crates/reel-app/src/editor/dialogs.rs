//! The project browser, presets, and the shortcut editor.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use dioxus::prelude::*;
use reel_core::format;
use reel_core::settings::{Preset, SettingsStore};

use super::keys::{self, REBINDABLE, Rebind};
use super::state::{Overlay, use_ed};
use crate::links::{self, ToEditor};
use crate::recording::support_folder;

/// The most projects the browser lists.
const NEWEST: usize = 24;

fn settings_path() -> PathBuf {
    support_folder().join("settings.json")
}

/// Where new projects go.
#[must_use]
pub fn projects_folder() -> PathBuf {
    SettingsStore::open(&settings_path())
        .get()
        .recordings_dir
        .clone()
        .unwrap_or_else(|| support_folder().join("recordings"))
}

/// A project that can be opened.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectEntry {
    pub path: PathBuf,
    pub name: String,
    pub modified: Option<SystemTime>,
}

fn entry_for(path: PathBuf) -> Option<ProjectEntry> {
    let name = path.file_stem()?.to_string_lossy().into_owned();
    let modified = std::fs::metadata(&path)
        .and_then(|info| info.modified())
        .ok();
    Some(ProjectEntry {
        path,
        name,
        modified,
    })
}

/// The newest projects in these folders and the recent list, without repeats.
#[must_use]
pub fn list_projects(folders: &[PathBuf], recent: &[PathBuf]) -> Vec<ProjectEntry> {
    let mut paths: Vec<PathBuf> = recent
        .iter()
        .filter(|path| path.exists())
        .cloned()
        .collect();
    for folder in folders {
        if let Ok(entries) = std::fs::read_dir(folder) {
            paths.extend(entries.flatten().map(|entry| entry.path()).filter(|path| {
                path.extension()
                    .is_some_and(|extension| extension == format::EXTENSION)
            }));
        }
    }
    paths.sort();
    paths.dedup();
    let mut entries: Vec<ProjectEntry> = paths.into_iter().filter_map(entry_for).collect();
    entries.sort_by(|a, b| b.modified.cmp(&a.modified));
    entries.truncate(NEWEST);
    entries
}

#[component]
pub fn ProjectBrowser(current: Option<PathBuf>) -> Element {
    let ed = use_ed();
    let mut overlay = ed.overlay;
    let recent = SettingsStore::open(&settings_path())
        .get()
        .recent_projects
        .clone();
    let folder = projects_folder();
    let entries = list_projects(&[folder.clone(), folder.join("Projects")], &recent);
    rsx! {
        div { class: "ex-panel ex-panel-left", id: "project-browser", role: "dialog", "aria-label": "Projects",
            header {
                h2 { "Projects" }
                button { class: "row-button", onclick: move |_| overlay.set(Overlay::None), "Close" }
            }
            button {
                class: "row-button",
                id: "project-import",
                onclick: move |_| {
                    let picked = rfd::FileDialog::new()
                        .set_title("Open a video or project")
                        .add_filter("Videos and projects", &["mp4", "mov", "m4v", "reel"])
                        .pick_file();
                    if let Some(path) = picked {
                        overlay.set(Overlay::None);
                        links::tell_editor(ToEditor::Open(path));
                    }
                },
                "Import a video or project…"
            }
            if entries.is_empty() {
                p { class: "panel-note", "Projects you make will show up here." }
            }
            for entry in entries {
                {
                    let is_current = current.as_deref() == Some(entry.path.as_path());
                    let path = entry.path.clone();
                    rsx! {
                        button {
                            class: "pb-row",
                            title: "{entry.path.display()}",
                            "data-selected": "{is_current}",
                            onclick: move |_| {
                                overlay.set(Overlay::None);
                                links::tell_editor(ToEditor::Open(path.clone()));
                            },
                            span { "{entry.name}" }
                            if is_current { span { class: "pb-badge", "Current" } }
                        }
                    }
                }
            }
        }
    }
}

/// Whether a preset's name is free. Names are unique regardless of case.
#[must_use]
pub fn name_is_free(presets: &[Preset], name: &str) -> bool {
    !presets
        .iter()
        .any(|preset| preset.name.eq_ignore_ascii_case(name.trim()))
}

#[component]
pub fn PresetsMenu() -> Element {
    let ed = use_ed();
    let mut overlay = ed.overlay;
    let mut stored = use_signal(|| SettingsStore::open(&settings_path()).get().presets.clone());
    let mut name = use_signal(String::new);
    let Some(project) = ed.project() else {
        return rsx! {};
    };
    let presets = stored();
    let can_save = !name().trim().is_empty() && name_is_free(&presets, &name());
    let for_save = project.clone();
    rsx! {
        div { class: "ex-panel", id: "presets-menu", role: "dialog", "aria-label": "Presets",
            header {
                h2 { "Presets" }
                button { class: "row-button", onclick: move |_| overlay.set(Overlay::None), "Close" }
            }
            if presets.is_empty() {
                p { class: "panel-note", "Save this project's look to use it again on others." }
            }
            for preset in presets.clone() {
                {
                    let active = preset.matches(&project);
                    let (apply, remove) = (preset.clone(), preset.name.clone());
                    rsx! {
                        div { class: "pb-row-group",
                            button {
                                class: "pb-row",
                                "data-selected": "{active}",
                                onclick: move |_| {
                                    ed.change("preset", |p| apply.apply_to(p));
                                    ed.end_gesture();
                                },
                                span { "{preset.name}" }
                            }
                            button {
                                class: "row-button",
                                "aria-label": "Delete {preset.name}",
                                onclick: move |_| {
                                    let mut store = SettingsStore::open(&settings_path());
                                    let _ = store.update(|settings| settings.delete_preset(&remove));
                                    stored.set(store.get().presets.clone());
                                },
                                "Delete"
                            }
                        }
                    }
                }
            }
            label { class: "row-text",
                span { class: "row-label", "Save the current look as" }
                input {
                    id: "preset-name",
                    r#type: "text",
                    value: "{name()}",
                    onkeydown: move |event| event.stop_propagation(),
                    oninput: move |event| name.set(event.value()),
                }
            }
            div { class: "row-buttons",
                button {
                    class: "row-button row-primary",
                    id: "preset-save",
                    disabled: !can_save,
                    onclick: move |_| {
                        let preset = Preset::from_project(name().trim(), &for_save);
                        let mut store = SettingsStore::open(&settings_path());
                        let _ = store.update(|settings| settings.save_preset(preset));
                        stored.set(store.get().presets.clone());
                        name.set(String::new());
                    },
                    "Save"
                }
            }
        }
    }
}

#[component]
pub fn ShortcutsDialog(bound: Signal<BTreeMap<String, String>>) -> Element {
    let ed = use_ed();
    let mut overlay = ed.overlay;
    let mut draft = use_signal(move || bound.read().clone());
    let mut listening = use_signal(|| None::<&'static str>);
    let mut conflict = use_signal(|| None::<(&'static str, &'static str, String, &'static str)>);
    let mut note = use_signal(|| None::<String>);
    rsx! {
        div {
            class: "ex-panel",
            id: "shortcuts-dialog",
            role: "dialog",
            "aria-label": "Keyboard shortcuts",
            tabindex: "0",
            onkeydown: move |event| {
                event.stop_propagation();
                let Some(name) = listening() else {
                    if event.key() == Key::Escape {
                        overlay.set(Overlay::None);
                    }
                    return;
                };
                event.prevent_default();
                let key = event.key().to_string();
                if keys::is_modifier(&key) {
                    return;
                }
                let modifiers = event.modifiers();
                let pressed = keys::describe(&key, modifiers.meta(), modifiers.ctrl(), modifiers.alt(), modifiers.shift());
                match keys::check_rebind(name, &pressed, &draft()) {
                    Rebind::Fine => {
                        draft.write().insert(name.to_string(), pressed);
                        listening.set(None);
                        note.set(None);
                    }
                    Rebind::Reserved => note.set(Some(format!("{pressed} is reserved, so choose another."))),
                    Rebind::Conflict { with, label } => {
                        conflict.set(Some((name, with, pressed, label)));
                        listening.set(None);
                    }
                }
            },
            header {
                h2 { "Keyboard shortcuts" }
            }
            for (_, name, label, default) in REBINDABLE {
                {
                    let current = keys::current_key(name, default, &draft()).to_string();
                    let is_listening = listening() == Some(name);
                    rsx! {
                        div { class: "pb-row-group",
                            span { class: "row-label", "{label}" }
                            button {
                                class: "row-button sc-key",
                                id: "shortcut-{name}",
                                "data-listening": "{is_listening}",
                                onclick: move |_| {
                                    note.set(None);
                                    conflict.set(None);
                                    listening.set(Some(name));
                                },
                                if is_listening { "Press keys…" } else { "{current}" }
                            }
                        }
                    }
                }
            }
            if let Some(text) = note() {
                p { class: "panel-error", role: "alert", "{text}" }
            }
            if let Some((name, with, pressed, label)) = conflict() {
                p { class: "panel-note", role: "alert", "{pressed} is already used by {label}." }
                div { class: "row-buttons",
                    button {
                        class: "row-button",
                        id: "shortcut-swap",
                        onclick: move |_| {
                            let (_, _, _, default) = REBINDABLE.iter().find(|(_, other, _, _)| *other == name).copied().unwrap_or(REBINDABLE[0]);
                            let old = keys::current_key(name, default, &draft()).to_string();
                            let mut map = draft.write();
                            map.insert(name.to_string(), pressed.clone());
                            map.insert(with.to_string(), old);
                            drop(map);
                            conflict.set(None);
                        },
                        "Swap"
                    }
                    button { class: "row-button", onclick: move |_| conflict.set(None), "Cancel" }
                }
            }
            div { class: "row-buttons",
                button {
                    class: "row-button",
                    id: "shortcuts-reset",
                    onclick: move |_| draft.set(BTreeMap::new()),
                    "Reset to defaults"
                }
                button { class: "row-button", onclick: move |_| overlay.set(Overlay::None), "Cancel" }
                button {
                    class: "row-button row-primary",
                    id: "shortcuts-save",
                    onclick: move |_| {
                        let chosen = draft();
                        let mut store = SettingsStore::open(&settings_path());
                        let saving = chosen.clone();
                        let _ = store.update(|settings| settings.shortcuts = saving);
                        bound.set(chosen);
                        overlay.set(Overlay::None);
                    },
                    "Save"
                }
            }
        }
    }
}

/// A project's name from its path.
#[must_use]
pub fn project_name(path: Option<&Path>) -> String {
    path.and_then(Path::file_stem).map_or_else(
        || "Untitled".to_string(),
        |stem| stem.to_string_lossy().into_owned(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use reel_core::project::Project;

    #[test]
    fn projects_are_listed_newest_first_without_repeats() {
        let folder = tempfile::tempdir().unwrap();
        let old = folder.path().join("old.reel");
        let new = folder.path().join("new.reel");
        std::fs::write(&old, "x").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(30));
        std::fs::write(&new, "x").unwrap();
        std::fs::write(folder.path().join("video.mp4"), "x").unwrap();
        let listed = list_projects(
            &[folder.path().to_path_buf()],
            &[old.clone(), folder.path().join("gone.reel")],
        );
        let names: Vec<_> = listed.iter().map(|entry| entry.name.as_str()).collect();
        assert_eq!(names, ["new", "old"]);
    }

    #[test]
    fn only_the_newest_are_listed() {
        let folder = tempfile::tempdir().unwrap();
        for index in 0..30 {
            std::fs::write(folder.path().join(format!("{index}.reel")), "x").unwrap();
        }
        assert_eq!(
            list_projects(&[folder.path().to_path_buf()], &[]).len(),
            NEWEST
        );
    }

    #[test]
    fn preset_names_are_unique_ignoring_case() {
        let presets = [Preset::from_project("Clean", &Project::default())];
        assert!(!name_is_free(&presets, "clean"));
        assert!(!name_is_free(&presets, " CLEAN "));
        assert!(name_is_free(&presets, "other"));
    }

    #[test]
    fn names_come_from_the_file() {
        assert_eq!(
            project_name(Some(Path::new("/a/my project.reel"))),
            "my project"
        );
        assert_eq!(project_name(None), "Untitled");
    }
}
