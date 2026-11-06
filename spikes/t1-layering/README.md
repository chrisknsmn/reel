# T1: Layering test app

A disposable test app for T1 in `BUILD_PLAN.md`: video in a wgpu surface, under a transparent Dioxus webview with real Rust/UI controls over it. The results are in `docs/spikes.md`.

It has its own Cargo workspace, so wgpu stays out of Reel's lockfile and `cargo deny` until it's adopted. It compiles the vendored components straight from `crates/reel-app`, and its lockfile started as a copy of Reel's, so it tests the same Dioxus, wry and tao versions.

## On macOS

You need `ffmpeg` on your `PATH`.

```sh
./run.sh        # The self-test. A window stays on top for about a minute.
./run.sh hand   # Open the app to try by hand.
```

Self-test results land in `out/`: `report.json`, `run.log` and window captures. Your mouse and keyboard aren't touched, but don't click elsewhere during a run. If the window loses focus, the report flags its clicks as inconclusive.

### Hand check

The self-test synthesises its clicks and drags, so these still need a person. Run `./run.sh hand`, then:

1. Click **Open popover** and **Open dialog** with the mouse. Both should open over the video, with the video still playing.
2. Click on bare video. The popover should close.
3. Drag the window's corner around, quickly and slowly. Is the grey band on the moving edges acceptable?
4. Watch the video for stutter while doing all of this.

The readout at the top right shows the frame rate, the worst recent frame time and any dropped frames. The lime and magenta lines are deliberate: they mark the two layers' edges so a capture can compare them.

## On Windows and Linux

The self-test is macOS-only, and the app hasn't been compiled on either OS yet. To try it by hand:

```sh
ffmpeg -f lavfi -i testsrc2=size=1920x1080:rate=60:duration=30 -pix_fmt yuv420p 1080p60.mp4
```

Then set the `T1_VIDEO` environment variable to that file's path, and run `cargo run --release`.

On Linux, expect it not to layer at all until the problem described in `docs/spikes.md` is dealt with.

## Settings

Environment variables, which `run.sh` forwards:

| Variable | Effect |
|---|---|
| `T1_GRAVITY=resize` | Keeps Core Animation's default of stretching frames, for comparison |
| `T1_LATENCY=2` | Lets two frames queue up rather than one |
| `T1_ONLY_RESIZE=1` | Runs only the resize checks, which takes about 25 seconds |

## Cleaning up

The build takes about 2 GB in `target/`, and the app and test video are in `~/Library/Caches/reel-spikes/`. Delete both when T1 is finished with.
