//! `reel-cli`, run as a real program.

use std::process::Command;

#[test]
fn bad_arguments_show_how_to_use_it() {
    let result = Command::new(env!("CARGO_BIN_EXE_reel-cli"))
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("Usage"));
}
