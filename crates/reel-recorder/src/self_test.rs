//! Recording for real: the screen, system audio, mic and webcam, as far as they're allowed.
//!
//! Everything downstream of capture is covered by the automatic tests, which use a test pattern. This
//! is what's left. It has to run on a person's machine, since macOS only gives these permissions to an
//! app a person has approved, so `scripts/run-record-test.sh` runs it from a small app bundle.

use std::path::Path;
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::mpsc;
use std::time::Duration;

use reel_capture::config::{DeviceChoice, Event, RecordConfig, SourceChoice};
use reel_capture::macos::permissions::{self, Access};
use reel_capture::macos::recorder::{Emit, validate};
use reel_capture::macos::run::{Command, record};
use reel_capture::macos::screen;
use reel_capture::session::RecordingSession;
use reel_media::macos::read_pcm;
use reel_motion::cursor::CursorTrack;
use serde_json::{Value, json};

struct Report {
    checks: Vec<Value>,
    passed: bool,
}

impl Report {
    fn check(&mut self, name: &str, pass: bool, detail: impl std::fmt::Display) {
        let detail = detail.to_string();
        println!("{} {name}: {detail}", if pass { "PASS" } else { "FAIL" });
        self.passed &= pass;
        self.checks
            .push(json!({ "name": name, "pass": pass, "detail": detail }));
    }

    /// Something that can't be tested here, rather than something that failed.
    fn note(&mut self, name: &str, detail: impl std::fmt::Display) {
        let detail = detail.to_string();
        println!("SKIP {name}: {detail}");
        self.checks
            .push(json!({ "name": name, "skipped": true, "detail": detail }));
    }

    fn finish(&self, folder: &Path) -> ExitCode {
        let text = json!({ "checks": self.checks, "passed": self.passed }).to_string();
        let _ = std::fs::write(folder.join("report.json"), text);
        if self.passed {
            ExitCode::SUCCESS
        } else {
            ExitCode::FAILURE
        }
    }
}

fn sound_seconds(path: &Path) -> Option<f64> {
    let pcm = read_pcm(path).ok()??;
    Some(pcm.samples.len() as f64 / 2.0 / 48_000.0)
}

pub fn run(folder: &Path, ask: bool) -> ExitCode {
    let _ = std::fs::create_dir_all(folder);
    let mut report = Report {
        checks: Vec::new(),
        passed: true,
    };

    if ask {
        let _ = permissions::request_screen_recording();
        if permissions::microphone() == Access::NotAsked {
            let _ = permissions::request_microphone();
        }
        if permissions::camera() == Access::NotAsked {
            let _ = permissions::request_camera();
        }
    }
    let (screen_access, mic, camera) = (
        permissions::screen_recording(),
        permissions::microphone(),
        permissions::camera(),
    );
    report.note(
        "permissions",
        format!("screen {screen_access:?}, microphone {mic:?}, camera {camera:?}"),
    );
    if screen_access != Access::Granted {
        report.check(
            "the screen can be recorded",
            false,
            "Allow this app in System Settings → Privacy & Security → Screen Recording, then run this again.",
        );
        return report.finish(folder);
    }

    let thumbnails = folder.join("thumbnails");
    let listed = screen::list(Some(&thumbnails));
    let (displays, windows) = match listed {
        Ok(found) => found,
        Err(error) => {
            report.check("what's on screen can be listed", false, error);
            return report.finish(folder);
        }
    };
    let with_thumbnails = displays
        .iter()
        .filter(|item| item.thumbnail.is_some())
        .count();
    report.check(
        "displays are listed with thumbnails",
        !displays.is_empty() && with_thumbnails == displays.len(),
        format!(
            "{} displays, {with_thumbnails} with thumbnails",
            displays.len()
        ),
    );
    report.check(
        "windows are listed, filtered and named",
        windows.iter().all(|window| !window.name.is_empty()),
        format!(
            "{} windows: {}",
            windows.len(),
            windows
                .iter()
                .take(4)
                .map(|window| window.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ),
    );
    let Some(display) = displays
        .iter()
        .find(|display| display.primary)
        .or(displays.first())
    else {
        return report.finish(folder);
    };

    let output = folder.join("recording-self-test.mp4");
    let config = RecordConfig {
        source: SourceChoice::Display { id: display.id },
        output: output.clone(),
        fps: 60,
        system_audio: true,
        microphone: (mic == Access::Granted).then(DeviceChoice::default),
        webcam: (camera == Access::Granted).then(DeviceChoice::default),
        cursor: true,
        clicks: true,
        start_paused: false,
        exclude_bundles: Vec::new(),
        hide_own_windows: true,
    };
    let (send, receive) = mpsc::channel();
    std::thread::spawn(move || {
        let wait = |seconds| std::thread::sleep(Duration::from_secs(seconds));
        wait(4);
        let _ = send.send(Command::Pause);
        wait(2);
        let _ = send.send(Command::Resume);
        wait(4);
        let _ = send.send(Command::Pause);
        wait(2);
        let _ = send.send(Command::Resume);
        wait(4);
        let _ = send.send(Command::Stop);
    });
    let events = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let log = events.clone();
    let emit: Emit = Arc::new(move |event: &Event| {
        if let Ok(line) = serde_json::to_string(event) {
            println!("{line}");
            if let Ok(mut lines) = log.lock() {
                lines.push(line);
            }
        }
    });

    let finished = match record(&config, &receive, emit) {
        Ok(finished) => finished,
        Err(error) => {
            report.check("a recording finishes", false, error);
            return report.finish(folder);
        }
    };
    report.check(
        "a recording finishes",
        true,
        format!("{:.1} s", finished.duration.secs()),
    );
    let warnings = events.lock().map_or(0, |lines| {
        lines.iter().filter(|line| line.contains("warning")).count()
    });
    report.note("warnings", warnings);

    // 12 seconds recorded, around 4 paused.
    let seconds = finished.duration.secs();
    report.check(
        "paused time is left out",
        (seconds - 12.0).abs() < 1.0,
        format!("{seconds:.2} s of about 12"),
    );
    match validate(&output) {
        Ok(info) => {
            let expected = (display.width * display.scale) as u32 & !1;
            report.check(
                "the video is at full resolution",
                info.width == expected,
                format!(
                    "{}×{}, expected a width of {expected}",
                    info.width, info.height
                ),
            );
        }
        Err(error) => report.check("the video passes its checks", false, error),
    }
    report.check(
        "frames aren't dropped",
        finished.dropped == 0,
        format!("{} of {}", finished.dropped, finished.frames),
    );

    let session = &finished.session;
    match &session.system_audio {
        Some(path) => report.check(
            "system audio is its own track",
            sound_seconds(path).is_some_and(|length| length > 1.0),
            format!("{:?} s", sound_seconds(path)),
        ),
        None => report.note(
            "system audio",
            "None arrived. Play something during the test to check it.",
        ),
    }
    if mic == Access::Granted {
        let length = session.microphone.as_deref().and_then(sound_seconds);
        report.check(
            "the mic is its own track, as long as the video",
            length.is_some_and(|length| (length - seconds).abs() < 1.0),
            format!("{length:?} s against {seconds:.2} s"),
        );
    } else {
        report.note("microphone", "Not allowed, so not tested.");
    }
    if camera == Access::Granted {
        match &session.webcam {
            Some(path) => match validate(path) {
                Ok(info) => report.check(
                    "the webcam has its own file and a sync offset",
                    info.duration.secs() > 1.0,
                    format!(
                        "{}×{}, {:.1} s, offset {} ms",
                        info.width,
                        info.height,
                        info.duration.secs(),
                        session.webcam_offset_us / 1000
                    ),
                ),
                Err(error) => report.check("the webcam has its own file", false, error),
            },
            None => report.check("the webcam has its own file", false, "No file was made."),
        }
    } else {
        report.note("webcam", "Not allowed, so not tested.");
    }
    let cursor = session
        .cursor
        .as_ref()
        .and_then(|path| std::fs::read_to_string(path).ok());
    let track = cursor.and_then(|text| CursorTrack::parse(&text).ok());
    report.check(
        "cursor data is recorded",
        track
            .as_ref()
            .is_some_and(|track| track.samples.len() > 200),
        format!("{} samples", track.map_or(0, |track| track.samples.len())),
    );
    let reloaded = RecordingSession::load(&folder.join("recording-self-test.mp4.session.json"));
    report.check("the session note says finished", reloaded.is_ok(), "");
    report.finish(folder)
}
