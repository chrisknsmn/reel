//! Running the recorder process, and everything that turns its output into a project.

use std::collections::HashSet;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use reel_capture::config::{Event, OutputPaths, RecordConfig};
use reel_capture::retention;
use reel_capture::session::{RecordingSession, find_interrupted};
use reel_capture::sources::{DisplaySource, WindowSource};
use reel_core::format;
use reel_core::project::Project;
use reel_core::time::Micros;
use serde::Deserialize;
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};

/// The recorder sits next to the app's own binary, in a bundle or a build folder.
fn recorder_path() -> PathBuf {
    let name = "reel-recorder";
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|folder| folder.join(name)))
        .filter(|path| path.exists())
        .unwrap_or_else(|| PathBuf::from(name))
}

/// Where Reel keeps its own files. Settings live here, and recordings unless the user says otherwise.
#[must_use]
pub fn support_folder() -> PathBuf {
    std::env::var_os("REEL_DATA").map_or_else(
        || {
            let home = std::env::var_os("HOME").map_or_else(|| PathBuf::from("."), PathBuf::from);
            home.join("Library/Application Support/Reel")
        },
        PathBuf::from,
    )
}

/// A new recording's video path, in `folder`.
#[must_use]
pub fn new_recording_path(folder: &Path) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_millis());
    folder.join(format!("recording-{stamp}.mp4"))
}

/// Runs a recorder command that just answers and exits.
async fn ask(arguments: Vec<String>) -> Result<serde_json::Value, String> {
    tokio::task::spawn_blocking(move || {
        let output = Command::new(recorder_path())
            .args(&arguments)
            .stdin(Stdio::null())
            .output()
            .map_err(|error| format!("The recorder couldn't be started: {error}"))?;
        let text = String::from_utf8_lossy(&output.stdout);
        let value: serde_json::Value = text
            .lines()
            .last()
            .and_then(|line| serde_json::from_str(line).ok())
            .ok_or_else(|| "The recorder gave no answer.".to_string())?;
        if value["event"] == "failed" {
            return Err(value["message"]
                .as_str()
                .unwrap_or("It failed.")
                .to_string());
        }
        Ok(value)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
pub struct Sources {
    pub displays: Vec<DisplaySource>,
    pub windows: Vec<WindowSource>,
}

/// What can be recorded. The system only says once Screen Recording is allowed.
pub async fn list_sources(thumbnails: &Path) -> Result<Sources, String> {
    let value = ask(vec![
        "list".to_string(),
        "--thumbnails".to_string(),
        thumbnails.to_string_lossy().into_owned(),
    ])
    .await?;
    serde_json::from_value(value).map_err(|error| error.to_string())
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
pub struct Device {
    pub id: String,
    pub name: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
pub struct Devices {
    pub microphones: Vec<Device>,
    pub cameras: Vec<Device>,
}

pub async fn list_devices() -> Result<Devices, String> {
    let value = ask(vec!["devices".to_string()]).await?;
    serde_json::from_value(value).map_err(|error| error.to_string())
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
pub struct Permissions {
    pub screen: String,
    pub microphone: String,
    pub camera: String,
}

impl Permissions {
    #[must_use]
    pub fn screen_allowed(&self) -> bool {
        self.screen == "granted"
    }

    #[must_use]
    pub fn microphone_allowed(&self) -> bool {
        self.microphone == "granted"
    }

    #[must_use]
    pub fn camera_allowed(&self) -> bool {
        self.camera == "granted"
    }
}

/// What's allowed. This never shows a prompt.
pub async fn permissions() -> Permissions {
    match ask(vec!["permissions".to_string()]).await {
        Ok(value) => serde_json::from_value(value).unwrap_or_default(),
        Err(_) => Permissions::default(),
    }
}

/// Shows the system's prompt, for something the user has just tried to use.
pub async fn request_permission(what: &str) -> bool {
    ask(vec!["request".to_string(), what.to_string()])
        .await
        .is_ok_and(|value| value["granted"] == true)
}

/// A recording in progress, in its own process.
pub struct RecorderProcess {
    child: Child,
    stdin: Option<ChildStdin>,
    pub events: UnboundedReceiver<Event>,
    pub output: PathBuf,
}

impl RecorderProcess {
    /// Starts recording.
    ///
    /// # Errors
    /// Returns an error if the process can't be started. Everything after that arrives as events.
    pub fn start(config: &RecordConfig) -> Result<Self, String> {
        let text = serde_json::to_string(config).map_err(|error| error.to_string())?;
        let mut child = Command::new(recorder_path())
            .arg("record")
            .arg(text)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|error| format!("The recorder couldn't be started: {error}"))?;
        let stdin = child.stdin.take();
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "The recorder has no output.".to_string())?;
        let (send, events) = unbounded_channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                if let Ok(event) = serde_json::from_str::<Event>(&line)
                    && send.send(event).is_err()
                {
                    break;
                }
            }
        });
        Ok(Self {
            child,
            stdin,
            events,
            output: config.output.clone(),
        })
    }

    fn send(&mut self, command: &str) {
        if let Some(stdin) = self.stdin.as_mut() {
            let _ = writeln!(stdin, "{command}");
            let _ = stdin.flush();
        }
    }

    pub fn pause(&mut self) {
        self.send("pause");
    }

    pub fn resume(&mut self) {
        self.send("resume");
    }

    pub fn stop(&mut self) {
        self.send("stop");
    }

    /// Ends it and throws away what it wrote.
    pub fn discard(mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        delete_recording(&self.output);
    }

    /// Waits for the process to go, after it's said it's finished.
    pub fn wait(mut self) {
        let _ = self.child.wait();
    }
}

/// Deletes a recording and everything beside it.
pub fn delete_recording(video: &Path) {
    for file in retention::files_of(video) {
        let _ = std::fs::remove_file(file);
    }
}

/// Turns a finished recording into a saved project beside it, and returns the project's path.
///
/// # Errors
/// Returns an error if its note can't be read or the project can't be written.
pub fn save_project_for(video: &Path, duration: Micros) -> Result<PathBuf, String> {
    let paths = OutputPaths::for_video(video);
    let session = RecordingSession::load(&paths.session)
        .map_err(|error| format!("The recording's details can't be read: {error}"))?;
    let mut project = session.to_project(duration);
    let settings =
        reel_core::settings::SettingsStore::open(&support_folder().join("settings.json"));
    if settings.get().auto_suggest_zooms {
        let track = session
            .cursor
            .as_ref()
            .and_then(|path| std::fs::read_to_string(path).ok())
            .and_then(|text| reel_motion::cursor::CursorTrack::parse(&text).ok());
        let size = reel_media::macos::probe(video)
            .ok()
            .map(|info| (info.width, info.height));
        if let (Some(track), Some(size)) = (track, size) {
            add_suggested_zooms(&mut project, &track, size);
        }
    }
    write_project(video, &project)
}

/// Adds zooms around clusters of clicks, unless the video is close to square or portrait.
pub fn add_suggested_zooms(
    project: &mut Project,
    track: &reel_motion::cursor::CursorTrack,
    size: (u32, u32),
) {
    if !reel_motion::suggest::suits_auto_zoom(size.0, size.1) {
        return;
    }
    for suggestion in reel_motion::suggest::suggest_zooms(track, &project.timeline) {
        let _ = project.timeline.apply(&reel_core::edit::Edit::AddZoom {
            start: suggestion.start,
            depth: 1,
            focus: suggestion.focus,
            mode: reel_core::timeline::ZoomMode::Auto,
        });
    }
}

fn write_project(video: &Path, project: &Project) -> Result<PathBuf, String> {
    let path = video.with_extension(format::EXTENSION);
    reel_core::store::atomic_write(&path, format::to_string(project).as_bytes(), false)
        .map_err(|error| format!("{} can't be written: {error}", path.display()))?;
    Ok(path)
}

/// A recording that was saved after being cut off.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecoveredRecording {
    pub project: PathBuf,
}

/// Saves whatever was cut off in a folder, by a crash or a power cut.
pub async fn recover_interrupted(folder: PathBuf) -> Vec<Result<RecoveredRecording, String>> {
    let found = tokio::task::spawn_blocking(move || find_interrupted(&folder))
        .await
        .unwrap_or_default();
    let mut results = Vec::new();
    for (session_path, session) in found {
        let result = ask(vec![
            "recover".to_string(),
            session_path.to_string_lossy().into_owned(),
        ])
        .await
        .and_then(|value| {
            let duration = Micros::from_ms(value["duration_ms"].as_f64().unwrap_or(0.0));
            save_project_for(&session.video, duration)
        })
        .map(|project| RecoveredRecording { project });
        results.push(result);
    }
    results
}

/// Applies the retention rules, if the user has turned them on (D7). Anything a project in the folder
/// uses is kept, as is the recording just made.
pub fn apply_retention(folder: &Path, current: &Path) -> Vec<PathBuf> {
    let mut spared = HashSet::from([current.to_path_buf()]);
    if let Ok(entries) = std::fs::read_dir(folder) {
        for entry in entries.flatten() {
            let path = entry.path();
            let is_project = path
                .extension()
                .is_some_and(|extension| extension == format::EXTENSION);
            // A recording's own project doesn't count as using it. Only one that's been edited does.
            if is_project
                && let Ok(text) = std::fs::read_to_string(&path)
                && let Ok(project) = format::from_str(&text)
                && project_was_edited(&path, &project)
            {
                spared.insert(project.media.video.resolve(path.parent()));
            }
        }
    }
    retention::prune(folder, &spared, SystemTime::now())
}

/// Whether a project has been changed since it was made for its recording.
fn project_was_edited(path: &Path, project: &Project) -> bool {
    let video = project.media.video.resolve(path.parent());
    let session = RecordingSession::load(&OutputPaths::for_video(&video).session);
    let Ok(session) = session else { return true };
    let mut untouched = session.to_project(project.timeline.duration());
    untouched.id.clone_from(&project.id);
    untouched != *project
}

#[cfg(test)]
mod tests {
    use super::*;
    use reel_core::time::SourceTime;
    use reel_core::timeline::Timeline;
    use reel_motion::cursor::{CursorSample, CursorShape, CursorTrack, Interaction};

    fn clicking_track() -> CursorTrack {
        let mut samples = Vec::new();
        for index in 0..300_u32 {
            let ms = f64::from(index) * 33.0;
            let interaction = if (60..120).contains(&index) && index % 10 == 0 {
                Interaction::Click
            } else {
                Interaction::Move
            };
            samples.push(CursorSample {
                time: SourceTime::from_ms(ms),
                x: 0.3,
                y: 0.3,
                interaction,
                shape: CursorShape::Arrow,
            });
        }
        CursorTrack { samples }
    }

    fn project() -> Project {
        Project {
            timeline: Timeline::for_recording(Micros::from_secs(10)),
            ..Project::default()
        }
    }

    #[test]
    fn a_wide_recording_with_clicks_gets_zooms() {
        let mut project = project();
        add_suggested_zooms(&mut project, &clicking_track(), (1920, 1080));
        assert!(!project.timeline.zooms.is_empty());
    }

    #[test]
    fn a_nearly_square_or_portrait_one_doesnt() {
        for size in [(1000, 1000), (1080, 1920)] {
            let mut project = project();
            add_suggested_zooms(&mut project, &clicking_track(), size);
            assert!(project.timeline.zooms.is_empty(), "{size:?}");
        }
    }

    #[test]
    fn no_clicks_means_no_zooms() {
        let mut project = project();
        add_suggested_zooms(&mut project, &CursorTrack::default(), (1920, 1080));
        assert!(project.timeline.zooms.is_empty());
    }

    #[test]
    fn recordings_get_names_that_retention_recognises() {
        let path = new_recording_path(Path::new("/r"));
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        assert!(
            name.starts_with("recording-") && name.ends_with(".mp4"),
            "{name}"
        );
    }
}
