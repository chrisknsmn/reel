//! The recorder's commands on macOS.

use std::io::{BufRead, Write};
use std::path::Path;
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::mpsc::{self, Sender};

use objc2::MainThreadMarker;
use objc2_app_kit::NSApplication;
use reel_capture::config::{Event, RecordConfig};
use reel_capture::macos::devices::{cameras, microphones, run_meter};
use reel_capture::macos::permissions;
use reel_capture::macos::pointer::{known_signatures, signature_of};
use reel_capture::macos::recorder::{Emit, recover, validate};
use reel_capture::macos::run::{Command, record};
use reel_capture::macos::screen;
use reel_capture::session::find_interrupted;
use serde_json::json;

fn say(event: &Event) {
    if let Ok(line) = serde_json::to_string(event) {
        let mut out = std::io::stdout().lock();
        let _ = writeln!(out, "{line}");
        let _ = out.flush();
    }
}

fn read_commands(send: Sender<Command>) {
    for line in std::io::stdin().lock().lines() {
        let Ok(line) = line else { break };
        let command = match line.trim() {
            "pause" => Command::Pause,
            "resume" => Command::Resume,
            "stop" => Command::Stop,
            _ => continue,
        };
        if send.send(command).is_err() {
            return;
        }
    }
    // Stdin closing means the app has gone. Finish the recording rather than lose it.
    let _ = send.send(Command::Stop);
}

fn run_record(config_text: &str) -> ExitCode {
    let config: RecordConfig = match serde_json::from_str(config_text) {
        Ok(config) => config,
        Err(error) => {
            say(&Event::Failed {
                message: format!("The recording settings can't be read: {error}"),
            });
            return ExitCode::from(2);
        }
    };
    let (send, receive) = mpsc::channel();
    std::thread::spawn(move || read_commands(send));
    let emit: Emit = Arc::new(say);
    match record(&config, &receive, emit) {
        Ok(finished) => {
            say(&Event::Stopped {
                output: config.output.clone(),
                duration_ms: finished.duration.ms(),
                frames: finished.frames,
                dropped_frames: finished.dropped,
            });
            ExitCode::SUCCESS
        }
        Err(message) => {
            say(&Event::Failed { message });
            ExitCode::FAILURE
        }
    }
}

fn print(value: &serde_json::Value) -> ExitCode {
    println!("{value}");
    ExitCode::SUCCESS
}

fn usage() -> ExitCode {
    eprintln!(
        "usage: reel-recorder <permissions|request|list|devices|record|interrupted|recover|validate|cursor-check>"
    );
    ExitCode::from(2)
}

/// Each standard cursor should be recognised as itself.
fn cursor_check() -> ExitCode {
    let known = known_signatures();
    let mut wrong = Vec::new();
    for (shape, signature) in &known {
        let found = reel_capture::cursor_shape::classify(signature, &known);
        if found != *shape {
            wrong.push(format!("{shape:?} read as {found:?}"));
        }
    }
    let current = objc2_app_kit::NSCursor::arrowCursor();
    let readable = signature_of(&current).is_some();
    let passed = !known.is_empty() && wrong.is_empty() && readable;
    let _ = print(&json!({ "known": known.len(), "wrong": wrong, "passed": passed }));
    if passed {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

pub fn main() -> ExitCode {
    // AppKit needs this before cursor images are filled in, and it has to be on the main thread. It
    // doesn't put anything on screen or in the Dock.
    if let Some(marker) = MainThreadMarker::new() {
        let _ = NSApplication::sharedApplication(marker);
    }
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let arguments: Vec<&str> = arguments.iter().map(String::as_str).collect();
    match arguments.as_slice() {
        ["permissions"] => print(&json!({
            "screen": permissions::screen_recording(),
            "microphone": permissions::microphone(),
            "camera": permissions::camera(),
        })),
        ["request", "screen"] => {
            print(&json!({ "granted": permissions::request_screen_recording() }))
        }
        ["request", "microphone"] => {
            print(&json!({ "granted": permissions::request_microphone() }))
        }
        ["request", "camera"] => print(&json!({ "granted": permissions::request_camera() })),
        ["list", rest @ ..] => {
            let thumbnails = match rest {
                ["--thumbnails", folder] => Some(Path::new(*folder)),
                [] => None,
                _ => return usage(),
            };
            match screen::list(thumbnails) {
                Ok((displays, windows)) => {
                    print(&json!({ "displays": displays, "windows": windows }))
                }
                Err(message) => {
                    say(&Event::Failed { message });
                    ExitCode::FAILURE
                }
            }
        }
        ["devices"] => print(&json!({ "microphones": microphones(), "cameras": cameras() })),
        ["record", config] => {
            // Either a file, or the settings themselves.
            let text = std::fs::read_to_string(config).unwrap_or_else(|_| (*config).to_string());
            run_record(&text)
        }
        ["interrupted", folder] => {
            let found: Vec<_> = find_interrupted(Path::new(folder))
                .into_iter()
                .map(|(path, session)| json!({ "session": path, "video": session.video }))
                .collect();
            print(&json!({ "interrupted": found }))
        }
        ["recover", session] => match recover(Path::new(session)) {
            Ok(saved) => print(&json!({
                "video": saved.session.video,
                "duration_ms": saved.duration.ms(),
                "system_audio": saved.session.system_audio,
                "microphone": saved.session.microphone,
                "webcam": saved.session.webcam,
                "cursor": saved.session.cursor,
            })),
            Err(message) => {
                say(&Event::Failed { message });
                ExitCode::FAILURE
            }
        },
        ["validate", video] => match validate(Path::new(video)) {
            Ok(info) => print(&json!({
                "width": info.width,
                "height": info.height,
                "fps": info.fps,
                "duration_ms": info.duration.ms(),
            })),
            Err(message) => {
                say(&Event::Failed { message });
                ExitCode::FAILURE
            }
        },
        ["cursor-check"] => cursor_check(),
        ["meter", device] => {
            // Stops when its input closes, which happens if the app goes away.
            let (send, receive) = mpsc::channel::<()>();
            std::thread::spawn(move || {
                for line in std::io::stdin().lock().lines() {
                    if line.is_err() {
                        break;
                    }
                }
                let _ = send.send(());
            });
            let result = run_meter(device, |level| {
                let mut out = std::io::stdout().lock();
                let _ = writeln!(out, "{level:.1}");
                let _ = out.flush();
                receive.try_recv().is_err()
            });
            match result {
                Ok(()) => ExitCode::SUCCESS,
                Err(message) => {
                    say(&Event::Failed { message });
                    ExitCode::FAILURE
                }
            }
        }
        ["self-test", folder] => crate::self_test::run(Path::new(folder), false),
        ["self-test", folder, "--ask"] => crate::self_test::run(Path::new(folder), true),
        _ => usage(),
    }
}
