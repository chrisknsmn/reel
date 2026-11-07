//! Separate recording process, so a UI crash can't lose a recording (§30).
//!
//! ```text
//! reel-recorder permissions                 What's allowed, without prompting.
//! reel-recorder request <screen|microphone|camera>  Show the system's prompt.
//! reel-recorder list [--thumbnails <folder>]   What can be recorded.
//! reel-recorder devices                     The microphones and cameras.
//! reel-recorder record <config.json>        Record. Takes pause, resume and stop on stdin.
//! reel-recorder interrupted <folder>        Recordings that got cut off.
//! reel-recorder recover <session.json>      Save what can be saved from one.
//! reel-recorder validate <video>            The checks a finished recording has to pass.
//! reel-recorder cursor-check                Whether cursor shapes can be told apart here.
//! reel-recorder self-test <folder> [--ask]  Record for real. Only `--ask` shows permission prompts.
//! ```

use std::process::ExitCode;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
mod self_test;

#[cfg(target_os = "macos")]
fn main() -> ExitCode {
    macos::main()
}

#[cfg(not(target_os = "macos"))]
fn main() -> ExitCode {
    eprintln!(
        "reel-recorder doesn't support this system yet (see Milestones 11 and 12 in BUILD_PLAN.md)"
    );
    ExitCode::FAILURE
}
