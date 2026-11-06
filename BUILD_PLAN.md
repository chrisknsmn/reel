# Reel: Build Plan

Reel is a Rust rewrite of the Recordly screen recorder and editor, with its UI built on [Rust/UI](https://rust-ui.com/).

- **Written:** 6 Nov 2026
- **Status:** Milestones 0 and 2 are done, apart from the hand checks in step 0.4. In Milestone 1, T2, T3 and T4 pass, and T1 and T5 pass on macOS but haven't been run on Windows or Linux.
- **Spec:** `Recordly-Feature-List.md`. In this plan, `§N` means section N of that file.
- **Logo:** `assets/logo.svg`

## Approach

1. **Riskiest work first.** Milestone 1 tests the parts that could force a change of UI toolkit, before anything depends on them.
2. **Every milestone ends with something you can run.** Each one has exit criteria that can be checked, so progress is never just "the code exists".
3. **One platform first.** macOS comes first because it has the fullest feature set in Recordly (§1). Windows and Linux follow once the product works.
4. **One renderer.** Preview and export share the same GPU code. This removes most of the mismatches listed in §31, §37 and §38.
5. **Rust/UI for controls, wgpu for pixels.** Anything that ends up in the exported video is drawn by wgpu. Everything you click on is Rust/UI.
6. **Don't copy Recordly's bugs.** Fixes from §58 are written into the steps that touch that area.

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
| D1 | **Licence:** stay AGPL-3.0 or aim for an independent implementation? Part 2 of the feature list closely paraphrases AGPL code, and Recordly's translations are AGPL too. | Stay AGPL unless you plan to ship closed source. If you do, get legal advice before using Part 2. | M0 |
| D2 | **Platform order** | macOS, then Windows, then Linux | M0 |
| D3 | **Scope:** accept the "Not carrying over" list below? | Yes | M0 |
| D4 | **Wayland at launch?** Wry's layering example panics on Wayland, and its `build_as_child` only supports X11. Dioxus 0.7.9 also ignores `with_as_child_window` on all of Linux, so X11 needs a patch or a workaround too (`docs/spikes.md`). | Test it in M1, ship Linux after macOS and Windows | M1 |
| D5 | **Media library:** FFmpeg or native APIs per OS? Recordly's `ffmpeg-static` is GPL-3.0-or-later. | An LGPL FFmpeg build through `ffmpeg-next`, so all three OSes share one pipeline. Settle it in test T2. | M1 |
| D6 | **Project file** | A new `.reel` format, with one-way `.recordly` import | M2 |
| D7 | **Recording retention:** Recordly silently deletes recordings after 14 days or beyond the newest 20 (§29). | Off by default, with an opt-in setting | M4 |
| D8 | **Wallpapers and cursors:** the feature list flags possible Apple artwork, with its origin unconfirmed. | Ship only assets you have rights to | M7 |
| D9 | **App icon background** | Black reel on a white rounded tile | M10 |

## Not carrying over

Accepting D3 removes all of this from scope.

- **Dead features (§57):** extensions, device frames, the account stub, saved settings nothing reads
- **Extra export paths (§38):** Legacy and "Breeze" pipelines, the pipeline picker, the NVIDIA and Direct3D 11 compositors, FFmpeg static layout
- **Browser capture paths (§27):** WebM recording and browser mic capture. WebM files can still be imported.
- **Local servers and loose security (§50):** the media server, renderer server, `webSecurity:false` and `read-local-file`
- **Announcement feed (§25):** add it back if you ever have something to announce
- **Recordly's identity:** Discord and feedback links, update feed, app ID, `.openscreen` files
- **Most environment variables (§54):** keep only the few that tests need

---

## Milestone 0: Foundations

**Goal:** a clean repo, working builds, and measurements of Recordly to compare against.

- [x] **0.1** Settle D1, D2 and D3.
- [x] **0.2** Make the first commit: `.gitignore`, licence, feature list, this plan, logo. The first commit already held the plan, feature list and logo, so the `.gitignore` and licence follow it.
- [x] **0.3** Correct the feature list:
  - No manual save on Windows or Linux (§20, §58)
  - Captions have no audio on native Windows recordings (§36, §58)
  - "blur 0–2" means zoom motion blur, and background blur is 0–8 (§45)
  - The Linux "bring window to front" looks unreachable (§3)
  - Add shadcn, sonner, the icon libraries and Tailwind 3 to §56. This plan first said lucide, but only `components.json` names it. Recordly really uses Phosphor and react-icons.
- [ ] **0.4** Build and run Recordly on the Mac. Confirm by hand the §58 items that affect the plan, starting with the macOS mic loss. The checks are in `docs/recordly-checks.md`.
- [x] **0.5** Record a baseline. Use Recordly's headless export (§55) on three sample projects: 1080p30, 1440p60, and 4K with webcam. Keep the output files and timings outside git. `scripts/record-recordly-baseline.sh` regenerates them in `baseline/`.
- [x] **0.6** Create the Cargo workspace with the crates above. Pin the toolchain and turn on `rustfmt`, `clippy` and `cargo deny`.
- [x] **0.7** Set up CI to build and test on macOS, Windows and Linux on every push.
- [x] **0.8** Install `dioxus-cli` and create `reel-app`. Pin Dioxus to an exact 0.7 version. Rust/UI itself pins `=0.7.9`, and 0.8 is still in alpha. The pin only covers the top-level crate, so `Cargo.lock` holds the sub-crates at 0.7.9 and `deny.toml` enforces it.
- [x] **0.9** Vendor Rust/UI:
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
- [x] **T3: Launch bar window.** Build a small, transparent, always-on-top window that can be dragged between monitors and grows when a popover opens. Don't port Electron's full-screen mouse forwarding (§47).
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

- [x] **2.1** Define the project types in `reel-core`. Give clips a source position as well as a timeline position, which makes real reordering possible (§15, §45).
- [x] **2.2** Use separate types for source time and timeline time. This makes the §58 mix-up between click times and zoom times a compile error.
- [x] **2.3** Implement edits as commands with the overlap rules in §14 and §15. Include splitting at non-1× speeds, which §58 reports as broken.
- [x] **2.4** Build undo over the whole document, including look settings and crop (§18). Merge slider drags and typing into single steps, and leave selection changes out of history (§58).
- [x] **2.5** Define `.reel` as versioned JSON with explicit migrations, unlike Recordly's unchecked `version` (§45). Store relative media paths with an absolute fallback.
- [x] **2.6** Implement atomic save and `.bak` (§45).
- [x] **2.7** Make autosave cover new recordings too (§20), and stop projects from reading as dirty on open (§58).
- [x] **2.8** Write the `.recordly` importer with all the legacy conversions in §45, tested against real project files from M0.
- [x] **2.9** Build one Rust-owned settings store with no partial overwrites (§46, §58). Don't use `localStorage`, because §58 shows how that fails across windows.
- [x] **2.10** Add `reel-cli convert`.

**Exit criteria**
- A real `.recordly` project converts to `.reel` and round-trips without changes.
- Property tests cover the edit rules.
- Custom cursor values survive a reload (§58).

## Milestone 3: Renderer and preview

**Goal:** the first end-to-end slice. Open a video, preview it with effects, and export it through the same renderer.

- [ ] **3.1** Decode video to GPU textures in `reel-media`, using the T2 result.
- [ ] **3.2** Build the compositor with one fixed draw order: background, shadow, then video, cursor and annotations inside the camera, then webcam and captions outside it. This resolves the disagreement in §31.
- [ ] **3.3** Implement layout, padding, crop, squircle corners and shadow (§31), with one shadow look on every path.
- [ ] **3.4** Implement image, colour, gradient and video backgrounds with blur. Honour gradient angles, and use the same blur radius in preview and export (§31).
- [ ] **3.5** Implement the zoom engine in `reel-motion` (§32), including connected zooms. Test against the constants in Part 2, subject to D1.
- [ ] **3.6** Compute camera and cursor paths ahead of time at a fixed step. Preview and export sample the same path, which removes the wall-clock versus content-time difference in §37.
- [ ] **3.7** Play audio with `cpal` and use it as the master clock. Support speed changes with pitch preserved, and skip clip gaps (§17, §37).
- [ ] **3.8** Add `reel-cli render`, which writes H.264 MP4 through a hardware encoder (§39).
- [ ] **3.9** Embed the preview in `reel-app` with basic transport controls.
- [ ] **3.10** Add golden-frame tests: render known frames headlessly and compare them with stored images.

**Exit criteria**
- Open a video file, apply a look from a config file, and play it smoothly with synced audio.
- The exported MP4 matches the preview at sampled frames, within a set tolerance.
- Export speed is recorded against the M0 baseline.

## Milestone 4: Recording on macOS

**Goal:** reliable recording from the CLI, before any recording UI exists.

- [ ] **4.1** Build `reel-recorder` as a separate process with start, pause, resume and stop commands over stdin and stdout (§30).
- [ ] **4.2** List displays and windows with thumbnails and the filtering in §3 and §27.
- [ ] **4.3** Capture with ScreenCaptureKit to H.264, tagged BT.709, at 60 fps (§27). Exclude Reel's windows in the content filter rather than relying only on content protection (§47).
- [ ] **4.4** Write fragmented MP4 while recording, so a crash still leaves a playable file, then finalise on stop (§58, §59).
- [ ] **4.5** Keep system audio and mic as separate tracks aligned to the first frame (§27). The editor must use both, which fixes the mic loss in §58.
- [ ] **4.6** Track the cursor: 33 ms samples, click events through a listen-only event tap, and cursor shapes (§28). Stream samples to disk rather than capping them in memory (§6).
- [ ] **4.7** Record the webcam to a separate file with a sync offset (§5, §27).
- [ ] **4.8** Implement pause and resume, and the warm-start countdown that removes startup lag (§6, §27).
- [ ] **4.9** Add validation on stop, recovery after a crash, and a disk-space check (§6, §59).
- [ ] **4.10** Handle permissions (§48). Ask only when a feature needs it, rather than on every launch.
- [ ] **4.11** Implement retention according to D7.

**Exit criteria**
- A 10-minute recording with screen, mic, system audio and webcam, paused twice, plays with drift under 40 ms.
- Force-quitting the app mid-recording still leaves a playable file.
- Both mic and system audio can be heard separately in the result.

## Milestone 5: Launch bar

**Goal:** the real recording flow, with no terminal involved.

- [ ] **5.1** Build the launch bar window from the T3 result (§2, §47).
- [ ] **5.2** Change the vendored Popover:
  - A Rust-controlled `open` signal
  - One open at a time (§2)
  - Positioning that doesn't need CSS anchor positioning, which is unsupported before Safari 26 while the minimum is macOS 14 (§1)
  - Removal of its `document` listeners on unmount
- [ ] **5.3** Grow the window when a popover opens, and shrink it when the popover closes.
- [ ] **5.4** Build the Source, Audio (with level meters, §27), Webcam, Countdown and More popovers (§2).
- [ ] **5.5** Build the recording state: REC and PAUSED labels, timer, pause, stop, and cancel with a confirmation, which Recordly lacks (§2).
- [ ] **5.6** Add the source highlight, countdown and floating webcam preview windows (§5, §47).
- [ ] **5.7** Handle finalising, then open the editor. Closing the editor brings the launch bar back (§48, §58).
- [ ] **5.8** Add a global hotkey to start and stop recording (§23, §59).

**Exit criteria**
- Launch, pick a source, record, stop and reach the editor without a terminal.
- The launch bar can move to a second monitor and remembers its position (§2).
- Nothing from Reel appears in the recording.

## Milestone 6: Editor shell and timeline

**Goal:** a usable basic editor.

- [ ] **6.1** Build the layout in §7: header, icon rail, settings panel, preview, timeline.
- [ ] **6.2** Keep the wgpu surface aligned with the clear area in the webview through resizes and DPI changes. Start from the settings that worked in T1 (`docs/spikes.md`). DPI changes are still untested.
- [ ] **6.3** Build shared controls: a labelled slider with reset (§7), and a colour picker with swatches and hex, since Rust/UI has none.
- [ ] **6.4** Change the other vendored components according to T5:
  - Focus trap in Dialog
  - Arrow-key navigation in Dropdown Menu, Select and Context Menu (§52)
  - Action button on Toast (§21)
  - Light, Dark and System theme, stored in Rust settings (§22)
  - A change handler on Switch, and a working `close_context_menu()`, the only `wasm32`-only branch in the vendored components
- [ ] **6.5** Build the timeline from the T4 result: rows, ruler, zoom and pan, playhead, keyframe markers, hover ghosts (§14). Rust/UI has no timeline, and its Drag and Drop does nothing on desktop, so this is custom.
- [ ] **6.6** Add clip editing: split, trim, move, speed, mute and delete, including from the keyboard (§15, §58). Clamp partly overlapping items on trim rather than deleting them, and add real reordering.
- [ ] **6.7** Build transport controls (§17) and shortcuts with the rebind dialog (§23). Save, Save As and Open must work on every OS.
- [ ] **6.8** Build the project browser, rename, unsaved-change prompts and relinking of missing media (§20, §59).
- [ ] **6.9** Support dropping files on the app and opening `.reel` files from the OS (§20, §58).

**Exit criteria**
- Open a recording, cut and reorder clips, save, quit, reopen, and find everything intact.
- Undo and redo work across every edit made.
- The whole flow works with the keyboard where §23 says it should.

## Milestone 7: Effects and overlays

**Goal:** the features that make Reel's output look good.

- [ ] **7.1** Build the Scene panel (§8) and bundled assets (D8). Make crop undoable.
- [ ] **7.2** Build the Cursor panel and engine: spring, bounce, sway, click effects and loop mode (§9, §34).
- [ ] **7.3** Build zooms: add, auto and manual modes, Classic mode, connected zooms, auto-suggestions (§10, §33). Fix the wand's enable rule (§58).
- [ ] **7.4** Build the webcam overlay, with crop, positioning, reaction to zoom, and dragging on the preview (§11, §35).
- [ ] **7.5** Build annotations: text, image, arrow, blur (§13). Draw them in wgpu, with invisible DOM handles for selecting and dragging.
- [ ] **7.6** Build audio editing: added tracks, per-track volume, waveforms (§14, §16, §37). Label ×1.35 as a boost rather than "Normalize", or implement real loudness normalisation.
- [ ] **7.7** Build presets and motion presets (§19).

**Exit criteria**
- A sample project using every effect looks the same in preview and in rendered frames.
- The same project matches the M0 baseline closely by eye.
- Everything saves, reloads and can be undone.

## Milestone 8: Full export

**Goal:** dependable output for everything from M7.

- [ ] **8.1** Build the export panel: format, quality with final size, encoding mode and frame rate (§21).
- [ ] **8.2** Implement the bitrate formula, keyframe intervals, colour tags and constant frame rate (§39).
- [ ] **8.3** Implement trim and speed time mapping (§41).
- [ ] **8.4** Render audio: mixing, speed with pitch preserved, soft limiter, AAC (§42). Keep mute correct after trims, and use each clip's own track settings (§58).
- [ ] **8.5** Build GIF export (§43). "Loop off" must play once (§58), and frames must stream rather than all sitting in memory.
- [ ] **8.6** Add progress, render speed, cancel, a disk-space check, temp files, atomic final move and Save Again (§21, §44). Cancelling must not report an error (§58).
- [ ] **8.7** Make sure a feature is never silently dropped from an export (§38). It either renders or the export reports why it can't.

**Exit criteria**
- The three M0 sample projects export correctly, at or above Recordly's baseline speed.
- Cancelling leaves no stray files.
- Golden-frame tests cover every effect.

## Milestone 9: Captions

**Goal:** local captions that transcribe the right audio.

- [ ] **9.1** Run Whisper locally through `whisper-rs`. Add model download with a checksum, which Recordly lacks, plus delete and custom model (§12, §36).
- [ ] **9.2** Add an audio source selector that defaults to the mic when one exists. This fixes the wrong-track and no-audio failures (§36).
- [ ] **9.3** Implement segmentation (§36), tested with sample transcripts.
- [ ] **9.4** Implement line layout and animation (§36), with text drawn in wgpu through `cosmic-text` so preview and export match.
- [ ] **9.5** Build caption editing (§12). For inline editing, place a DOM text box over the caption's computed position.
- [ ] **9.6** Export SRT and VTT mapped through trims and speed changes, and respect the Show switch at export (§58). Report any write failure to the user.

**Exit criteria**
- A mic-and-system recording produces captions of the voice, not the system audio.
- Caption files stay in sync after cuts and speed changes.

## Milestone 10: macOS beta

**Goal:** something you could give to another person.

- [ ] **10.1** Build the settings surface (§22).
- [ ] **10.2** Add the macOS menu (§49). Remove Toggle DevTools from production builds.
- [ ] **10.3** Add translations for the 10 languages (§22). Reuse of Recordly's text depends on D1.
- [ ] **10.4** Do an accessibility pass: labels, keyboard access to the launch bar, reduced motion (§52). Rust/UI's RTL support may also close that gap.
- [ ] **10.5** Set the webview security policy, allow only the IPC the app needs, and scope file access (§50).
- [ ] **10.6** Confirm network use is limited to what the user asks for, plus the update check (§51).
- [ ] **10.7** Add diagnostics: recording and export logs, with pruning (§55).
- [ ] **10.8** Generate the app icon set from `assets/logo.svg`, with padding, according to D9.
- [ ] **10.9** Package a DMG for x64 and arm64, with hardened runtime, signing and notarisation (§53). Check the entitlements needed for the transparent window, and that wry's private `drawsBackground` setting doesn't get in the way of notarisation.
- [ ] **10.10** Add auto-update with stable and experimental channels (§24).

**Exit criteria**
- A notarised build installs and runs on a clean Mac with no developer tools.
- Part 1 of the feature list is audited for macOS, with each item marked done, changed or dropped.
- An update installs from one beta to the next.

## Milestone 11: Windows

**Goal:** the same product on Windows.

- [ ] **11.1** Re-run T1, T3 and T5 on real Windows hardware, including high-DPI and multiple monitors.
- [ ] **11.2** Capture with Windows Graphics Capture to H.264 through a hardware encoder (§27).
- [ ] **11.3** Capture system audio through WASAPI loopback, with the timing repair in §27, and capture the mic natively.
- [ ] **11.4** Add native mic echo cancellation and noise suppression, replacing what Chromium provided (§27, §56).
- [ ] **11.5** Track cursor shapes and clicks (§28), including the closed-hand gap and reliably hiding the real cursor (§58).
- [ ] **11.6** Handle a closed source window, which Recordly doesn't (§6).
- [ ] **11.7** Replace the macOS menu with in-app access to everything it offered (§49).
- [ ] **11.8** Build an installer, with code signing and WebView2 runtime handling (§53).

**Exit criteria**
- The M4 recording test and the M8 export tests pass on Windows.
- Captions work on native recordings.
- Save works from the keyboard.

## Milestone 12: Linux

**Goal:** the same product on Linux, within what the platform allows.

- [ ] **12.1** Apply the T1 result and D4 for Wayland and X11.
- [ ] **12.2** Capture through the XDG desktop portal and PipeWire (§27).
- [ ] **12.3** Capture system audio and mic through PipeWire, as separate tracks where possible (§1, §4).
- [ ] **12.4** Do what's possible for cursor data on Wayland, and write down the limits (§28, §58).
- [ ] **12.5** Add the tray with Open, Quit, Show Controls and Stop Recording (§49).
- [ ] **12.6** Handle a second launch during recording without affecting it (§58).
- [ ] **12.7** Package an AppImage and test it on the WebKitGTK versions of current mainstream distros (§53).

**Exit criteria**
- The M4 recording test and the M8 export tests pass on one Wayland distro and one X11 setup.
- Known platform limits are listed in the README.

## Milestone 13: 1.0

**Goal:** a release you can stand behind.

- [ ] **13.1** Finish the Part 1 audit across all three OSes.
- [ ] **13.2** Run a test matrix: each OS, with and without a webcam, one and two monitors, light and dark, long recordings.
- [ ] **13.3** Automate releases: builds, smoke tests, checksums (§53).
- [ ] **13.4** Write the README, user docs and licence notices, including Rust/UI's MIT notice and FFmpeg's.
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
| Parity | Compare with the M0 baseline, and audit against Part 1 | M7 |

## Main risks

| Risk | Mitigation |
|---|---|
| Webview-over-wgpu layering fails on a platform | Tested first in T1, with fallbacks listed |
| Rust/UI is early stage and experimental | Vendored at a pinned commit, so upstream changes can't break the app |
| Rust/UI's desktop behaviour is lightly tested upstream | T5 audit, with changes scheduled in M5 and M6 |
| Dioxus 0.8 brings breaking changes | Pin an exact version, and upgrade deliberately after 1.0 |
| Mic quality drops without Chromium's audio processing | Evaluate `webrtc-audio-processing` during M4 and M11 |
| GPU and driver variety on Windows and Linux | Test on real hardware in M11 and M12, and offer a software encoder fallback |
| The licence decision changes late | D1 is settled in M0, before Part 2 is used |
| Scope: Recordly is about 105,000 lines | The "Not carrying over" list, and the macOS-first order |

## After 1.0

Candidates from §59 that this plan doesn't include:

- **Capture:** area recording, keystroke overlay, restarting a recording, toggling the mic while recording
- **Editing:** snapping, ripple delete, multi-select, copy and paste, J/K/L transport, row labels and locks, transitions, device frames, annotation animation, editable easing
- **Audio:** real loudness normalisation if not done in M7, fades, trimming into a file, noise removal
- **Captions:** active word highlighting, font choice, translation
- **Projects:** packing media into the project, project search, sort and delete
- **Export:** HEVC, AV1, ProRes, WebM, transparent video, batch export, render resume
- **Sharing:** upload and share links
