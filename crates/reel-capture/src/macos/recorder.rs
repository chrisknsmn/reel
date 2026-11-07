//! The heart of a recording. Capture sources push frames and sound in from their own threads, and this
//! puts each on the recording's timeline and into its file.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use objc2_core_foundation::CFRetained;
use objc2_core_media::CMSampleBuffer;
use objc2_core_video::CVPixelBuffer;
use reel_core::time::Micros;
use reel_media::macos::record::{AudioRecorder, RecordSettings, VideoRecorder, host_now};
use reel_media::macos::{Decoder, VideoInfo, probe};
use reel_motion::cursor::{CursorShape, Interaction};

use crate::clock::RecordingClock;
use crate::config::{Event, OutputPaths, RecordConfig};
use crate::cursor_log::{self, CursorLog, Sample};
use crate::session::{RecordingSession, SessionState};
use crate::sources::recording_bitrate;

const SYSTEM_AUDIO_BITRATE: u32 = 160_000;
const MIC_BITRATE: u32 = 128_000;
const WEBCAM_BITRATE: u32 = 8_000_000;
const WEBCAM_FPS: u32 = 30;

pub type Emit = Arc<dyn Fn(&Event) + Send + Sync>;

/// Which sound track something belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Track {
    System,
    Microphone,
}

/// A frame held on to for the tail frame at the end.
struct Held(CFRetained<CVPixelBuffer>);

// SAFETY: once a pixel buffer has been delivered it isn't written to, so sharing it between threads is
// safe, and CoreVideo's reference counting is thread-safe.
unsafe impl Send for Held {}

struct WebcamTrack {
    recorder: Option<VideoRecorder>,
    /// Where the webcam's first frame fell on the recording's timeline.
    first: Option<Micros>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// How a finished recording went.
#[derive(Clone, Debug, PartialEq)]
pub struct Finished {
    pub duration: Micros,
    pub frames: u64,
    pub dropped: u64,
    pub session: RecordingSession,
}

pub struct Recorder {
    paths: OutputPaths,
    size: (u32, u32),
    clock: Mutex<RecordingClock>,
    video: Mutex<Option<VideoRecorder>>,
    last_frame: Mutex<Option<Held>>,
    system: Mutex<Option<AudioRecorder>>,
    microphone: Mutex<Option<AudioRecorder>>,
    webcam: Mutex<WebcamTrack>,
    cursor: Mutex<Option<CursorLog>>,
    session: Mutex<RecordingSession>,
    started: AtomicBool,
    fail_soft: AtomicBool,
    problem: Mutex<Option<String>>,
    emit: Emit,
}

impl Recorder {
    /// Opens all the files a recording needs.
    ///
    /// # Errors
    /// Returns why the recording can't start. A sound or webcam track that won't open isn't one of
    /// those reasons: the recording carries on without it, and says so.
    pub fn new(config: &RecordConfig, size: (u32, u32), emit: Emit) -> Result<Self, String> {
        let paths = OutputPaths::for_video(&config.output);
        let fps = config.fps.max(1);
        let video = VideoRecorder::create(
            &paths.video,
            RecordSettings {
                width: size.0,
                height: size.1,
                fps,
                bitrate: recording_bitrate(size.0, size.1),
                // A keyframe about every second keeps seeking in the editor quick.
                keyframe_interval: fps,
            },
        )
        .map_err(|error| error.0)?;

        let mut session = RecordingSession::new(&paths, std::process::id());
        let open_audio = |wanted: bool, path: &Path, bitrate: u32, name: &str| {
            if !wanted {
                return None;
            }
            match AudioRecorder::create(path, bitrate) {
                Ok(recorder) => Some(recorder),
                Err(error) => {
                    emit(&Event::Warning {
                        message: format!("{name} won't be recorded: {error}"),
                    });
                    None
                }
            }
        };
        let system = open_audio(
            config.system_audio,
            &paths.system_audio,
            SYSTEM_AUDIO_BITRATE,
            "System audio",
        );
        let microphone = open_audio(
            config.microphone.is_some(),
            &paths.microphone,
            MIC_BITRATE,
            "The microphone",
        );
        if system.is_some() {
            session.system_audio = Some(paths.system_audio.clone());
        }
        if microphone.is_some() {
            session.microphone = Some(paths.microphone.clone());
        }
        let cursor = if config.cursor {
            match CursorLog::create(&paths.cursor_log) {
                Ok(log) => {
                    session.cursor_log = Some(paths.cursor_log.clone());
                    session.cursor = Some(paths.cursor.clone());
                    Some(log)
                }
                Err(error) => {
                    emit(&Event::Warning {
                        message: format!("The cursor won't be recorded: {error}"),
                    });
                    None
                }
            }
        } else {
            None
        };
        session
            .save(&paths.session)
            .map_err(|error| format!("{} can't be written: {error}", paths.session.display()))?;

        Ok(Self {
            size,
            clock: Mutex::new(RecordingClock::new(config.start_paused)),
            video: Mutex::new(Some(video)),
            last_frame: Mutex::new(None),
            system: Mutex::new(system),
            microphone: Mutex::new(microphone),
            webcam: Mutex::new(WebcamTrack {
                recorder: None,
                first: None,
            }),
            cursor: Mutex::new(cursor),
            session: Mutex::new(session),
            started: AtomicBool::new(false),
            fail_soft: AtomicBool::new(false),
            problem: Mutex::new(None),
            emit,
            paths,
        })
    }

    fn fail(&self, message: String) {
        let mut problem = lock(&self.problem);
        if problem.is_none() {
            *problem = Some(message);
        }
    }

    /// What's gone wrong, if something has ended the recording.
    #[must_use]
    pub fn problem(&self) -> Option<String> {
        lock(&self.problem).clone()
    }

    /// The capture source has ended by itself, such as when a display is unplugged. What's been
    /// recorded so far still gets finished.
    pub fn source_ended(&self, message: String) {
        self.warn(message);
        self.fail_soft.store(true, Ordering::Release);
    }

    /// Whether the source has ended by itself.
    #[must_use]
    pub fn source_has_ended(&self) -> bool {
        self.fail_soft.load(Ordering::Acquire)
    }

    /// Reports a problem that doesn't stop the recording.
    pub fn warn(&self, message: String) {
        (self.emit)(&Event::Warning { message });
    }

    /// Takes a frame of the screen, captured at a host time.
    pub fn on_frame(&self, buffer: &CVPixelBuffer, host: Micros) {
        if !self.started.swap(true, Ordering::AcqRel) {
            (self.emit)(&Event::Started {
                width: self.size.0,
                height: self.size.1,
            });
        }
        let Some(time) = lock(&self.clock).place_frame(host.0) else {
            return;
        };
        let mut video = lock(&self.video);
        let Some(recorder) = video.as_mut() else {
            return;
        };
        match recorder.append(buffer, time) {
            Ok(true) => {
                *lock(&self.last_frame) = Some(Held(buffer.into()));
            }
            Ok(false) => {}
            Err(error) => self.fail(error.0),
        }
    }

    /// Takes a buffer of sound, captured at a host time.
    pub fn on_sound(&self, track: Track, sample: &CMSampleBuffer, host: Micros) {
        let Some(time) = lock(&self.clock).place(host.0) else {
            return;
        };
        let slot = match track {
            Track::System => &self.system,
            Track::Microphone => &self.microphone,
        };
        let mut slot = lock(slot);
        let Some(recorder) = slot.as_mut() else {
            return;
        };
        if let Err(error) = recorder.append(sample, time) {
            // Losing a sound track shouldn't lose the recording: stop writing to it and carry on.
            self.warn(format!("Sound stopped recording: {error}"));
            *slot = None;
        }
    }

    /// Opens the webcam's file, once it's known that the camera works.
    pub fn open_webcam(&self) {
        let opened = VideoRecorder::create(
            &self.paths.webcam,
            RecordSettings {
                width: 1280,
                height: 720,
                fps: WEBCAM_FPS,
                bitrate: WEBCAM_BITRATE,
                keyframe_interval: WEBCAM_FPS,
            },
        );
        match opened {
            Ok(recorder) => {
                lock(&self.webcam).recorder = Some(recorder);
                let mut session = lock(&self.session);
                session.webcam = Some(self.paths.webcam.clone());
                let _ = session.save(&self.paths.session);
            }
            Err(error) => self.warn(format!("The webcam won't be recorded: {error}")),
        }
    }

    /// Takes a webcam frame, captured at a host time. Its file starts from its own first frame, and
    /// the gap between that and the recording's start is kept as the sync offset (§27).
    pub fn on_webcam_frame(&self, buffer: &CVPixelBuffer, host: Micros) {
        let Some(time) = lock(&self.clock).place(host.0) else {
            return;
        };
        let mut webcam = lock(&self.webcam);
        let first = if let Some(first) = webcam.first {
            first
        } else {
            webcam.first = Some(time);
            let mut session = lock(&self.session);
            session.webcam_offset_us = time.0;
            let _ = session.save(&self.paths.session);
            time
        };
        let Some(recorder) = webcam.recorder.as_mut() else {
            return;
        };
        if let Err(error) = recorder.append(buffer, Micros(time.0 - first.0)) {
            self.warn(format!("The webcam stopped recording: {error}"));
            webcam.recorder = None;
        }
    }

    /// Takes a cursor position, as fractions of the source.
    pub fn on_cursor(
        &self,
        x: f64,
        y: f64,
        interaction: Interaction,
        shape: CursorShape,
        host: Micros,
    ) {
        let Some(time) = lock(&self.clock).place(host.0) else {
            return;
        };
        let mut cursor = lock(&self.cursor);
        let Some(log) = cursor.as_mut() else { return };
        let sample = Sample {
            time_ms: time.ms(),
            cx: x,
            cy: y,
            interaction,
            shape,
        };
        if let Err(error) = log.push(&sample) {
            self.warn(format!("The cursor stopped recording: {error}"));
            *cursor = None;
        }
    }

    /// Where the recording is on its own timeline, for classifying clicks.
    #[must_use]
    pub fn place(&self, host: Micros) -> Option<Micros> {
        lock(&self.clock).place(host.0)
    }

    pub fn pause(&self) {
        lock(&self.clock).pause(host_now().0);
        if let Some(log) = lock(&self.cursor).as_mut() {
            let _ = log.flush();
        }
        (self.emit)(&Event::Paused);
    }

    pub fn resume(&self) {
        lock(&self.clock).resume(host_now().0);
        (self.emit)(&Event::Resumed);
    }

    /// Finishes every file and checks the result.
    ///
    /// # Errors
    /// Returns an error if nothing playable came of it. Sound and webcam problems only warn.
    pub fn finish(&self) -> Result<Finished, String> {
        let length = lock(&self.clock).length_at(host_now().0);
        let held = lock(&self.last_frame).take();
        let mut video = lock(&self.video)
            .take()
            .ok_or_else(|| "The recording has already been finished.".to_string())?;
        if video.last_time().is_none() {
            video.cancel();
            let _ = std::fs::remove_file(&self.paths.session);
            if let Some(log) = lock(&self.cursor).take() {
                drop(log);
                let _ = std::fs::remove_file(&self.paths.cursor_log);
            }
            return Err("The recording ended before any frames were kept.".to_string());
        }
        // Screens only send frames when something changes, so add one at the very end to make the
        // video as long as the recording was (§27).
        if let Some(Held(frame)) = &held
            && video
                .last_time()
                .is_some_and(|last| last.0 + 1000 < length.0)
        {
            let _ = video.append(frame, length);
        }
        let end = video.last_time().map_or(length, |last| last.max(length));
        let stats = video.stats();
        video.finish(end).map_err(|error| error.0)?;

        let mut session = lock(&self.session).clone();
        for (slot, path, name) in [
            (&self.system, &mut session.system_audio, "System audio"),
            (&self.microphone, &mut session.microphone, "The microphone"),
        ] {
            if let Some(recorder) = lock(slot).take() {
                match recorder.finish(end) {
                    Ok(true) => {}
                    Ok(false) => {
                        self.warn(format!("{name} recorded no sound."));
                        *path = None;
                    }
                    Err(error) => {
                        self.warn(format!("{name} couldn't be saved: {error}"));
                        *path = None;
                    }
                }
            } else if path.is_some() {
                // It was given up on part way. What it wrote before then may still be playable.
                self.warn(format!("{name} stopped recording part way through."));
            }
        }
        let webcam = lock(&self.webcam).recorder.take();
        if let Some(recorder) = webcam {
            let last = recorder.last_time();
            if last.is_some() {
                let webcam_end = last.map_or(Micros::ZERO, |last| last + Micros(33_333));
                if let Err(error) = recorder.finish(webcam_end) {
                    self.warn(format!("The webcam couldn't be saved: {error}"));
                    session.webcam = None;
                }
            } else {
                recorder.cancel();
                session.webcam = None;
                self.warn("The webcam recorded no frames.".to_string());
            }
        }
        if let Some(mut log) = lock(&self.cursor).take() {
            let _ = log.flush();
            drop(log);
            if let Err(error) = cursor_log::finalise(&self.paths.cursor_log, &self.paths.cursor) {
                self.warn(format!("The cursor data couldn't be saved: {error}"));
                session.cursor = None;
            }
            session.cursor_log = None;
        }
        session.state = SessionState::Finished;
        let _ = session.save(&self.paths.session);
        *lock(&self.session) = session.clone();

        let info = validate(&self.paths.video)?;
        Ok(Finished {
            duration: info.duration,
            frames: stats.frames,
            dropped: stats.dropped,
            session,
        })
    }
}

/// The checks a finished recording must pass (§6): at least 1,024 bytes, a video stream, a duration
/// above zero, and one frame that decodes.
///
/// # Errors
/// Returns which check failed.
pub fn validate(path: &Path) -> Result<VideoInfo, String> {
    let size = std::fs::metadata(path)
        .map_err(|error| format!("The recording can't be found: {error}"))?
        .len();
    if size < 1024 {
        return Err(format!("The recording is only {size} bytes long."));
    }
    let info = probe(path).map_err(|error| format!("The recording has no video: {error}"))?;
    if info.duration <= Micros::ZERO {
        return Err("The recording has no length.".to_string());
    }
    let mut decoder =
        Decoder::open(path).map_err(|error| format!("The recording can't be read: {error}"))?;
    match decoder.frame_at(reel_core::time::SourceTime(0)) {
        Ok(Some(_)) => Ok(info),
        Ok(None) => Err("The recording has no frames that decode.".to_string()),
        Err(error) => Err(format!("The recording's frames don't decode: {error}")),
    }
}

/// What was saved from a recording that got cut off.
#[derive(Clone, Debug, PartialEq)]
pub struct Recovered {
    pub session: RecordingSession,
    pub duration: Micros,
}

/// Saves what can be saved from a recording that got cut off (§59).
///
/// # Errors
/// Returns why not, when even the video isn't playable.
pub fn recover(session_path: &Path) -> Result<Recovered, String> {
    let mut session = RecordingSession::load(session_path)
        .map_err(|error| format!("{} can't be read: {error}", session_path.display()))?;
    let info = validate(&session.video)?;
    let paths = OutputPaths::for_video(&session.video);
    if paths.cursor_log.exists() {
        match cursor_log::finalise(&paths.cursor_log, &paths.cursor) {
            Ok(_) => session.cursor = Some(paths.cursor.clone()),
            Err(_) => session.cursor = None,
        }
    }
    session.cursor_log = None;
    // A track that can't be read would only cause trouble later, so leave it out.
    for track in [&mut session.system_audio, &mut session.microphone] {
        let readable = track
            .as_ref()
            .is_some_and(|path| matches!(reel_media::macos::read_pcm(path), Ok(Some(_))));
        if !readable {
            *track = None;
        }
    }
    if session
        .webcam
        .as_ref()
        .is_some_and(|path| validate(path).is_err())
    {
        session.webcam = None;
    }
    session.state = SessionState::Recovered;
    session
        .save(session_path)
        .map_err(|error| error.to_string())?;
    Ok(Recovered {
        session,
        duration: info.duration,
    })
}
