//! Separate recording process, so a UI crash can't lose a recording.

use std::process::ExitCode;

fn main() -> ExitCode {
    eprintln!("reel-recorder is not implemented yet (see Milestone 4 in BUILD_PLAN.md)");
    ExitCode::FAILURE
}
