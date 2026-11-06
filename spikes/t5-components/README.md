# T5: Rust/UI on real webviews

A disposable test app for T5 in `BUILD_PLAN.md`: every vendored Rust/UI component on a real webview, driven with real input through AppKit. The results are in `docs/spikes.md`.

macOS only. From this folder:

```sh
./run.sh        # The self-test, which takes about 25 seconds. Results land in out/.
./run.sh hand   # Open the page to try by hand.
```

It refuses to run while the screen is locked, because windows can't take focus or be captured then. Failures in its output are findings about the components, so a run with failures isn't a failed run. Only `INCONCLUSIVE` means something went wrong with the test itself.

## Hand check

Hover over **Hover me**. A tooltip should appear. A synthetic mouse move doesn't set `:hover`, so the self-test can't check this.
