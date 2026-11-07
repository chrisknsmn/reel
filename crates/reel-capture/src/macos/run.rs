//! Running a whole recording, from the config to the finished files.

use std::sync::Arc;
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

use reel_core::time::Micros;

use super::devices::DeviceCapture;
use super::permissions::{Access, screen_recording};
use super::pointer::{CursorTracker, POLL_SHAPE_EVERY};
use super::recorder::{Emit, Finished, Recorder};
use super::screen::{self, ScreenCapture};
use super::synthetic::Synthetic;
use super::windows::window_exists;
use crate::config::{Event, RecordConfig, SourceChoice};
use crate::disk;

const WINDOW_CHECK: Duration = Duration::from_millis(500);
const DISK_CHECK: Duration = Duration::from_secs(2);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    Pause,
    Resume,
    Stop,
}

enum Source {
    Screen(ScreenCapture),
    Synthetic(Synthetic),
}

impl Source {
    fn stop(self) {
        match self {
            Self::Screen(capture) => capture.stop(),
            Self::Synthetic(synthetic) => synthetic.stop(),
        }
    }
}

/// Runs a recording until it's told to stop, or something stops it.
///
/// # Errors
/// Returns why nothing usable came of it.
pub fn record(
    config: &RecordConfig,
    commands: &Receiver<Command>,
    emit: Emit,
) -> Result<Finished, String> {
    if let Some(folder) = config.output.parent() {
        std::fs::create_dir_all(folder)
            .map_err(|error| format!("{} can't be made: {error}", folder.display()))?;
    }
    if !disk::may_start(disk::free_bytes(&config.output)) {
        return Err("There isn't enough disk space to start recording.".to_string());
    }
    let is_real = !matches!(config.source, SourceChoice::Synthetic { .. });
    if is_real && screen_recording() != Access::Granted {
        return Err("Reel needs the Screen Recording permission to record.".to_string());
    }

    let size = match &config.source {
        SourceChoice::Synthetic { width, height, .. } => (*width & !1, *height & !1),
        source => screen::size_of(source)?,
    };
    let recorder = Arc::new(Recorder::new(config, size, emit.clone())?);

    let source = match &config.source {
        SourceChoice::Synthetic { mic_delay_ms, .. } => Source::Synthetic(Synthetic::start(
            recorder.clone(),
            size,
            config.fps,
            Micros::from_ms(f64::from(*mic_delay_ms)),
        )?),
        _ => Source::Screen(ScreenCapture::start(config, size, recorder.clone())?),
    };

    // A mic or webcam that won't start doesn't stop the recording, and the user is told (§4, §5). A
    // test pattern makes its own mic sound.
    let microphone = match (&config.microphone, is_real) {
        (Some(choice), true) => match DeviceCapture::microphone(choice, recorder.clone()) {
            Ok(capture) => Some(capture),
            Err(error) => {
                recorder.warn(format!("The microphone won't be recorded: {error}"));
                None
            }
        },
        _ => None,
    };
    let webcam = match &config.webcam {
        Some(choice) => match DeviceCapture::camera(choice, recorder.clone()) {
            Ok(capture) => Some(capture),
            Err(error) => {
                recorder.warn(format!("The webcam won't be recorded: {error}"));
                None
            }
        },
        None => None,
    };
    let cursor = config
        .cursor
        .then(|| CursorTracker::start(recorder.clone(), &config.source, size, config.clicks));

    let mut last_window_check = Instant::now();
    let mut last_disk_check = Instant::now();
    loop {
        match commands.recv_timeout(POLL_SHAPE_EVERY) {
            Ok(Command::Pause) => recorder.pause(),
            Ok(Command::Resume) => recorder.resume(),
            Ok(Command::Stop) | Err(RecvTimeoutError::Disconnected) => break,
            Err(RecvTimeoutError::Timeout) => {}
        }
        if let Some(cursor) = &cursor {
            cursor.poll_shape();
        }
        if recorder.problem().is_some() || recorder.source_has_ended() {
            break;
        }
        if let SourceChoice::Window { id } = &config.source
            && last_window_check.elapsed() >= WINDOW_CHECK
        {
            last_window_check = Instant::now();
            if !window_exists(*id) {
                emit(&Event::WindowUnavailable);
                break;
            }
        }
        if last_disk_check.elapsed() >= DISK_CHECK {
            last_disk_check = Instant::now();
            if let Some(free) = disk::free_bytes(&config.output)
                && free < disk::MIN_TO_CONTINUE
            {
                emit(&Event::LowDisk { free_bytes: free });
                break;
            }
        }
    }

    source.stop();
    if let Some(capture) = microphone {
        capture.stop();
    }
    if let Some(capture) = webcam {
        capture.stop();
    }
    if let Some(cursor) = cursor {
        cursor.stop();
    }
    let finished = recorder.finish();
    if let Some(problem) = recorder.problem() {
        // Finished or not, whatever was written is still on disk for recovery to find.
        return Err(problem);
    }
    finished
}
