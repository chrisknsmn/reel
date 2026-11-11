//! App settings, all in one Rust-owned file (BUILD_PLAN 2.8).
//!
//! There's one store, and every change writes the whole thing from memory, so one setting can never
//! overwrite the others. Keys this version doesn't know are kept, so a newer version's settings survive too.

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::project::{CaptionStyle, CursorLook, ExportSettings, Look, Project, Webcam, ZoomMotion};
use crate::store;

/// How many recent projects to remember.
pub const RECENT_PROJECTS: usize = 16;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Theme {
    Light,
    Dark,
    #[default]
    System,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MotionPreset {
    #[default]
    Focused,
    Smooth,
}

/// A saved look, without any timeline items.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Preset {
    pub name: String,
    pub look: Look,
    pub zoom_motion: ZoomMotion,
    pub cursor: CursorLook,
    pub webcam: Webcam,
    pub export: ExportSettings,
    pub caption_style: CaptionStyle,
}

impl Preset {
    #[must_use]
    pub fn from_project(name: &str, project: &Project) -> Self {
        Self {
            name: name.to_string(),
            look: project.look.clone(),
            zoom_motion: project.zoom_motion.clone(),
            cursor: project.cursor.clone(),
            webcam: project.webcam.clone(),
            export: project.export.clone(),
            caption_style: project.caption_style.clone(),
        }
    }

    /// Puts this look on a project, leaving its timeline and media alone.
    pub fn apply_to(&self, project: &mut Project) {
        project.look = self.look.clone();
        project.zoom_motion = self.zoom_motion.clone();
        project.cursor = self.cursor.clone();
        project.webcam = Webcam {
            // Which footage it uses belongs to the project.
            enabled: project.webcam.enabled,
            source: project.webcam.source.clone(),
            offset: project.webcam.offset,
            ..self.webcam.clone()
        };
        project.export = self.export.clone();
        project.caption_style = self.caption_style.clone();
    }

    /// Whether a project's look matches this, which is how the active preset is found.
    #[must_use]
    pub fn matches(&self, project: &Project) -> bool {
        let mut applied = project.clone();
        self.apply_to(&mut applied);
        applied == *project
    }
}

/// The look new recordings start with.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Defaults {
    pub look: Look,
    pub zoom_motion: ZoomMotion,
    pub cursor: CursorLook,
    pub webcam: Webcam,
    pub export: ExportSettings,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub theme: Theme,
    pub language: String,
    pub experimental_updates: bool,

    pub hide_bar_from_recording: bool,
    pub bar_position: Option<(f64, f64)>,
    pub countdown_seconds: u32,

    pub recordings_dir: Option<PathBuf>,
    pub microphone_enabled: bool,
    pub system_audio_enabled: bool,
    pub microphone_device: Option<String>,
    /// Whether old recordings are deleted. It's opt-in, so nothing goes without the user asking (D7).
    pub retention_enabled: bool,

    pub auto_suggest_zooms: bool,
    pub motion_preset: MotionPreset,
    pub whisper_model: Option<PathBuf>,

    pub shortcuts: BTreeMap<String, String>,
    pub recent_projects: Vec<PathBuf>,
    pub presets: Vec<Preset>,
    pub defaults: Defaults,

    /// Anything this version doesn't recognise, kept as it was.
    #[serde(flatten)]
    pub unknown: BTreeMap<String, serde_json::Value>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: Theme::System,
            language: "en".to_string(),
            experimental_updates: false,
            hide_bar_from_recording: true,
            bar_position: None,
            countdown_seconds: 3,
            recordings_dir: None,
            microphone_enabled: false,
            system_audio_enabled: false,
            microphone_device: None,
            retention_enabled: false,
            auto_suggest_zooms: true,
            motion_preset: MotionPreset::Focused,
            whisper_model: None,
            shortcuts: BTreeMap::new(),
            recent_projects: Vec::new(),
            presets: Vec::new(),
            defaults: Defaults::default(),
            unknown: BTreeMap::new(),
        }
    }
}

impl Settings {
    /// Moves a project to the top of the recent list.
    pub fn remember_project(&mut self, path: &Path) {
        self.recent_projects.retain(|existing| existing != path);
        self.recent_projects.insert(0, path.to_path_buf());
        self.recent_projects.truncate(RECENT_PROJECTS);
    }

    /// Saves a preset under a name. Names are unique, ignoring case.
    pub fn save_preset(&mut self, preset: Preset) {
        match self
            .presets
            .iter_mut()
            .find(|existing| existing.name.eq_ignore_ascii_case(&preset.name))
        {
            Some(existing) => *existing = preset,
            None => self.presets.push(preset),
        }
    }

    pub fn delete_preset(&mut self, name: &str) {
        self.presets
            .retain(|preset| !preset.name.eq_ignore_ascii_case(name));
    }
}

/// The one place settings are read and written.
pub struct SettingsStore {
    path: PathBuf,
    settings: Settings,
}

impl SettingsStore {
    /// Loads settings. A missing file gives defaults. A damaged one is set aside as `.corrupt`, so
    /// the app still starts and nothing is lost for good.
    #[must_use]
    pub fn open(path: &Path) -> Self {
        let settings = match std::fs::read_to_string(path) {
            Ok(text) => serde_json::from_str(text.strip_prefix('\u{feff}').unwrap_or(&text))
                .unwrap_or_else(|_| {
                    let mut aside = path.as_os_str().to_os_string();
                    aside.push(".corrupt");
                    let _ = std::fs::rename(path, aside);
                    Settings::default()
                }),
            Err(_) => Settings::default(),
        };
        Self {
            path: path.to_path_buf(),
            settings,
        }
    }

    #[must_use]
    pub fn get(&self) -> &Settings {
        &self.settings
    }

    /// Changes settings and saves them all, atomically.
    ///
    /// # Errors
    /// Returns any error from writing the file. The new values stay in memory either way.
    pub fn update(&mut self, change: impl FnOnce(&mut Settings)) -> io::Result<()> {
        change(&mut self.settings);
        let mut text = serde_json::to_string_pretty(&self.settings).map_err(io::Error::other)?;
        text.push('\n');
        store::atomic_write(&self.path, text.as_bytes(), false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn choosing_a_recordings_folder_keeps_the_audio_settings() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let mut store = SettingsStore::open(&path);
        store
            .update(|settings| {
                settings.microphone_enabled = true;
                settings.system_audio_enabled = true;
                settings.microphone_device = Some("usb-mic".to_string());
            })
            .unwrap();
        store
            .update(|settings| settings.recordings_dir = Some(PathBuf::from("/videos")))
            .unwrap();

        let reopened = SettingsStore::open(&path);
        assert!(reopened.get().microphone_enabled);
        assert!(reopened.get().system_audio_enabled);
        assert_eq!(reopened.get().microphone_device.as_deref(), Some("usb-mic"));
        assert_eq!(
            reopened.get().recordings_dir,
            Some(PathBuf::from("/videos"))
        );
    }

    #[test]
    fn a_missing_file_gives_the_documented_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let store = SettingsStore::open(&dir.path().join("nope.json"));
        assert_eq!(store.get().countdown_seconds, 3);
        assert!(store.get().hide_bar_from_recording);
        assert!(
            !store.get().retention_enabled,
            "D7 makes deleting old recordings opt-in"
        );
    }

    #[test]
    fn a_damaged_file_is_set_aside_rather_than_crashing_or_being_lost() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(&path, b"{ not json").unwrap();
        let store = SettingsStore::open(&path);
        assert_eq!(store.get().countdown_seconds, 3);
        assert_eq!(
            std::fs::read(dir.path().join("settings.json.corrupt")).unwrap(),
            b"{ not json"
        );
    }

    #[test]
    fn settings_from_a_newer_version_survive_a_save() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(
            &path,
            br#"{ "countdown_seconds": 5, "from_the_future": { "a": 1 } }"#,
        )
        .unwrap();
        let mut store = SettingsStore::open(&path);
        store
            .update(|settings| settings.countdown_seconds = 10)
            .unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("from_the_future"));
        assert_eq!(SettingsStore::open(&path).get().countdown_seconds, 10);
    }

    #[test]
    fn recent_projects_stay_unique_newest_first_and_limited() {
        let mut settings = Settings::default();
        for n in 0..20 {
            settings.remember_project(Path::new(&format!("/p{n}.reel")));
        }
        settings.remember_project(Path::new("/p5.reel"));
        assert_eq!(settings.recent_projects.len(), RECENT_PROJECTS);
        assert_eq!(settings.recent_projects[0], Path::new("/p5.reel"));
        assert_eq!(
            settings
                .recent_projects
                .iter()
                .filter(|p| *p == Path::new("/p5.reel"))
                .count(),
            1
        );
    }

    #[test]
    fn preset_names_are_unique_ignoring_case() {
        let mut settings = Settings::default();
        settings.save_preset(Preset::from_project("Clean", &Project::default()));
        let mut changed = Project::default();
        changed.look.shadow = 0.1;
        settings.save_preset(Preset::from_project("CLEAN", &changed));
        assert_eq!(settings.presets.len(), 1);
        assert_eq!(settings.presets[0].look.shadow, 0.1);
        settings.delete_preset("clean");
        assert!(settings.presets.is_empty());
    }

    #[test]
    fn a_preset_changes_the_look_and_leaves_the_timeline_and_footage_alone() {
        let mut source = Project::default();
        source.look.shadow = 0.1;
        let preset = Preset::from_project("A", &source);

        let mut target = Project {
            timeline: crate::timeline::Timeline::for_recording(crate::time::Micros::from_secs(5)),
            ..Project::default()
        };
        target.webcam.enabled = true;
        target.webcam.source = Some(crate::project::MediaRef::absolute("/cam.mp4"));
        preset.apply_to(&mut target);

        assert_eq!(target.look.shadow, 0.1);
        assert_eq!(target.timeline.clips.len(), 1);
        assert!(target.webcam.enabled);
        assert!(target.webcam.source.is_some());
    }

    #[test]
    fn the_active_preset_is_found_by_its_values() {
        let mut project = Project::default();
        let preset = Preset::from_project("A", &project);
        assert!(preset.matches(&project));
        project.look.shadow = 0.1;
        assert!(!preset.matches(&project));
    }
}
