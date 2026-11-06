# T3: Launch bar test app

A disposable test app for T3 in `BUILD_PLAN.md`: a small, transparent, always-on-top bar that grows upward when a popover opens. The results are in `docs/spikes.md`.

macOS only. From this folder:

```sh
./run.sh        # The self-test, which takes about 15 seconds. Results land in out/.
./run.sh hand   # Open the bar to try by hand.
```

## Hand check

The self-test can't check these, so they need a person:

1. Drag the bar by its grey handle. It should follow the mouse smoothly.
2. If you have a second monitor, drag the bar onto it, then click **Audio**.
3. With another app in front, click **Audio** once. The popover should open on that first click.
4. Click on another app beside the bar. The other app should get the click.
