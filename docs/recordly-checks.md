# Recordly hand checks

These are the checks for step 0.4 of `BUILD_PLAN.md`. Each one tests a likely bug from §58 of the feature list on a real Mac. All of them come from reading the code, and none has been seen at runtime yet.

Use the build at `~/Projects/Recordly/release/mac-arm64/Recordly.app`. Afterwards, move anything confirmed to the "Confirmed" list in §58, and correct or remove anything that turns out not to be real.

**Back up first.** Finishing a recording makes Recordly delete older recordings (`electron/ipc/recording/prune.ts:120`). It removes any older than 14 days, or beyond the newest 20, unless a saved project uses them. Copy `~/Library/Application Support/Recordly/recordings` somewhere safe if you want to keep what's there.

## Do this first: macOS mic loss

Step 4.5 depends on this result.

1. In the Audio popover, turn on both the mic and system audio.
2. Record for about 20 seconds. Speak while something distinctive plays through the speakers.
3. Stop, and listen in the editor.
4. Check that the mic was captured separately. A `.mic.m4a` file should sit beside the new recording:
   ```sh
   ls -t ~/Library/Application\ Support/Recordly/recordings | head
   ```

**If the bug is real:** you hear the system audio but not your voice, even though `.mic.m4a` exists.

| Result | Notes |
|---|---|
| | |

## The other checks

| §58 issue | Plan step | How to test | If the bug is real | Result |
|---|---|---|---|---|
| Custom motion values snap to a preset | M2 exit | Set a custom Cursor Size, Bounce and Bounce Speed. Save, then reopen the project. | The values change to the Focused preset | |
| Choosing a recordings folder wipes audio settings | 2.9 | Set audio options, then choose a new recordings folder and relaunch. | The audio options are back to defaults | |
| A project reads as dirty after opening | 2.7 | Open a saved project and change nothing. Wait two seconds, then try to close it. | You get an unsaved-changes prompt, or the file's modified time changes | |
| Selection changes use up undo steps | 2.4 | Click between timeline items without editing anything, then press Undo. | Undo steps back through the selections | |
| Splitting a clip at non-1× speed misaligns it | 2.3 | Set a clip to 2×, split it, then play across the cut. | The right half shows the wrong frames | |
| Closing the editor strands the app | 5.7 | Finish a recording, then close the editor window. | The launch bar doesn't come back | |
| Cancelling the countdown may hang | 5.6 | Start a recording with a countdown, then cancel during the countdown. | Record stays disabled | |
| The keyboard can't delete clips | 6.6 | Select a clip on the timeline and press Delete. | Nothing happens | |
| `.recordly` double-click does nothing useful | 6.9 | Double-click a saved project in Finder. | The app opens without that project | |
| GIF "loop off" plays twice | 8.5 | Export a short GIF with Loop off, then open it in a browser. | It plays twice before stopping | |
| Cancelling an export reports an error | 8.6 | Start a long MP4 export and cancel it. | An error message appears | |
| Caption files aren't adjusted for edits | 9.6 | Generate captions, cut a section near the start, and export with caption files. | The `.srt` times no longer match the video | |
| Captions burn in with Show off | 9.6 | Generate captions, turn Show off, and export an MP4. | Captions appear in the export | |
