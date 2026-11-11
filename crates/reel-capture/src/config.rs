//! What the app tells the recorder process, and what the recorder says back.
//!
//! The app gives it a config as JSON, and then sends one command per line on stdin: `pause`, `resume`,
//! or `stop`. Stdin closing also stops and finishes the recording, so if the app dies the recording
//! still ends cleanly. The recorder reports one JSON event per line on stdout.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SourceChoice {
    Display {
        id: u32,
    },
    Window {
        id: u32,
    },
    /// A generated test pattern with flashes and beeps on the same clock, for tests that need no
    /// permissions: a flash and a beep at the same moment have to stay together.
    Synthetic {
        width: u32,
        height: u32,
        /// How long after the start the mic track's sound begins, to test a late-starting track.
        #[serde(default)]
        mic_delay_ms: u32,
    },
}

/// A device to record from. No id means the default.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceChoice {
    #[serde(default)]
    pub id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RecordConfig {
    pub source: SourceChoice,
    /// The video's path. Everything else goes beside it.
    pub output: PathBuf,
    #[serde(default = "default_fps")]
    pub fps: u32,
    #[serde(default)]
    pub system_audio: bool,
    #[serde(default)]
    pub microphone: Option<DeviceChoice>,
    #[serde(default)]
    pub webcam: Option<DeviceChoice>,
    #[serde(default = "yes")]
    pub cursor: bool,
    /// Whether to listen for clicks. Only tests turn this off.
    #[serde(default = "yes")]
    pub clicks: bool,
    /// Warm start: capture runs but nothing is kept until `resume`.
    #[serde(default)]
    pub start_paused: bool,
    /// Apps to keep out of a display recording, by bundle id, such as Reel's own windows.
    #[serde(default)]
    pub exclude_bundles: Vec<String>,
    /// Whether Reel's own windows stay out of a display recording.
    #[serde(default = "yes")]
    pub hide_own_windows: bool,
}

fn default_fps() -> u32 {
    60
}

fn yes() -> bool {
    true
}

/// Everything a recording writes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutputPaths {
    pub video: PathBuf,
    pub system_audio: PathBuf,
    pub microphone: PathBuf,
    pub webcam: PathBuf,
    pub cursor_log: PathBuf,
    pub cursor: PathBuf,
    pub session: PathBuf,
}

fn beside(video: &Path, suffix: &str) -> PathBuf {
    let mut name = video.as_os_str().to_os_string();
    name.push(suffix);
    PathBuf::from(name)
}

impl OutputPaths {
    /// These names are the ones the project importer already looks for.
    #[must_use]
    pub fn for_video(video: &Path) -> Self {
        let stem = video.file_stem().map_or_else(
            || "recording".to_string(),
            |stem| stem.to_string_lossy().into_owned(),
        );
        Self {
            video: video.to_path_buf(),
            system_audio: beside(video, ".system.m4a"),
            microphone: beside(video, ".mic.m4a"),
            webcam: video.with_file_name(format!("{stem}-webcam.mp4")),
            cursor_log: beside(video, ".cursor.jsonl"),
            cursor: beside(video, ".cursor.json"),
            session: beside(video, ".session.json"),
        }
    }
}

/// What the recorder reports.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum Event {
    /// The first frame has arrived. With a warm start, this is when the countdown can begin.
    Started {
        width: u32,
        height: u32,
    },
    Paused,
    Resumed,
    /// Something's missing but the recording carries on, like a mic that wouldn't start.
    Warning {
        message: String,
    },
    /// The window being recorded has gone. The recording is finishing.
    WindowUnavailable,
    /// The disk is nearly full. The recording is finishing while there's room to do so.
    LowDisk {
        free_bytes: u64,
    },
    /// Finished, and the file has passed its checks.
    Stopped {
        output: PathBuf,
        duration_ms: f64,
        frames: u64,
        dropped_frames: u64,
    },
    Failed {
        message: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_small_config_gets_sensible_defaults() {
        let config: RecordConfig = serde_json::from_str(
            r#"{ "source": { "kind": "display", "id": 7 }, "output": "/tmp/a.mp4" }"#,
        )
        .unwrap();
        assert_eq!(config.source, SourceChoice::Display { id: 7 });
        assert_eq!(config.fps, 60);
        assert!(config.cursor);
        assert!(!config.system_audio && config.microphone.is_none() && config.webcam.is_none());
        assert!(!config.start_paused);
    }

    #[test]
    fn files_are_named_so_the_importer_finds_them() {
        let paths = OutputPaths::for_video(Path::new("/r/recording-1.mp4"));
        assert_eq!(
            paths.system_audio,
            Path::new("/r/recording-1.mp4.system.m4a")
        );
        assert_eq!(paths.microphone, Path::new("/r/recording-1.mp4.mic.m4a"));
        assert_eq!(paths.webcam, Path::new("/r/recording-1-webcam.mp4"));
        assert_eq!(paths.cursor, Path::new("/r/recording-1.mp4.cursor.json"));
    }

    #[test]
    fn events_are_one_line_of_tagged_json() {
        let line = serde_json::to_string(&Event::Started {
            width: 10,
            height: 20,
        })
        .unwrap();
        assert_eq!(line, r#"{"event":"started","width":10,"height":20}"#);
        let back: Event = serde_json::from_str(&line).unwrap();
        assert_eq!(
            back,
            Event::Started {
                width: 10,
                height: 20
            }
        );
    }
}
