//! Milestone 2's exit criterion: real Recordly projects convert to `.reel` and round-trip unchanged.

use std::path::{Path, PathBuf};

use reel_core::format;
use reel_core::project::Project;
use reel_core::recordly::{self, ImportOptions};
use reel_core::time::{SourceTime, TimelineTime};

fn fixture(name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name);
    std::fs::read_to_string(path).expect("fixture is missing")
}

fn convert(text: &str) -> Project {
    let imported =
        recordly::import(text, ImportOptions::default()).expect("a real project should import");
    imported
        .project
        .timeline
        .check()
        .expect("the timeline should be valid");
    imported.project
}

fn assert_round_trips(project: &Project) {
    let text = format::to_string(project);
    let loaded = format::from_str(&text).expect("a converted project should load");
    assert_eq!(&loaded, project, "loading changed something");
    assert_eq!(
        format::to_string(&loaded),
        text,
        "saving again changed the file"
    );
}

#[test]
fn every_fixture_converts_and_round_trips() {
    for name in [
        "baseline-1080p30.recordly",
        "baseline-4k-webcam.recordly",
        "recorded-on-a-mac.recordly",
    ] {
        assert_round_trips(&convert(&fixture(name)));
    }
}

#[test]
fn the_baseline_project_keeps_what_it_said() {
    let project = convert(&fixture("baseline-1080p30.recordly"));
    assert_eq!(project.id, "4c1afc5b-7f43-42b6-8eeb-e8c6baaa896f");
    assert!(project.media.video.absolute.ends_with("1080p30.mp4"));

    assert_eq!(project.look.shadow, 0.6);
    assert_eq!(project.look.corner_radius, 12.0);
    assert_eq!(project.look.padding.top, 10.0);

    let timeline = &project.timeline;
    assert_eq!(timeline.clips.len(), 1);
    assert_eq!(timeline.clips[0].end(), TimelineTime::from_ms(30_000.0));
    assert_eq!(
        timeline.to_source(TimelineTime::from_ms(12_345.0)),
        Some(SourceTime::from_ms(12_345.0))
    );

    assert_eq!(timeline.zooms.len(), 2);
    assert_eq!(timeline.zooms[0].start, TimelineTime::from_ms(5000.0));
    assert_eq!(timeline.zooms[0].end, TimelineTime::from_ms(11_000.0));
    assert_eq!(timeline.zooms[0].scale(), 1.5);
    assert_eq!(timeline.zooms[1].scale(), 1.8);
    assert_eq!(timeline.zooms[1].focus.y, 0.7);
}

#[test]
fn the_webcam_project_keeps_its_webcam() {
    let project = convert(&fixture("baseline-4k-webcam.recordly"));
    assert!(project.webcam.enabled);
    assert!(project.webcam.source.is_some());
}

/// Any real projects on this machine, which CI won't have.
fn local_projects() -> Vec<PathBuf> {
    let mut folders = vec![Path::new(env!("CARGO_MANIFEST_DIR")).join("../../baseline/projects")];
    if let Some(home) = std::env::var_os("HOME") {
        folders.push(
            PathBuf::from(home).join("Library/Application Support/Recordly/recordings/Projects"),
        );
    }
    folders
        .iter()
        .filter_map(|folder| std::fs::read_dir(folder).ok())
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "recordly"))
        .collect()
}

#[test]
fn real_projects_on_this_machine_convert_and_round_trip() {
    for path in local_projects() {
        let text = std::fs::read_to_string(&path).expect("couldn't read a local project");
        assert_round_trips(&convert(&text));
    }
}
