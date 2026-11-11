# Reel: Build Plan

Reel is a screen recorder and editor written in Rust, with its UI built on [Rust/UI](https://rust-ui.com/).

- **Written:** 6 Nov 2026
- **Status:** Milestones 0, 2 and 3 are done. In Milestone 1, T2, T3 and T4 pass, and T1 and T5 pass on macOS but haven't been run on Windows or Linux. Milestones 4 to 9 are built on macOS. Their tests pass, except that real capture is untested because it needs permissions a person has to grant (`scripts/run-record-test.sh --ask`). Each milestone says what's left. Milestone 10 is partly done, and 11 and 12 aren't started.
- **Logo:** `assets/logo.svg`

## Approach

1. **Riskiest work first.** Milestone 1 tests the parts that could force a change of UI toolkit, before anything depends on them.
2. **Every milestone ends with something you can run.** Each one has exit criteria that can be checked, so progress is never just "the code exists".
3. **One platform first.** macOS comes first. Windows and Linux follow once the product works.
4. **One renderer.** Preview and export share the same GPU code, so they can't disagree about how a project looks.
5. **Rust/UI for controls, wgpu for pixels.** Anything that ends up in the exported video is drawn by wgpu. Everything you click on is Rust/UI.

## Milestones

Sizes are relative to each other. They aren't time estimates.

| # | Milestone | What you can run at the end | Size |
|---|---|---|---|
| 0 | Foundations | An empty Reel window with the logo, building on all three OSes | S |
| 1 | Go/no-go tests | Small test apps that prove or rule out the architecture | M |
| 2 | Core model and project format | A CLI that converts and round-trips project files | M |
| 3 | Renderer and preview | Open a video, preview it with effects, export an MP4 | L |
| 4 | Recording on macOS | Record screen, audio, cursor and webcam from a CLI | L |
| 5 | Launch bar | Record from the real launch bar and land in the editor | M |
| 6 | Editor shell and timeline | Cut a recording in the real UI, save and reopen it | L |
| 7 | Effects and overlays | Zooms, cursor, webcam, annotations, audio and presets | L |
| 8 | Full export | MP4 and GIF with every feature, progress and cancel | M |
| 9 | Captions | Local transcription, editing, burn-in and caption files | M |
| 10 | macOS beta | A signed, notarised app on a clean Mac | M |
| 11 | Windows | Feature parity on Windows | L |
| 12 | Linux | Feature parity on Linux, within platform limits | L |
| 13 | 1.0 | Audited, released on all three OSes | S |

## Architecture

```text
+------------------------- Editor window --------------------------+
|  Transparent webview: Dioxus + Rust/UI                           |
|  header, panels, timeline, menus, dialogs, invisible drag handles|
|  (clear area over the preview, so menus can overlap the video)   |
+------------------------------------------------------------------+
|  wgpu surface: background, video, cursor, annotations, webcam,   |
|  captions                                                        |
+------------------------------------------------------------------+

reel-render -> wgpu surface        = preview
reel-render -> offscreen -> encoder = export
```

### Workspace

| Crate | Responsibility | Depends on UI or GPU? |
|---|---|---|
| `reel-core` | Project model, timeline edits, undo, settings | No |
| `reel-motion` | Zoom, cursor, spring and suggestion maths as pure functions | No |
| `reel-media` | Demux, decode, encode, mux, audio mixing | No |
| `reel-render` | The one wgpu compositor | GPU |
| `reel-capture` | Platform capture and cursor tracking | No |
| `reel-recorder` | Separate recording process, so a UI crash can't lose a recording | No |
| `reel-captions` | Whisper, segmentation, caption layout | No |
| `reel-cli` | Headless render and test harness | GPU |
| `reel-app` | Dioxus desktop app, windows, vendored Rust/UI | Both |

### Candidate crates

All of these were on crates.io with a release in the last 18 months when checked on 6 Nov 2026. Confirm each one when you adopt it.

| Need | Candidates |
|---|---|
| UI shell | `dioxus` and `dioxus-desktop` 0.7, `wry`, `tao`, `tw_merge`, `icons` |
| GPU | `wgpu` |
| macOS capture and media | `screencapturekit`, `objc2-av-foundation`, `objc2-video-toolbox` |
| Windows capture and media | `windows-capture`, `windows` |
| Linux capture | `ashpd`, `pipewire` |
| Decode, encode, mux | `ffmpeg-next`, or native APIs per OS; `gstreamer` and `openh264` as options |
| Audio | `cpal`, `symphonia`, `rubato`, `webrtc-audio-processing` (non-standard licence, so check it) |
| Webcam | `nokhwa`, or native APIs per OS |
| Text in wgpu | `cosmic-text`, `glyphon` |
| Captions | `whisper-rs` |
| GIF | `gif` |
| Desktop integration | `tray-icon`, `muda`, `global-hotkey` |
| Packaging and updates | `dx bundle`, `cargo-packager`, `cargo-packager-updater` |
| Translation | `rust-i18n` |
| Testing | `insta`, `proptest` |

**Avoid:** `mp4` and `rdev`, which have had no release since 2023.

## Decisions needed

Each decision has a recommended default, so work can start. Change any of them before the milestone listed.

**Settled on 6 Nov 2026:** D1 (AGPL-3.0-only), D2 and D3, each at its recommended default. D5 was settled by T2, against the default below: native APIs on macOS, with an LGPL FFmpeg build as the default for Windows and Linux (`docs/spikes.md`).

| # | Decision | Recommended default | Needed by |
|---|---|---|---|
| D1 | **Licence** | AGPL-3.0-only | M0 |
| D2 | **Platform order** | macOS, then Windows, then Linux | M0 |
| D3 | **Scope:** accept the "Out of scope" list below? | Yes | M0 |
| D4 | **Wayland at launch?** Wry's layering example panics on Wayland, and its `build_as_child` only supports X11. Dioxus 0.7.9 also ignores `with_as_child_window` on all of Linux, so X11 needs a patch or a workaround too (`docs/spikes.md`). | Test it in M1, ship Linux after macOS and Windows | M1 |
| D5 | **Media library:** FFmpeg or native APIs per OS? | An LGPL FFmpeg build through `ffmpeg-next`, so all three OSes share one pipeline. Settle it in test T2. | M1 |
| D6 | **Project file** | A new `.reel` format | M2 |
| D7 | **Recording retention:** delete recordings after 14 days or beyond the newest 20? | Off by default, with an opt-in setting | M4 |
| D8 | **Wallpapers and cursors:** where does the artwork come from? | Ship only assets you have rights to | M7 |
| D9 | **App icon background** | Black reel on a white rounded tile | M10 |

## Out of scope

Accepting D3 removes all of this from scope.

- **Extras:** extensions, device frames and accounts
- **More than one export path:** one pipeline and one compositor per OS
- **Browser capture:** WebM recording and browser mic capture. WebM files can still be imported.
- **Local servers and loose security:** no media or renderer server, and no turning off the webview's security
- **Announcement feed:** add one if there's ever something to announce
- **Most environment variables:** keep only the few that tests need

---

## Milestone 0: Foundations

**Goal:** a clean repo and working builds.

- [x] **0.1** Settle D1, D2 and D3.
- [x] **0.2** Make the first commit: `.gitignore`, licence, this plan, logo. The first commit already held the plan and logo, so the `.gitignore` and licence follow it.
- [x] **0.3** Create the Cargo workspace with the crates above. Pin the toolchain and turn on `rustfmt`, `clippy` and `cargo deny`.
- [x] **0.4** Set up CI to build and test on macOS, Windows and Linux on every push.
- [x] **0.5** Install `dioxus-cli` and create `reel-app`. Pin Dioxus to an exact 0.7 version. Rust/UI itself pins `=0.7.9`, and 0.8 is still in alpha. The pin only covers the top-level crate, so `Cargo.lock` holds the sub-crates at 0.7.9 and `deny.toml` enforces it.
- [x] **0.6** Vendor Rust/UI:
  - Copy from `app_crates/registry/src/ui/` at a pinned commit. Don't use `ui-cli`, whose README still targets Leptos.
  - Start with Button, Card, Dialog, Alert Dialog, Dropdown Menu, Input, Label, Popover, Select, Separator, Skeleton, Slider, Switch, Tabs, Toggle Group, Toast, Tooltip, Progress, Spinner, Kbd, Scroll Area, Accordion and Context Menu.
  - Set up the Tailwind v4 build.
  - Record the commit and MIT licence in `VENDORED.md`.

**Exit criteria**
- `dx serve --platform desktop` opens a Reel window with the logo and a working Rust/UI button.
- Light and dark themes both work, and the logo is visible in both.
- CI is green on all three OSes.

## Milestone 1: Go/no-go tests

**Goal:** prove the architecture with small, disposable test apps. Write each result in `docs/spikes.md`.

- [ ] **T1: Layering.** Play video in a wgpu surface under a transparent Dioxus webview, using `Config::with_as_child_window`. Open a Rust/UI popover and dialog over the video. Test on macOS, Windows, X11 and Wayland.
  - **Pass:** smooth 1080p60 playback, DOM controls drawn over the video and receiving clicks, and alignment that holds through window resizing.
  - **Status:** macOS passes, using `spikes/t1-layering` (`docs/spikes.md`). Still to run: Windows, X11 and Wayland, plus a hand check on the Mac with a real mouse.
- [x] **T2: Decode.** Hardware-decode H.264 up to 4K60 into a wgpu texture, and compare FFmpeg with native APIs (D5).
  - **Pass:** real-time playback at 4K60 on the reference Mac, with responsive seeking.
- [x] **T3: Launch bar window.** Build a small, transparent, always-on-top window that can be dragged between monitors and grows when a popover opens. Don't use a full-screen window with mouse forwarding.
  - **Pass:** no flicker on resize, clicks outside the bar reach other apps, and the window stays out of its own recordings.
  - **Status:** passes on macOS (`docs/spikes.md`). Dragging between monitors still needs a hand check, because this Mac has one display, and ScreenCaptureKit exclusion needs step 4.3.
- [x] **T4: Timeline speed.** Build a timeline with 500 items and a moving playhead in Dioxus. Measure drag and playhead latency across the webview boundary. Compare DOM items with `<canvas>`.
  - **Pass:** a chosen approach that holds 60 fps while dragging and playing.
- [ ] **T5: Rust/UI on real webviews.** Render every vendored component in WKWebView, WebView2 and WebKitGTK. Its own tests run only on Chromium, so list every failure. Also list each `wasm32`-only branch that those components use, since those do nothing on desktop.
  - **Pass:** a complete list of what needs fixing.
  - **Status:** done for WKWebView (`docs/spikes.md`). Still to run: WebView2 and WebKitGTK.

**If a test fails**
- **T1 fails on macOS or Windows:** stop and revisit the UI choice before M2.
- **T1 fails only on Wayland:** try running under XWayland, or copying reduced-resolution frames to a `<canvas>` on Linux only. Update D4.
- **T4 fails with DOM items:** draw the timeline items on a canvas or in wgpu, and keep Rust/UI for everything around it.

**Exit criteria**
- All five results are written up.
- D4 and D5 are settled.
- The architecture above is confirmed or changed.

## Milestone 2: Core model and project format

**Goal:** a tested data model with no UI or GPU dependency.

- [x] **2.1** Define the project types in `reel-core`. Give clips a source position as well as a timeline position, which makes real reordering possible.
- [x] **2.2** Use separate types for source time and timeline time. This makes mixing up click times and zoom times a compile error.
- [x] **2.3** Implement edits as commands with overlap rules. Include splitting at speeds other than 1×.
- [x] **2.4** Build undo over the whole document, including look settings and crop. Merge slider drags and typing into single steps, and leave selection changes out of history.
- [x] **2.5** Define `.reel` as versioned JSON with explicit migrations. Store relative media paths with an absolute fallback.
- [x] **2.6** Implement atomic save and `.bak`.
- [x] **2.7** Make autosave cover new recordings too, and stop projects from reading as dirty on open.
- [x] **2.8** Build one Rust-owned settings store with no partial overwrites. Don't use `localStorage`, which can't be shared safely across windows.

**Exit criteria**
- A project saves and reloads without changes.
- Property tests cover the edit rules.
- Custom cursor values survive a reload.

## Milestone 3: Renderer and preview

**Goal:** the first end-to-end slice. Open a video, preview it with effects, and export it through the same renderer.

- [x] **3.1** Decode video to GPU textures in `reel-media`, using the T2 result.
- [x] **3.2** Build the compositor with one fixed draw order: background, shadow, then video, cursor and annotations inside the camera, then webcam and captions outside it.
- [x] **3.3** Implement layout, padding, crop, squircle corners and shadow, with one shadow look on every path.
- [x] **3.4** Implement image, colour, gradient and video backgrounds with blur. Honour gradient angles, and use the same blur radius in preview and export.
- [x] **3.5** Implement the zoom engine in `reel-motion`, including connected zooms.
- [x] **3.6** Compute camera and cursor paths ahead of time at a fixed step. Preview and export sample the same path, so they can't move differently.
- [x] **3.7** Play audio with `cpal` and use it as the master clock. Support speed changes with pitch preserved, and skip clip gaps.
- [x] **3.8** Add `reel-cli render`, which writes H.264 MP4 through a hardware encoder.
- [x] **3.9** Embed the preview in `reel-app` with basic transport controls.
- [x] **3.10** Add golden-frame tests: render known frames headlessly and compare them with stored images.

**Exit criteria**
- Open a video file, apply a look from a config file, and play it smoothly with synced audio.
- The exported MP4 matches the preview at sampled frames, within a set tolerance.
- Export speed is recorded.

**Result.** All three pass on macOS. `scripts/run-app-test.sh` plays a project with a cursor, webcam, annotations and a zoom at 2× and 120 Hz: the clock holds 1.000×, the picture stays within 12.4 ms of it at p99, and the screen matches the export path's still at 32 dB. Exported frames match those stills at 47–54 dB. Nobody has listened to the audio yet, so "synced" rests on audio being the master clock. Video backgrounds (part of 3.4) move to 7.1. Until then a project with one warns and shows its fallback colour.

| Project | Export speed |
|---|---|
| 1080p30 | 10.62× real time |
| 1440p60 | 3.60× real time |
| 4K with webcam | 3.43× real time |

Measured with `reel-cli render` on an M4 Max, with every overlay these projects use turned on.

The webcam's margin and corner radius scale from a 1920×1080 reference, like the frame and cursor, so the bubble keeps its proportions at every resolution and matches the preview.

## Milestone 4: Recording on macOS

**Goal:** reliable recording from the CLI, before any recording UI exists.

- [x] **4.1** Build `reel-recorder` as a separate process with start, pause, resume and stop commands over stdin and stdout.
- [ ] **4.2** List displays and windows with thumbnails, filtered and named.
- [ ] **4.3** Capture with ScreenCaptureKit to H.264, tagged BT.709, at 60 fps. Exclude Reel's windows in the content filter rather than relying only on content protection.
- [x] **4.4** Write fragmented MP4 while recording, so a crash still leaves a playable file, then finalise on stop.
- [ ] **4.5** Keep system audio and mic as separate tracks aligned to the first frame. The editor must use both, so the mic is never lost.
- [ ] **4.6** Track the cursor: 33 ms samples, click events through a listen-only event tap, and cursor shapes. Stream samples to disk rather than capping them in memory.
- [ ] **4.7** Record the webcam to a separate file with a sync offset.
- [x] **4.8** Implement pause and resume, and the warm-start countdown that removes startup lag.
- [x] **4.9** Add validation on stop, recovery after a crash, and a disk-space check.
- [ ] **4.10** Handle permissions. Ask only when a feature needs it, rather than on every launch.
- [x] **4.11** Implement retention according to D7.

**Exit criteria**
- A 10-minute recording with screen, mic, system audio and webcam, paused twice, plays with drift under 40 ms.
- Force-quitting the app mid-recording still leaves a playable file.

**Result so far.** Everything after capture passes through a test pattern that flashes and beeps on one clock, run through the real recorder process (`crates/reel-recorder/tests/record.rs`). Ten minutes at 1080p60, paused twice: 36,003 frames, none dropped, 600.03 s long, and a flash and its beep at most 33.4 ms apart, or 11.7 ms in the last minute. `kill -9` leaves a video that plays and validates, plus sound and cursor data that `recover` saves. A deliberate 100 ms offset in the sound fails these tests, so they can catch drift.

**Not tested yet, because it needs a person to grant permissions:** real capture with ScreenCaptureKit (4.2, 4.3), real system audio and mic clocks (4.5), click tracking (4.6), the webcam (4.7) and the permission prompts (4.10). All are written. `scripts/run-record-test.sh --ask` tests them. The test pattern's tracks all share one clock, so real drift between devices can only show up there.

Fragmented MP4 and frame reordering don't mix: AVAssetWriter fails with `-16341` at a fragment boundary, 6 times out of 6 at 1080p60. With reordering off, it's 0 in 36. Recordings turn it off, and exports, which aren't fragmented, keep it.
- Both mic and system audio can be heard separately in the result.

## Milestone 5: Launch bar

**Goal:** the real recording flow, with no terminal involved.

- [x] **5.1** Build the launch bar window from the T3 result.
- [x] **5.2** Change the vendored Popover:
  - A Rust-controlled `open` signal
  - One open at a time
  - Positioning that doesn't need CSS anchor positioning, which is unsupported before Safari 26 while the minimum is macOS 14
  - Removal of its `document` listeners on unmount
- [x] **5.3** Grow the window when a popover opens, and shrink it when the popover closes.
- [ ] **5.4** Build the Source, Audio (with level meters), Webcam, Countdown and More popovers.
- [x] **5.5** Build the recording state: REC and PAUSED labels, timer, pause, stop, and cancel with a confirmation.
- [ ] **5.6** Add the source highlight, countdown and floating webcam preview windows.
- [x] **5.7** Handle finalising, then open the editor. Closing the editor brings the launch bar back.
- [ ] **5.8** Add a global hotkey to start and stop recording.

**Exit criteria**
- Launch, pick a source, record, stop and reach the editor without a terminal.
- The launch bar can move to a second monitor and remembers its position.
- Nothing from Reel appears in the recording.

**Result so far.** `scripts/run-bar-test.sh` clicks through the real bar on a test pattern: popovers grow the window without moving the bar, one opens at a time, and a recording runs through pause, the cancel confirmation and stop to the editor, and closing that brings the bar back. Step 5.2 replaced the vendored Popover with a small one owned by the bar, rather than changing it, because all it needs is the four things listed there. The app opens as the bar alone: the user asked for the empty "open a project" screen to go, so the editor window only appears with a project loaded, and if a file can't be opened the bar says why. The test asks AppKit which windows are really showing, because Dioxus shows a window itself just after its first render, which the app's own flag can't see.

**Not tested yet:** the real source list and thumbnails (5.4), mic meters and the highlight and camera preview windows (5.6), which need permissions; the hotkey with a real key press (5.8), which is registered but which a test can't press without Accessibility; and moving the bar to a second monitor, which needs one. "Nothing from Reel appears in the recording" needs `scripts/run-record-test.sh`.

## Milestone 6: Editor shell and timeline

**Goal:** a usable basic editor.

- [x] **6.1** Build the layout: header, icon rail, settings panel, preview, timeline.
- [ ] **6.2** Keep the wgpu surface aligned with the clear area in the webview through resizes and DPI changes. Start from the settings that worked in T1 (`docs/spikes.md`). DPI changes are still untested.
- [x] **6.3** Build shared controls: a labelled slider with reset, and a colour picker with swatches and hex, since Rust/UI has none.
- [ ] **6.4** Change the other vendored components according to T5:
  - Focus trap in Dialog
  - Arrow-key navigation in Dropdown Menu, Select and Context Menu
  - Action button on Toast
  - Light, Dark and System theme, stored in Rust settings
  - A change handler on Switch, and a working `close_context_menu()`, the only `wasm32`-only branch in the vendored components
- [x] **6.5** Build the timeline from the T4 result: rows, ruler, zoom and pan, playhead, keyframe markers, hover ghosts. Rust/UI has no timeline, and its Drag and Drop does nothing on desktop, so this is custom.
- [x] **6.6** Add clip editing: split, trim, move, speed, mute and delete, including from the keyboard. Clamp partly overlapping items on trim rather than deleting them, and add real reordering.
- [x] **6.7** Build transport controls and shortcuts with the rebind dialog. Save, Save As and Open must work on every OS.
- [x] **6.8** Build the project browser, rename, unsaved-change prompts and relinking of missing media.
- [ ] **6.9** Support dropping files on the app and opening `.reel` files from the OS.

**Exit criteria**
- Open a recording, cut and reorder clips, save, quit, reopen, and find everything intact.
- Undo and redo work across every edit made.
- The whole flow works with the keyboard.

**Result.** `REEL_TEST=editor scripts/run-app-test.sh` runs 56 checks with real clicks, keys and drags, and all pass. Saving is checked by reading the file back, and keys go through the application so that they meet the menu as real ones do. Not tested: quitting and reopening in one run, and a DPI change (6.2), which needs a second display.

- **6.2** isn't ticked: resizes are tested and DPI changes aren't.
- **6.4** isn't ticked, because the vendored Dialog, Dropdown Menu, Context Menu and Select aren't used. The editor's menus and dialogs are its own, and have no focus trap or arrow-key navigation yet. Those belong with the accessibility pass in 10.4.
- **6.8** has no thumbnails in the project browser. Unsaved-change prompts aren't needed, because everything autosaves and opening something else saves first. A save that fails says so.
- **6.9** isn't ticked: dropping files and opening `.reel` files from Finder are written, and neither has been tried, since a script can't drag from Finder.

## Milestone 7: Effects and overlays

**Goal:** the features that make Reel's output look good.

- [x] **7.1** Build the Scene panel and bundled assets (D8). Make crop undoable.
- [x] **7.2** Build the Cursor panel and engine: spring, bounce, sway, click effects and loop mode.
- [x] **7.3** Build zooms: add, auto and manual modes, Classic mode, connected zooms, auto-suggestions. Fix the wand's enable rule.
- [x] **7.4** Build the webcam overlay, with crop, positioning, reaction to zoom, and dragging on the preview.
- [x] **7.5** Build annotations: text, image, arrow, blur. Draw them in wgpu, with invisible DOM handles for selecting and dragging.
- [x] **7.6** Build audio editing: added tracks, per-track volume, waveforms. Label ×1.35 as a boost rather than "Normalize", or implement real loudness normalisation.
- [x] **7.7** Build presets and motion presets.

**Exit criteria**
- A sample project using every effect looks the same in preview and in rendered frames.
- Everything saves, reloads and can be undone.

**Result.** Preview and export go through one renderer, and match at 32–33 dB with every effect on.

- **7.1:** there are no bundled wallpapers, because of D8. There are 12 gradients of Reel's own instead.
- **7.4:** the webcam's crop is sliders, not an inline live editor. Dragging the bubble is new.
- **7.5:** annotations can't be dragged between rows, and custom Google Fonts aren't there. Asking the internet for fonts doesn't fit the content policy in 10.5.

## Milestone 8: Full export

**Goal:** dependable output for everything from M7.

- [x] **8.1** Build the export panel: format, quality with final size, encoding mode and frame rate.
- [x] **8.2** Implement the bitrate formula, keyframe intervals, colour tags and constant frame rate.
- [x] **8.3** Implement trim and speed time mapping.
- [x] **8.4** Render audio: mixing, speed with pitch preserved, soft limiter, AAC. Keep mute correct after trims, and use each clip's own track settings.
- [x] **8.5** Build GIF export. "Loop off" must play once, and frames must stream rather than all sitting in memory.
- [x] **8.6** Add progress, render speed, cancel, a disk-space check, temp files, atomic final move and Save Again. Cancelling must not report an error.
- [x] **8.7** Make sure a feature is never silently dropped from an export. It either renders or the export reports why it can't.

**Exit criteria**
- The three sample projects export correctly, at well above real time.

**Result.** They do (Milestone 3's table). GIFs stream to disk, keep their frame rate on average, and play once with Loop off. Export has a stall limit, so an encoder that stops responding becomes an error rather than a hang.

A hang in exports with sound was found by a screenshot. AVAssetWriter holds video back until it has sound from about a second past it, so sound written only after each frame could leave each side waiting for the other, depending on timing. `an_export_with_sound_and_a_blur_finishes` reproduces it.

The Fast, Balanced and Quality encoding modes only set the bitrate.
- Cancelling leaves no stray files.
- Golden-frame tests cover every effect.

## Milestone 9: Captions

**Goal:** local captions that transcribe the right audio.

- [x] **9.1** Run Whisper locally through `whisper-rs`. Add model download with a checksum, plus delete and custom model.
- [x] **9.2** Add an audio source selector that defaults to the mic when one exists. This fixes the wrong-track and no-audio failures.
- [x] **9.3** Implement segmentation, tested with sample transcripts.
- [x] **9.4** Implement line layout and animation, with text drawn in wgpu through `cosmic-text` so preview and export match.
- [ ] **9.5** Build caption editing. For inline editing, place a DOM text box over the caption's computed position.
- [x] **9.6** Export SRT and VTT mapped through trims and speed changes, and respect the Show switch at export. Report any write failure to the user.

**Result.** Real speech from macOS's `say` goes through the real model and the real panel, onto the timeline and into the picture. The model's checksum is verified on download.

**Silences:** Whisper's timings smear real words into pauses, so dropping every word inside a long silence loses real speech after a pause. A test with real audio showed this. Runs of words that flow up to the end of a silence and on into the next word are moved onto that speech, and only words stranded inside it are dropped.

- **9.5** isn't ticked: captions are edited in the panel, without the inline box over the preview.
- Whisper has only been tried on clear synthetic speech with the tiny model. Real voices and other languages haven't been tried.

**Exit criteria**
- A mic-and-system recording produces captions of the voice, not the system audio.
- Caption files stay in sync after cuts and speed changes.

## Milestone 10: macOS beta

**Goal:** something you could give to another person.

- [ ] **10.1** Build the settings surface.
- [x] **10.2** Add the macOS menu. Remove Toggle DevTools from production builds.
- [ ] **10.3** Add translations for the 10 languages.
- [ ] **10.4** Do an accessibility pass: labels, keyboard access to the launch bar, reduced motion. Rust/UI's RTL support may also close that gap.
- [x] **10.5** Set the webview security policy, allow only the IPC the app needs, and scope file access.
- [x] **10.6** Confirm network use is limited to what the user asks for, plus the update check.
- [x] **10.7** Add diagnostics: recording and export logs, with pruning.
- [x] **10.8** Generate the app icon set from `assets/logo.svg`, with padding, according to D9.
- [ ] **10.9** Package a DMG for x64 and arm64, with hardened runtime, signing and notarisation. Check the entitlements needed for the transparent window, and that wry's private `drawsBackground` setting doesn't get in the way of notarisation.
- [ ] **10.10** Add auto-update with stable and experimental channels.

**Exit criteria**
- A notarised build installs and runs on a clean Mac with no developer tools.
- Every feature is audited on macOS, and marked done, changed or dropped.
- An update installs from one beta to the next.

**Result so far.** `scripts/package-macos.sh` makes a 12 MB DMG. Mounted, its signature verifies, and the app opens from it and runs. It's signed ad hoc, and for arm64 only on this Mac, because Homebrew's Rust has no other target.

- **10.1** has every setting except language.
- **10.3 isn't started.** Everything is in English. The UI's text is written inline, so this needs a pass over all of it.
- **10.4 is partly done:** controls have labels and reduced motion is respected, but keyboard access to the launch bar and focus handling in panels haven't been gone through.
- **10.5:** pages can't load anything from outside the app, through a content policy that's been run with every self-test. Dioxus still gets all of its IPC, and file access isn't scoped: the app reads what the user opens.
- **10.6:** `reel-captions/src/models.rs`, which downloads a model when asked, is the only place with a network client. There's no update check, since 10.10 isn't built.
- **10.9 needs your Apple Developer certificate.** The script signs with hardened runtime and notarises when given one, but that path has never run, so the entitlements and wry's private `drawsBackground` are unchecked. A universal build needs `rustup`.
- **10.10 isn't started.** It needs somewhere to publish releases.
- **10.8** uses D9's default, a black reel on a white rounded tile, from `packaging/macos/icon.svg`.

## Milestone 11: Windows

**Goal:** the same product on Windows.

- [ ] **11.1** Re-run T1, T3 and T5 on real Windows hardware, including high-DPI and multiple monitors.
- [ ] **11.2** Capture with Windows Graphics Capture to H.264 through a hardware encoder.
- [ ] **11.3** Capture system audio through WASAPI loopback, with timing repair, and capture the mic natively.
- [ ] **11.4** Add native mic echo cancellation and noise suppression, replacing what Chromium provided.
- [ ] **11.5** Track cursor shapes and clicks, including the closed-hand gap and reliably hiding the real cursor.
- [ ] **11.6** Handle a source window that closes while recording.
- [ ] **11.7** Replace the macOS menu with in-app access to everything it offered.
- [ ] **11.8** Build an installer, with code signing and WebView2 runtime handling.

**Exit criteria**
- The M4 recording test and the M8 export tests pass on Windows.
- Captions work on native recordings.
- Save works from the keyboard.

## Milestone 12: Linux

**Goal:** the same product on Linux, within what the platform allows.

- [ ] **12.1** Apply the T1 result and D4 for Wayland and X11.
- [ ] **12.2** Capture through the XDG desktop portal and PipeWire.
- [ ] **12.3** Capture system audio and mic through PipeWire, as separate tracks where possible.
- [ ] **12.4** Do what's possible for cursor data on Wayland, and write down the limits.
- [ ] **12.5** Add the tray with Open, Quit, Show Controls and Stop Recording.
- [ ] **12.6** Handle a second launch during recording without affecting it.
- [ ] **12.7** Package an AppImage and test it on the WebKitGTK versions of current mainstream distros.

**Exit criteria**
- The M4 recording test and the M8 export tests pass on one Wayland distro and one X11 setup.
- Known platform limits are listed in the README.

## Milestone 13: 1.0

**Goal:** a release you can stand behind.

- [ ] **13.1** Finish the feature audit across all three OSes.
- [ ] **13.2** Run a test matrix: each OS, with and without a webcam, one and two monitors, light and dark, long recordings.
- [ ] **13.3** Automate releases: builds, smoke tests, checksums.
- [x] **13.4** Write the README, user docs and licence notices, including Rust/UI's MIT notice and FFmpeg's.
- [ ] **13.5** Publish 1.0. Add Homebrew and winget if wanted.

**Exit criteria**
- No open bugs that lose data or break recording.
- The first-run flow works on a clean machine on each OS.

---

## Testing

| Layer | How | From |
|---|---|---|
| Edit rules, motion maths, parsing | Unit and property tests | M2 |
| Rendering | Golden frames through `reel-cli` | M3 |
| Recording | Long-run test scripts on real hardware | M4 |
| UI | Manual checklist at first; add automation where it proves stable | M5 |
| Parity | Compare preview with export, and audit every feature | M7 |

## Main risks

| Risk | Mitigation |
|---|---|
| Webview-over-wgpu layering fails on a platform | Tested first in T1, with fallbacks listed |
| Rust/UI is early stage and experimental | Vendored at a pinned commit, so upstream changes can't break the app |
| Rust/UI's desktop behaviour is lightly tested upstream | T5 audit, with changes scheduled in M5 and M6 |
| Dioxus 0.8 brings breaking changes | Pin an exact version, and upgrade deliberately after 1.0 |
| Mic quality drops without Chromium's audio processing | Evaluate `webrtc-audio-processing` during M4 and M11 |
| GPU and driver variety on Windows and Linux | Test on real hardware in M11 and M12, and offer a software encoder fallback |
| Scope creep | The "Out of scope" list, and the macOS-first order |

## After 1.0

Candidates this plan doesn't include:

- **Capture:** area recording, keystroke overlay, restarting a recording, toggling the mic while recording
- **Editing:** snapping, ripple delete, multi-select, copy and paste, J/K/L transport, row labels and locks, transitions, device frames, annotation animation, editable easing
- **Audio:** real loudness normalisation if not done in M7, fades, trimming into a file, noise removal
- **Captions:** active word highlighting, font choice, translation
- **Projects:** packing media into the project, project search, sort and delete
- **Export:** HEVC, AV1, ProRes, WebM, transparent video, batch export, render resume
- **Sharing:** upload and share links
