# T2: Decode test app

A disposable test app for T2 in `BUILD_PLAN.md`: hardware-decoding H.264 into wgpu textures, comparing AVFoundation with FFmpeg's library. The results are in `docs/spikes.md`.

It runs headless, on macOS only, and needs FFmpeg's development libraries (`brew install ffmpeg`).

```sh
cargo run --release -- <video> [report.json]
```

Clips for it should stay outside `~/Documents`, so no permission prompt interrupts a run. `~/Library/Caches/reel-spikes/t2-decode/` is a good place, and the results quoted in `docs/spikes.md` came from `testsrc2` clips at 3840x2160 and 60 fps.
