//! Real speech through the real model. It downloads a 75 MB model the first time, so it only runs when
//! asked for: `cargo test -p reel-captions --features whisper -- --ignored`.
#![cfg(all(target_os = "macos", feature = "whisper"))]

use std::path::{Path, PathBuf};
use std::process::Command;

use reel_captions::models::{TINY_EN, download, sha1_of};
use reel_captions::transcribe::transcribe;
use reel_media::macos::read_pcm;

fn model() -> PathBuf {
    let home = std::env::var_os("HOME").expect("a home folder");
    let path = TINY_EN.path_in(&Path::new(&home).join("Library/Caches/reel-spikes/models"));
    if !path.exists() {
        download(&TINY_EN, &path, |_, _| true).expect("the model should download");
    }
    path
}

/// Speech from macOS's own voice, with a long pause in the middle.
fn speech(folder: &Path) -> PathBuf {
    let spoken = folder.join("speech.aiff");
    let status = Command::new("say")
        .args(["-o"])
        .arg(&spoken)
        .arg("Hello there. This is a test of captions. [[slnc 2500]] And this is the second part.")
        .status()
        .unwrap();
    assert!(status.success());
    let converted = folder.join("speech.m4a");
    let status = Command::new("afconvert")
        .args([
            "-f",
            "m4af",
            "-d",
            "aac",
            "-c",
            "2",
            "--src-complexity",
            "bats",
            "-r",
            "127",
        ])
        .arg(&spoken)
        .arg(&converted)
        .status()
        .unwrap();
    assert!(status.success());
    converted
}

#[test]
#[ignore = "downloads a model"]
fn a_downloaded_model_matches_its_checksum() {
    assert_eq!(sha1_of(&model()).unwrap(), TINY_EN.sha1);
}

#[test]
#[ignore = "downloads a model"]
fn real_speech_becomes_timed_captions() {
    let folder = tempfile::tempdir().unwrap();
    let pcm = read_pcm(&speech(folder.path())).unwrap().expect("sound");
    let total_ms = pcm.samples.len() as f64 / 2.0 / 48.0;
    let last_seen = std::sync::Arc::new(std::sync::atomic::AtomicI32::new(0));
    let seen = last_seen.clone();
    let cues = transcribe(&model(), &pcm.samples, "en", move |percent| {
        seen.store(percent, std::sync::atomic::Ordering::Relaxed);
    })
    .unwrap();

    for cue in &cues {
        println!("{:7.0} to {:7.0}: {}", cue.start_ms, cue.end_ms, cue.text());
    }
    let text = cues
        .iter()
        .map(reel_captions::segment::Cue::text)
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();
    for word in ["hello", "test", "captions", "second", "part"] {
        assert!(text.contains(word), "{word} wasn't heard in: {text}");
    }
    // In order, inside the recording, and split by the pause.
    assert!(cues.len() >= 2, "{cues:?}");
    for pair in cues.windows(2) {
        assert!(pair[0].end_ms <= pair[1].start_ms, "{pair:?}");
    }
    assert!(cues.last().unwrap().end_ms <= total_ms + 500.0);
    let second = cues
        .iter()
        .find(|cue| cue.text().to_lowercase().contains("second"))
        .unwrap();
    let first = &cues[0];
    assert!(
        second.start_ms - first.start_ms > 2000.0,
        "the pause should put them apart: {cues:?}"
    );
    assert!(
        last_seen.load(std::sync::atomic::Ordering::Relaxed) > 0,
        "progress was reported"
    );
}
