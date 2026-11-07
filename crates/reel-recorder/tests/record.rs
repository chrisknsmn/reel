//! The real recorder process, end to end, using its test pattern so that no permissions are needed.
//!
//! The pattern flashes and beeps once a second on one clock. So a flash and a beep that come apart in
//! the finished files measure drift directly, and a pause that isn't cut cleanly from every track shows
//! up the same way.
//!
//! These need a hardware encoder and take real time, so they're skipped on CI, where `CI` is set.
#![cfg(target_os = "macos")]

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

use reel_capture::session::{RecordingSession, SessionState};
use reel_core::time::SourceTime;
use reel_media::macos::record::average_luma;
use reel_media::macos::{Decoder, probe, read_pcm};
use reel_motion::cursor::CursorTrack;
use serde_json::{Value, json};

/// The most that a flash and its beep may differ by. It's the target in the plan's exit criteria.
const MAX_DRIFT_MS: f64 = 40.0;
/// One frame at the rate used here, which is as finely as a flash can be located.
const FRAME_MS: f64 = 1000.0 / 30.0;

fn on_ci() -> bool {
    std::env::var_os("CI").is_some()
}

/// These run in real time, so they take turns. Run together, they starve each other and frames arrive
/// late, which measures the machine rather than the recorder.
static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());

fn take_turn() -> MutexGuard<'static, ()> {
    ONE_AT_A_TIME
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

struct Run {
    child: Child,
    stdin: Option<ChildStdin>,
    events: Receiver<Value>,
    output: PathBuf,
}

fn start(folder: &Path, name: &str, extra: &Value) -> Run {
    let output = folder.join(name);
    let mut config = json!({
        "source": { "kind": "synthetic", "width": 640, "height": 360 },
        "output": output,
        "fps": 30,
        "system_audio": true,
        "microphone": {},
        "clicks": false,
    });
    for (key, value) in extra.as_object().unwrap() {
        config[key] = value.clone();
    }
    let mut child = Command::new(env!("CARGO_BIN_EXE_reel-recorder"))
        .arg("record")
        .arg(config.to_string())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    let stdin = child.stdin.take();
    let stdout = child.stdout.take().unwrap();
    let (send, events) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            let Ok(line) = line else { break };
            let Ok(event) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            if send.send(event).is_err() {
                break;
            }
        }
    });
    Run {
        child,
        stdin,
        events,
        output,
    }
}

impl Run {
    fn send(&mut self, command: &str) {
        let stdin = self.stdin.as_mut().unwrap();
        writeln!(stdin, "{command}").unwrap();
        stdin.flush().unwrap();
    }

    fn wait_for(&self, name: &str) -> Value {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            let event = self
                .events
                .recv_timeout(left)
                .unwrap_or_else(|_| panic!("waited too long for {name}"));
            if event["event"] == name {
                return event;
            }
            assert_ne!(
                event["event"], "failed",
                "while waiting for {name}: {event}"
            );
        }
    }

    fn stop(&mut self) -> Value {
        self.send("stop");
        let stopped = self.wait_for("stopped");
        assert!(self.child.wait().unwrap().success());
        stopped
    }
}

fn sleep(ms: u64) {
    std::thread::sleep(Duration::from_millis(ms));
}

/// When each flash starts in a video, in milliseconds.
fn flashes(path: &Path) -> Vec<f64> {
    let info = probe(path).unwrap();
    let mut decoder = Decoder::open(path).unwrap();
    let mut found = Vec::new();
    let mut lit = false;
    let mut last_pts = None;
    let mut ms = 0.0;
    while ms * 1000.0 < info.duration.0 as f64 {
        if let Ok(Some(frame)) = decoder.frame_at(SourceTime((ms * 1000.0) as i64))
            && last_pts != Some(frame.pts)
        {
            last_pts = Some(frame.pts);
            let bright = average_luma(&frame.buffer).unwrap() > 128.0;
            if bright && !lit {
                found.push(frame.pts.0 as f64 / 1000.0);
            }
            lit = bright;
        }
        ms += FRAME_MS / 2.0;
    }
    found
}

/// When each beep starts in a sound file, in milliseconds.
fn beeps(path: &Path) -> Vec<f64> {
    let pcm = read_pcm(path).unwrap().expect("the file has sound");
    let samples = &pcm.samples;
    let mut found = Vec::new();
    let mut sounding = false;
    let mut quiet_for = 0;
    for (index, frame) in samples.chunks_exact(2).enumerate() {
        let loud = i32::from(frame[0]).abs() > 3000;
        if loud && !sounding {
            found.push(index as f64 / 48.0);
            sounding = true;
        }
        quiet_for = if loud { 0 } else { quiet_for + 1 };
        // A sine wave passes through zero all the time, so only a real stretch of quiet ends a beep.
        if quiet_for > 480 {
            sounding = false;
        }
    }
    found
}

fn nearest(values: &[f64], to: f64) -> f64 {
    values
        .iter()
        .copied()
        .min_by(|a, b| (a - to).abs().total_cmp(&(b - to).abs()))
        .unwrap_or(f64::INFINITY)
}

/// The worst gap between a flash and the nearest beep, which should be the one that goes with it.
fn worst_drift(flash_times: &[f64], beep_times: &[f64]) -> f64 {
    let worst = flash_times
        .iter()
        .map(|flash| (nearest(beep_times, *flash) - flash).abs())
        .fold(0.0, f64::max);
    println!("{} flashes, worst drift {worst:.1} ms", flash_times.len());
    worst
}

#[test]
fn a_clean_recording_keeps_sound_and_picture_together() {
    if on_ci() {
        return;
    }
    let _turn = take_turn();
    let folder = tempfile::tempdir().unwrap();
    let mut run = start(folder.path(), "a.mp4", &json!({}));
    run.wait_for("started");
    sleep(6000);
    let stopped = run.stop();
    assert_eq!(stopped["dropped_frames"], 0, "{stopped}");

    let flash_times = flashes(&run.output);
    assert!(
        flash_times.len() >= 5,
        "only {} flashes in {flash_times:?}",
        flash_times.len()
    );
    for sound in [".system.m4a", ".mic.m4a"] {
        let path = PathBuf::from(format!("{}{sound}", run.output.display()));
        let beep_times = beeps(&path);
        assert!(beep_times.len() >= 5, "{sound}: {beep_times:?}");
        let drift = worst_drift(&flash_times, &beep_times);
        assert!(
            drift < MAX_DRIFT_MS,
            "{sound} is off by {drift} ms\nflashes {flash_times:?}\nbeeps {beep_times:?}"
        );
    }
}

#[test]
fn pausing_twice_cuts_the_same_time_out_of_everything() {
    if on_ci() {
        return;
    }
    let _turn = take_turn();
    let folder = tempfile::tempdir().unwrap();
    let mut run = start(folder.path(), "a.mp4", &json!({}));
    run.wait_for("started");
    sleep(2300);
    run.send("pause");
    run.wait_for("paused");
    sleep(1500);
    run.send("resume");
    run.wait_for("resumed");
    sleep(2200);
    run.send("pause");
    run.wait_for("paused");
    sleep(1300);
    run.send("resume");
    run.wait_for("resumed");
    sleep(2000);
    let stopped = run.stop();

    // 6.5 s recorded and 2.8 s paused. Allow for how long commands take to get there.
    let duration = stopped["duration_ms"].as_f64().unwrap();
    assert!(
        (duration - 6500.0).abs() < 400.0,
        "the video is {duration} ms long"
    );

    let flash_times = flashes(&run.output);
    let path = PathBuf::from(format!("{}.system.m4a", run.output.display()));
    let beep_times = beeps(&path);
    let drift = worst_drift(&flash_times, &beep_times);
    assert!(
        drift < MAX_DRIFT_MS,
        "off by {drift} ms\nflashes {flash_times:?}\nbeeps {beep_times:?}"
    );

    // The sound file is as long as the video, so nothing paused got left in.
    let pcm = read_pcm(&path).unwrap().unwrap();
    let sound_ms = pcm.samples.len() as f64 / 2.0 / 48.0;
    assert!(
        (sound_ms - duration).abs() < 200.0,
        "{sound_ms} ms of sound against {duration} ms of video"
    );
}

#[test]
fn a_track_that_starts_late_stays_on_time() {
    if on_ci() {
        return;
    }
    let _turn = take_turn();
    let folder = tempfile::tempdir().unwrap();
    let mut run = start(
        folder.path(),
        "a.mp4",
        &json!({ "source": { "kind": "synthetic", "width": 640, "height": 360, "mic_delay_ms": 1400 } }),
    );
    run.wait_for("started");
    sleep(5000);
    run.stop();
    let flash_times = flashes(&run.output);
    let mic = PathBuf::from(format!("{}.mic.m4a", run.output.display()));
    let beep_times = beeps(&mic);
    assert!(
        beep_times.first().is_some_and(|first| *first > 1000.0),
        "the mic starts late: {beep_times:?}"
    );
    // Only the flashes that have a beep to go with them.
    let later: Vec<f64> = flash_times
        .iter()
        .copied()
        .filter(|time| *time > 1500.0)
        .collect();
    let drift = worst_drift(&later, &beep_times);
    assert!(
        drift < MAX_DRIFT_MS,
        "off by {drift} ms\nflashes {later:?}\nbeeps {beep_times:?}"
    );
}

#[test]
fn killing_the_recorder_still_leaves_a_playable_recording_that_can_be_recovered() {
    if on_ci() {
        return;
    }
    let _turn = take_turn();
    let folder = tempfile::tempdir().unwrap();
    let mut run = start(folder.path(), "a.mp4", &json!({}));
    run.wait_for("started");
    sleep(4500);
    run.child.kill().unwrap();
    run.child.wait().unwrap();

    let output = &run.output;
    let info = probe(output).expect("the video should open after a crash");
    assert!(
        info.duration.ms() > 2500.0,
        "only {} ms survived",
        info.duration.ms()
    );
    let flash_times = flashes(output);
    assert!(flash_times.len() >= 2, "{flash_times:?}");

    let binary = env!("CARGO_BIN_EXE_reel-recorder");
    let listing = Command::new(binary)
        .args(["interrupted"])
        .arg(folder.path())
        .output()
        .unwrap();
    let listing: Value = serde_json::from_slice(&listing.stdout).unwrap();
    let interrupted = listing["interrupted"].as_array().unwrap();
    assert_eq!(interrupted.len(), 1, "{listing}");
    let session_path = PathBuf::from(interrupted[0]["session"].as_str().unwrap());

    let saved = Command::new(binary)
        .arg("recover")
        .arg(&session_path)
        .output()
        .unwrap();
    assert!(
        saved.status.success(),
        "{}",
        String::from_utf8_lossy(&saved.stdout)
    );
    let saved: Value = serde_json::from_slice(&saved.stdout).unwrap();
    assert!(saved["duration_ms"].as_f64().unwrap() > 2500.0, "{saved}");

    let session = RecordingSession::load(&session_path).unwrap();
    assert_eq!(session.state, SessionState::Recovered);
    let cursor = session.cursor.as_ref().expect("the cursor data is saved");
    let track = CursorTrack::parse(&std::fs::read_to_string(cursor).unwrap()).unwrap();
    assert!(
        track.samples.len() > 50,
        "{} cursor samples",
        track.samples.len()
    );

    // Sound written before the crash survives too, and stays in step with the picture.
    let system = session
        .system_audio
        .as_ref()
        .expect("the system audio is saved");
    let drift = worst_drift(&flash_times, &beeps(system));
    assert!(drift < MAX_DRIFT_MS, "off by {drift} ms");

    // And it isn't listed as interrupted any more.
    let listing = Command::new(binary)
        .args(["interrupted"])
        .arg(folder.path())
        .output()
        .unwrap();
    let listing: Value = serde_json::from_slice(&listing.stdout).unwrap();
    assert_eq!(listing["interrupted"].as_array().unwrap().len(), 0);
}

#[test]
fn the_app_going_away_finishes_the_recording_properly() {
    if on_ci() {
        return;
    }
    let _turn = take_turn();
    let folder = tempfile::tempdir().unwrap();
    let mut run = start(folder.path(), "a.mp4", &json!({}));
    run.wait_for("started");
    sleep(2000);
    // This is what the recorder sees if the app crashes: its input closes.
    drop(run.stdin.take());
    run.wait_for("stopped");
    assert!(run.child.wait().unwrap().success());
    let session = RecordingSession::load(&PathBuf::from(format!(
        "{}.session.json",
        run.output.display()
    )))
    .unwrap();
    assert_eq!(session.state, SessionState::Finished);
    assert!(probe(&run.output).unwrap().duration.ms() > 1500.0);
}

#[test]
fn a_warm_start_keeps_nothing_until_it_resumes() {
    if on_ci() {
        return;
    }
    let _turn = take_turn();
    let folder = tempfile::tempdir().unwrap();
    let mut run = start(folder.path(), "a.mp4", &json!({ "start_paused": true }));
    // The first frame has arrived, so the countdown can run with capture already warmed up.
    run.wait_for("started");
    sleep(2000);
    run.send("resume");
    run.wait_for("resumed");
    sleep(2000);
    let stopped = run.stop();
    let duration = stopped["duration_ms"].as_f64().unwrap();
    assert!(
        (duration - 2000.0).abs() < 300.0,
        "{duration} ms: the countdown shouldn't be in the video"
    );
}

#[test]
fn stopping_before_anything_was_kept_reports_it_and_leaves_nothing() {
    if on_ci() {
        return;
    }
    let _turn = take_turn();
    let folder = tempfile::tempdir().unwrap();
    let mut run = start(folder.path(), "a.mp4", &json!({ "start_paused": true }));
    run.wait_for("started");
    run.send("stop");
    let failed = run.wait_for("failed");
    assert!(
        failed["message"]
            .as_str()
            .unwrap()
            .contains("before any frames"),
        "{failed}"
    );
    assert!(!run.child.wait().unwrap().success());
    assert!(!run.output.exists());
}

#[test]
fn a_finished_recording_has_a_cursor_file_the_editor_can_read() {
    if on_ci() {
        return;
    }
    let _turn = take_turn();
    let folder = tempfile::tempdir().unwrap();
    let mut run = start(folder.path(), "a.mp4", &json!({}));
    run.wait_for("started");
    sleep(1500);
    run.stop();
    let cursor = PathBuf::from(format!("{}.cursor.json", run.output.display()));
    let track = CursorTrack::parse(&std::fs::read_to_string(&cursor).unwrap()).unwrap();
    // 33 ms apart, for about a second and a half.
    assert!(
        (30..60).contains(&track.samples.len()),
        "{} samples",
        track.samples.len()
    );
    let log = PathBuf::from(format!("{}.cursor.jsonl", run.output.display()));
    assert!(!log.exists(), "the log is cleaned up");
}

#[test]
fn every_standard_cursor_is_recognised_as_itself() {
    let output = Command::new(env!("CARGO_BIN_EXE_reel-recorder"))
        .arg("cursor-check")
        .output()
        .unwrap();
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["passed"], true, "{report}");
    assert!(report["known"].as_u64().unwrap() >= 9);
}

#[test]
fn a_recording_without_permission_says_so_rather_than_prompting() {
    if on_ci() {
        return;
    }
    let binary = env!("CARGO_BIN_EXE_reel-recorder");
    let permissions = Command::new(binary).arg("permissions").output().unwrap();
    let permissions: Value = serde_json::from_slice(&permissions.stdout).unwrap();
    if permissions["screen"] == "granted" {
        // Nothing to learn on a machine that has it.
        return;
    }
    let folder = tempfile::tempdir().unwrap();
    let output = folder.path().join("a.mp4");
    let config = json!({ "source": { "kind": "display", "id": 1 }, "output": output });
    let result = Command::new(binary)
        .arg("record")
        .arg(config.to_string())
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(!result.status.success());
    let text = String::from_utf8_lossy(&result.stdout);
    assert!(text.contains("Screen Recording permission"), "{text}");
    assert!(!output.exists());
}

/// The plan's exit criterion: ten minutes, paused twice, with drift under 40 ms. The test pattern's
/// tracks all run on one clock, so this checks the files and the writing, not real devices' clocks.
/// It takes eleven minutes, so it only runs when asked for: `cargo test -p reel-recorder -- --ignored`.
#[test]
#[ignore = "takes eleven minutes"]
fn ten_minutes_paused_twice_stays_in_step() {
    let _turn = take_turn();
    let folder = tempfile::tempdir().unwrap();
    let mut run = start(
        folder.path(),
        "a.mp4",
        &json!({ "source": { "kind": "synthetic", "width": 1920, "height": 1080 }, "fps": 60 }),
    );
    run.wait_for("started");
    sleep(200_000);
    run.send("pause");
    run.wait_for("paused");
    sleep(30_000);
    run.send("resume");
    run.wait_for("resumed");
    sleep(200_000);
    run.send("pause");
    run.wait_for("paused");
    sleep(30_000);
    run.send("resume");
    run.wait_for("resumed");
    sleep(200_000);
    let stopped = run.stop();
    println!("{stopped}");
    assert_eq!(stopped["dropped_frames"], 0, "{stopped}");
    let duration = stopped["duration_ms"].as_f64().unwrap();
    assert!((duration - 600_000.0).abs() < 2_000.0, "{duration} ms long");

    let flash_times = flashes(&run.output);
    assert!(flash_times.len() > 590, "{} flashes", flash_times.len());
    for sound in [".system.m4a", ".mic.m4a"] {
        let path = PathBuf::from(format!("{}{sound}", run.output.display()));
        let beep_times = beeps(&path);
        let drift = worst_drift(&flash_times, &beep_times);
        assert!(drift < MAX_DRIFT_MS, "{sound} is off by {drift} ms");
        // The last minute especially, which is where any slow drift would have built up.
        let tail: Vec<f64> = flash_times
            .iter()
            .copied()
            .filter(|time| *time > duration - 60_000.0)
            .collect();
        let tail_drift = worst_drift(&tail, &beep_times);
        assert!(
            tail_drift < MAX_DRIFT_MS,
            "{sound} ends up off by {tail_drift} ms"
        );
    }
}
