//! A note on disk of what a recording is writing. It's what makes recovery possible: if the note says
//! a recording is still going but its process is gone, the files are what's left of a crash.

use std::path::{Path, PathBuf};

use reel_core::project::{MediaRef, Project, SourceTrack, SourceTrackKind};
use reel_core::store::atomic_write;
use reel_core::time::Micros;
use reel_core::timeline::Timeline;
use serde::{Deserialize, Serialize};

use crate::config::OutputPaths;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionState {
    Recording,
    Finished,
    /// A crash was found, and what could be saved has been.
    Recovered,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RecordingSession {
    pub version: u32,
    pub state: SessionState,
    pub pid: u32,
    pub video: PathBuf,
    #[serde(default)]
    pub system_audio: Option<PathBuf>,
    #[serde(default)]
    pub microphone: Option<PathBuf>,
    #[serde(default)]
    pub webcam: Option<PathBuf>,
    /// How long after the recording's first frame the webcam's first frame came.
    #[serde(default)]
    pub webcam_offset_us: i64,
    #[serde(default)]
    pub cursor: Option<PathBuf>,
    #[serde(default)]
    pub cursor_log: Option<PathBuf>,
}

impl RecordingSession {
    #[must_use]
    pub fn new(paths: &OutputPaths, pid: u32) -> Self {
        Self {
            version: 1,
            state: SessionState::Recording,
            pid,
            video: paths.video.clone(),
            system_audio: None,
            microphone: None,
            webcam: None,
            webcam_offset_us: 0,
            cursor: None,
            cursor_log: None,
        }
    }

    /// Saves it, all at once so a crash can't leave half a note.
    ///
    /// # Errors
    /// Returns an error if the file can't be written.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let text = serde_json::to_string_pretty(self)?;
        atomic_write(path, text.as_bytes(), false)
    }

    /// # Errors
    /// Returns an error if the file can't be read or isn't a session.
    pub fn load(path: &Path) -> std::io::Result<Self> {
        let text = std::fs::read_to_string(path)?;
        serde_json::from_str(&text).map_err(std::io::Error::other)
    }

    /// A project for this recording, with every track linked up, so both audio tracks always reach
    /// the editor.
    #[must_use]
    pub fn to_project(&self, duration: Micros) -> Project {
        let mut project = Project {
            id: reel_core::project::new_id(),
            timeline: Timeline::for_recording(duration),
            ..Project::default()
        };
        project.media.video = MediaRef::absolute(self.video.clone());
        project.media.cursor_data = self
            .cursor
            .as_ref()
            .filter(|path| path.exists())
            .map(|path| MediaRef::absolute(path.clone()));
        let mut track = |kind: SourceTrackKind, file: &Option<PathBuf>| {
            if let Some(file) = file.as_ref().filter(|path| path.exists()) {
                project.media.source_tracks.push(SourceTrack {
                    kind,
                    file: Some(MediaRef::absolute(file.clone())),
                    volume: 1.0,
                    boost: false,
                });
            }
        };
        track(SourceTrackKind::System, &self.system_audio);
        track(SourceTrackKind::Mic, &self.microphone);
        if let Some(webcam) = self.webcam.as_ref().filter(|path| path.exists()) {
            project.webcam.enabled = true;
            project.webcam.source = Some(MediaRef::absolute(webcam.clone()));
            project.webcam.offset = Micros(self.webcam_offset_us);
        }
        project
    }
}

/// Whether a process is still running.
#[must_use]
pub fn process_is_alive(pid: u32) -> bool {
    #[cfg(unix)]
    {
        let Ok(pid) = libc::pid_t::try_from(pid) else {
            return false;
        };
        // SAFETY: signal 0 sends nothing, and only checks whether the process can be reached.
        let result = unsafe { libc::kill(pid, 0) };
        result == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        false
    }
}

/// Finds recordings in a folder that were cut off: their note says recording, but nothing is.
#[must_use]
pub fn find_interrupted(folder: &Path) -> Vec<(PathBuf, RecordingSession)> {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return Vec::new();
    };
    let mut found: Vec<_> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .is_some_and(|name| name.to_string_lossy().ends_with(".session.json"))
        })
        .filter_map(|path| Some((path.clone(), RecordingSession::load(&path).ok()?)))
        .filter(|(_, session)| {
            session.state == SessionState::Recording && !process_is_alive(session.pid)
        })
        .collect();
    found.sort_by(|a, b| a.0.cmp(&b.0));
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A pid nothing will have, since it's above the kernel's limit.
    const GONE: u32 = 99_999_999;

    fn session_in(folder: &Path, name: &str, pid: u32, state: SessionState) -> PathBuf {
        let video = folder.join(format!("{name}.mp4"));
        let paths = OutputPaths::for_video(&video);
        let session = RecordingSession {
            state,
            ..RecordingSession::new(&paths, pid)
        };
        session.save(&paths.session).unwrap();
        paths.session
    }

    #[test]
    fn a_session_survives_saving_and_loading() {
        let folder = tempfile::tempdir().unwrap();
        let path = session_in(folder.path(), "a", 5, SessionState::Finished);
        let loaded = RecordingSession::load(&path).unwrap();
        assert_eq!(loaded.pid, 5);
        assert_eq!(loaded.state, SessionState::Finished);
    }

    #[test]
    fn this_process_is_alive_and_a_made_up_one_isnt() {
        assert!(process_is_alive(std::process::id()));
        assert!(!process_is_alive(GONE));
    }

    #[test]
    fn only_cut_off_recordings_count_as_interrupted() {
        let folder = tempfile::tempdir().unwrap();
        let dead = session_in(folder.path(), "dead", GONE, SessionState::Recording);
        session_in(
            folder.path(),
            "live",
            std::process::id(),
            SessionState::Recording,
        );
        session_in(folder.path(), "done", GONE, SessionState::Finished);
        session_in(folder.path(), "saved", GONE, SessionState::Recovered);
        let found = find_interrupted(folder.path());
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].0, dead);
    }

    #[test]
    fn a_damaged_note_is_ignored_rather_than_stopping_the_search() {
        let folder = tempfile::tempdir().unwrap();
        std::fs::write(folder.path().join("bad.session.json"), "{ not json").unwrap();
        assert!(find_interrupted(folder.path()).is_empty());
        assert!(find_interrupted(&folder.path().join("missing")).is_empty());
    }

    #[test]
    fn a_project_links_every_track_that_exists() {
        let folder = tempfile::tempdir().unwrap();
        let paths = OutputPaths::for_video(&folder.path().join("a.mp4"));
        for path in [
            &paths.system_audio,
            &paths.microphone,
            &paths.webcam,
            &paths.cursor,
        ] {
            std::fs::write(path, b"x").unwrap();
        }
        let session = RecordingSession {
            system_audio: Some(paths.system_audio.clone()),
            microphone: Some(paths.microphone.clone()),
            webcam: Some(paths.webcam.clone()),
            webcam_offset_us: 120_000,
            cursor: Some(paths.cursor.clone()),
            ..RecordingSession::new(&paths, 1)
        };
        let project = session.to_project(Micros::from_secs(10));
        assert_eq!(project.timeline.duration(), Micros::from_secs(10));
        assert_eq!(
            project.media.source_tracks.len(),
            2,
            "both the system audio and the mic reach the editor"
        );
        assert!(
            project
                .media
                .source_tracks
                .iter()
                .any(|track| track.kind == SourceTrackKind::Mic)
        );
        assert!(project.media.cursor_data.is_some());
        assert!(project.webcam.enabled);
        assert_eq!(project.webcam.offset, Micros(120_000));
    }

    #[test]
    fn files_that_never_appeared_arent_linked() {
        let folder = tempfile::tempdir().unwrap();
        let paths = OutputPaths::for_video(&folder.path().join("a.mp4"));
        let session = RecordingSession {
            system_audio: Some(paths.system_audio.clone()),
            webcam: Some(paths.webcam.clone()),
            cursor: Some(paths.cursor.clone()),
            ..RecordingSession::new(&paths, 1)
        };
        let project = session.to_project(Micros::from_secs(1));
        assert!(project.media.source_tracks.is_empty());
        assert!(project.media.cursor_data.is_none());
        assert!(!project.webcam.enabled);
    }
}
