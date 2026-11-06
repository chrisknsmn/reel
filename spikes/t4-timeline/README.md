# T4: Timeline speed test app

A disposable test app for T4 in `BUILD_PLAN.md`: a timeline with all its state in Rust, comparing DOM items with a canvas. The results are in `docs/spikes.md`.

macOS only. From this folder:

```sh
./run.sh           # The self-test, at 500 and 2,000 items. Takes about two minutes.
./run.sh hand dom  # Open a mode, dom or canvas, to try by hand.
```

Reports land in `out/`.
