//! `reel-cli convert`, run as a real program.

use std::path::Path;
use std::process::Command;

use reel_core::format;

fn cli() -> Command {
    Command::new(env!("CARGO_BIN_EXE_reel-cli"))
}

fn fixture() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../reel-core/tests/fixtures/baseline-1080p30.recordly")
}

#[test]
fn converts_a_project_next_to_the_original() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("a.recordly");
    std::fs::copy(fixture(), &input).unwrap();
    let original = std::fs::read(&input).unwrap();

    let result = cli().arg("convert").arg(&input).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );

    assert_eq!(
        std::fs::read(&input).unwrap(),
        original,
        "the original is untouched"
    );
    let text = std::fs::read_to_string(dir.path().join("a.reel")).unwrap();
    let project = format::from_str(&text).unwrap();
    assert_eq!(project.timeline.zooms.len(), 2);
}

#[test]
fn refuses_to_overwrite_without_force() {
    let dir = tempfile::tempdir().unwrap();
    let output = dir.path().join("out.reel");
    std::fs::write(&output, b"precious").unwrap();

    let result = cli()
        .arg("convert")
        .arg(fixture())
        .arg(&output)
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert_eq!(std::fs::read(&output).unwrap(), b"precious");

    let result = cli()
        .arg("convert")
        .arg(fixture())
        .arg(&output)
        .arg("--force")
        .output()
        .unwrap();
    assert!(result.status.success());
    assert!(format::from_str(&std::fs::read_to_string(&output).unwrap()).is_ok());
}

#[test]
fn says_what_is_wrong_with_a_bad_project() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("bad.recordly");
    std::fs::write(&input, b"{}").unwrap();
    let result = cli().arg("convert").arg(&input).output().unwrap();
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("isn't a Recordly project"));
}

#[test]
fn bad_arguments_show_how_to_use_it() {
    let result = cli().output().unwrap();
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("Usage"));
}
