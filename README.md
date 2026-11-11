# Reel

Reel is a screen recorder and editor for macOS, written in Rust. Record your screen with your cursor, mic, system audio and webcam, then polish it with zooms, a styled cursor, annotations and captions, and export an MP4 or GIF. Everything runs on your Mac, and nothing you record is uploaded.

**Status: not released.** It runs on macOS, but recording real screens hasn't been tested yet, and Windows and Linux aren't started. See [Status](#status).

## Build and run

You need macOS 14 or later, Rust 1.92, and CMake, which builds whisper.cpp for captions.

```sh
brew install cmake
cargo build --release -p reel-app -p reel-recorder
./target/release/reel-app                 # The launch bar.
./target/release/reel-app recording.mp4   # Straight into the editor, with a video or a .reel project.
```

Running it from a terminal means macOS asks for permissions on behalf of the terminal. To have them asked for by Reel itself, package it and open the app:

```sh
scripts/package-macos.sh
open target/package/Reel.app
```

The script makes a DMG too. It's signed ad hoc unless you give it a certificate, and it explains how at the top.

There's also a command-line tool, `reel-cli`, which renders projects without a window:

```sh
cargo run --release -p reel-cli -- render project.reel out.mp4
cargo run --release -p reel-cli -- frame project.reel --at-ms 1000 frame.png
```

## How it's put together

| Crate | What it does |
|---|---|
| `reel-core` | The project, timeline edits, undo, settings and files. No UI or GPU. |
| `reel-motion` | Zooms, cursor motion, layout and caption layout, as plain maths. |
| `reel-media` | Decoding, encoding and sound, through AVFoundation on macOS. |
| `reel-render` | The one wgpu compositor, used by both the preview and export. |
| `reel-capture` | Screen, mic, webcam and cursor capture. |
| `reel-recorder` | A separate recording process, so a crash in the app can't lose a recording. |
| `reel-captions` | Whisper, segmentation, and subtitle files. |
| `reel-cli` | Headless rendering. |
| `reel-app` | The app: Dioxus with [Rust/UI](https://rust-ui.com/) for controls, and wgpu showing through a hole in the page. |

`BUILD_PLAN.md` is the plan and its progress, and `docs/spikes.md` has what the early experiments found.

## Tests

```sh
cargo test --workspace                  # Everything automatic.
scripts/run-app-test.sh                 # Plays a project in the real app, and compares it with export.
REEL_TEST=editor scripts/run-app-test.sh    # Clicks through the whole editor.
scripts/run-bar-test.sh                 # Records a test pattern through the launch bar.
scripts/run-record-test.sh --ask        # Records your real screen. See below.
```

The scripts open a window that stays on top for up to a minute, and don't touch your mouse or keyboard.

`run-record-test.sh` is the one that needs you. macOS only gives Screen Recording, microphone and camera access to an app a person has approved, so it can't run unattended. With `--ask` it shows the prompts. Allow Screen Recording in System Settings, then run it again. It records about 16 seconds, and leaves the result for you to look at.

Two slower tests only run when asked for: a ten-minute recording (`cargo test --release -p reel-recorder -- --ignored`) and real speech through Whisper, which downloads a 75 MB model (`cargo test -p reel-captions --features whisper -- --ignored`).

## Status

**Works, and tested on macOS:**

- Editing: clips, reordering, zooms, annotations, audio, captions, undo, presets, autosave and shortcuts
- The preview, which matches export
- MP4 and GIF export, at 3.4× to 10.6× real time on an M4 Max
- Captions from real speech, run locally
- Everything in recording after capture itself, using a generated test pattern: pausing, keeping tracks in sync over ten minutes, surviving `kill -9`, and recovery

**Written but not tested yet, because it needs permissions a person has to grant:**

- Real screen and window capture, system audio, mic and webcam
- Click tracking, the mic meters, the floating webcam preview

**Not built:**

- Windows and Linux
- Translations (only English so far)
- Auto-update, which needs somewhere to publish releases
- Notarised releases, which need an Apple Developer certificate

`BUILD_PLAN.md` has the full list.

## Licence

AGPL-3.0-only. See `LICENSE`, and `THIRD-PARTY.md` for everything it builds on. The cursors are original art, and the gradients are Reel's own.
