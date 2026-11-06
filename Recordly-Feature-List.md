# Recordly: Complete Feature List

- **Version reviewed:** 1.3.5-beta.2, commit `aaf7727a` (31 Aug 2026)
- **Repo:** `github.com/chrisknsmn/Recordly`, a fork of `webadderallorg/Recordly`
- **Reviewed:** 6 Nov 2026, from the local clone at `~/Projects/Recordly`
- **Stack:** Electron 43, React 18, TypeScript, PixiJS 8, with native helpers in Swift and C++
- **Size:** about 105,000 lines of non-test code

## How to read this

- **Part 1** lists everything a user can see and do.
- **Part 2** records the behaviour and numbers a rewrite needs to match, such as algorithms, file formats and output specs.
- **Part 3** lists what looks like a feature but isn't, likely bugs you shouldn't copy, and what the app doesn't do.

File paths are relative to the repo root.

**How far to trust it.** This comes from reading the code. Nothing was run, so Windows and Linux behaviour in particular has not been observed. About 35 of the most surprising claims were re-checked directly against the source. The two native GPU compositors (`gpu-export-probe`, `nvidia-cuda-compositor`) were only skimmed.

**Your local copy** has one uncommitted change: `VideoEditor.tsx` passes the real `borderRadius` to the preview instead of the hard-coded `0` upstream uses. This list describes behaviour with that fix.

## Before you reuse anything

- **Licence:** Recordly is AGPL-3.0. Copying its code into a new app generally brings the AGPL with it. Part 2 records constants and formulas so you can match behaviour, so decide how closely you want to follow them, and get proper advice if the licence matters for your product.
- **Assets:** the wallpaper and cursor names (`sonoma-*`, `ventura*`, `tahoe-*`, `sequoia-*`, `ipad-17-*`, and the `macos` and `tahoe` cursor sets) suggest Apple artwork. Their origin wasn't confirmed, so check the rights before shipping any of them.

## Where the docs and code disagree

| Docs, strings or code suggest | What actually happens |
|---|---|
| Extension system and marketplace (`EXTENSIONS.md`) | **Removed.** The Extensions tab only shows "Extensions are no longer available". |
| Add trim regions and speed regions | **Not reachable.** Replaced by splitting clips, dragging clip edges and per-clip speed. |
| Cursor smoothing and motion blur controls | **No sliders.** Smoothing comes from motion presets, and motion blur is forced to 0.6. |
| Export is MP4 and GIF with basic settings | Also has encoding mode, MP4 frame rate, pipeline choice, an NVIDIA option and caption files. |
| Text, image and figure annotations | Also has blur and censor annotations. |
| Nothing about captions | Full Whisper auto-captions with editing, burn-in and SRT/VTT files. |
| Zoom easing presets (in the code and project files) | Saved and passed around, but never read. One fixed curve is always used. |
| `.recordly` file association | Declared on macOS, but no handler loads the file. |
| "Drag and drop a video to start editing" (in the UI) | No file drag-and-drop exists anywhere. |
| Russian translation folder exists | Not enabled. The app offers 10 languages, and Russian isn't one of them. |

---

# Part 1: What the user can do

## 1. Platforms

| | macOS | Windows | Linux |
|---|---|---|---|
| Minimum version | 14.0 | 10 build 19041 for native capture; older builds fall back | Modern distro; system audio generally needs PipeWire |
| Capture engine | ScreenCaptureKit helper | Windows Graphics Capture helper, with a browser fallback | Chromium capture only |
| Source picker in the app | Yes | Yes | No; the system portal handles it |
| Real cursor hidden from video | Yes | Yes with native capture | Requested, but the README says it's unsupported |
| Cursor shape tracking | Yes | Yes, except closed hand | No, always arrow |
| System audio | Yes | Yes, WASAPI loopback | Through browser capture |
| Hide launch bar from recording | Yes | Yes | No |
| Floating webcam preview | Yes | Yes | No |
| Application menu | Yes | No | No |
| Tray icon | No | No | Yes |
| Package | DMG and ZIP (x64, arm64) | NSIS installer (x64) | AppImage (x64) |

## 2. Launch bar (HUD)

A floating, always-on-top bar at the bottom centre of the screen, with idle, recording and finalizing modes.

### Idle controls, left to right

| Control | What it does |
|---|---|
| Drag handle | Moves the bar |
| Source button | Opens the source picker and shows the selected name, scrolling if long. Default label "Screen". Hidden on Linux. |
| Audio popover | Toggle system audio; pick a microphone or turn it off. Each mic row has a live level meter. Shows "No microphones found" when empty. |
| Webcam popover | Pick a camera or turn it off; 96×96 mirrored live preview; show or hide the floating preview. Shows "No webcams found" when empty. |
| Countdown popover | No delay, 3, 5 or 10 seconds. Default 3. |
| Record | Starts recording. With no source selected (outside Linux), it opens the picker instead. Disabled during the countdown. |
| More popover | See below |
| Hide (−) | Minimises the bar. Return through the Dock, taskbar or tray. |
| Close (×) | Closes the bar. The app quits if no other windows remain. |

### More popover

- Hide HUD from recording, or show it (macOS and Windows; on by default)
- Recordings Path: choose the folder
- Open video file: webm, mp4, mov, avi, mkv, `.recordly`, `.openscreen`, all files
- Open project: grid of up to 24 projects with thumbnails and a "Current" badge
- Appearance: Light, Dark, System
- Language: 10 options
- App version as selectable text

### While recording

- REC label with a blinking red dot; PAUSED with an amber dot
- `MM:SS` timer that excludes paused time
- Pause and Resume
- Stop
- Hide the bar
- Cancel (×), which discards with **no confirmation**
- Mic icon shown but disabled, with the tooltip "Microphone cannot be toggled during recording"
- Source, audio, webcam, countdown, More and Close are all locked; the floating webcam preview stays draggable

### After stopping

The bar shows a spinner with "Preparing recording / Opening the editor in a moment", then the editor opens.

### Bar behaviour

- Popovers are 300 px wide, open upward, and only one is open at a time. Clicking away closes them.
- Dragging moves the bar within its window, so on macOS and Windows it can't move to another monitor.
- The bar's position isn't saved and is lost when its window reloads or the app restarts.
- Clicks on empty areas pass through to the apps behind it on macOS and Windows.

## 3. Choosing what to record

- Whole display or a single window. **No area capture.**
- A list with Screens and Windows sections, showing thumbnails and app icons
- Displays are named `Screen N (Primary)`, and windows `App — Title` on macOS
- Recordly's own windows are filtered out
- Choosing a source shows an animated purple border around it for about 1.7 seconds
- Choosing a window brings it to the front on macOS first. Linux code exists (`electron/ipc/register/sources.ts:351`) but looks unreachable: only the launch bar's Source popover triggers it, and that popover is hidden on Linux (`LaunchWindow.tsx:225`).
- The selection isn't saved between launches
- On Linux the system portal picks the source

## 4. Audio capture

- System audio on or off
- Microphone off, or a specific device
- Choices are saved between sessions
- The device list refreshes when devices change
- Separate system and mic tracks are kept where the platform allows it (see section 27)
- If system audio or the mic isn't available, recording continues without it and the user is told

## 5. Webcam capture

- Off by default and not saved between sessions
- Device picker, defaulting to the first camera
- Continuity Camera is enabled on macOS
- Records alongside the screen as a separate file, then links to it automatically in the editor
- Floating preview, also hidden from recordings by capture protection:
  - 288×288, mirrored, rounded, draggable
  - Shown while the webcam is enabled, both idle and while recording
  - Can be turned off from the webcam popover
- If the webcam fails, recording continues without it, with only a console message

## 6. Recording lifecycle

- **Countdown:** a large number in a floating card. Click anywhere or press Esc to cancel.
- **Pause and resume:** paused time is removed from the video timeline and the cursor data.
- **Stop:** the button on all platforms; also "Stop Recording" in the Linux tray.
- **No time limit.** Cursor data keeps only the most recent hour or so of samples (108,000).
- **Validation on stop:** the file must be at least 1,024 bytes, have a video stream, have a duration above zero, and decode one frame.
- **Window closed mid-recording (macOS):** recording finalises and the user is alerted, but the editor doesn't open and the file stays on disk.
- **Unexpected helper exit:** the user sees "Recording stopped unexpectedly". On macOS, the app tries to recover and open the editor. On Windows, it only shows a toast.
- **Not handled:** display disconnects, closed windows on Windows, and recovery after an app restart.

## 7. Editor layout

From top to bottom: a 44 px header, an announcement banner when there is one, then a row with the icon rail, the 332 px settings panel and the preview, then a full-width timeline. The timeline is 15% of the height with a 160 px minimum, and nothing is resizable.

- **Header, left:** Open Projects, Discord, Feedback, Undo, Redo
- **Header, centre:** project name, `.recordly`, and a blue dot for unsaved changes. Click to rename; Enter commits, Esc or clicking away cancels.
- **Header, right:** Presets, Export
- **Rail:** Scene, Cursor, Webcam, Captions, Settings, Extensions (placeholder), and a bottom Account button that only says "Account coming soon"
- **Above the preview:** aspect ratio menu and Crop button, with a dot when cropped
- **Below the preview:** Add Layer (Annotation, Audio), Add Zoom, Suggest Zooms, Split, transport controls, mute and volume
- **Contextual panels:** selecting a zoom, clip, audio item, caption or annotation on the timeline swaps the panel to its settings

**Aspect ratios:** Native, 16:9 (default), 9:16, 1:1, 4:3, 4:5, 16:10, 10:16.

All sliders are drag or arrow-key only, with no typed entry and no double-click reset. Each section's Reset returns to the values loaded when the editor opened, not factory defaults.

## 8. Scene panel

### Background

| Control | Options | Default |
|---|---|---|
| Blur | 0–8, step 0.25 | 0 |
| Image | 24 built-in wallpapers, plus custom uploads (JPG only, removable) | `tahoe-light.jpg` |
| Video | 1 built-in (`wispysky.mp4`), plus your own videos; tiles play on hover | — |
| Colour | 15 swatches plus a colour picker | — |
| Gradient | 24 CSS gradients | — |

**Colour swatches:** `#FF0000 #FFD700 #00FF00 #FFFFFF #0000FF #FF6B00 #9B59B6 #E91E63 #00BCD4 #FF5722 #8BC34A #FFC107 #2563EB #000000 #607D8B`

### Frame

| Control | Range | Default |
|---|---|---|
| Shadow | 0–1, step 0.01 | 0.67 |
| Radius | 0–200, step 0.5 | 12.5 |
| Padding (linked) | 0–100%, step 1 | 20 |
| Padding (Advanced, unlinked) | Top and bottom 0–250, left and right 0–100 | — |
| Remove Background | Switch: forces native aspect and zero padding; restores the old values when turned off | Off |

Relinking the padding averages the four values.

### Crop

- Top, Bottom, Left and Right insets, 0–50%, with a 5% minimum remaining size
- A crop dialog with four draggable edges on a live frame (10% minimum). Done keeps the crop; X or clicking outside reverts it.
- No aspect lock, and crop changes can't be undone

## 9. Cursor panel

The cursor is drawn as an overlay from recorded data, so it can be restyled after recording.

| Control | Range or options | Default |
|---|---|---|
| Show | On or off | On (off for browser-captured recordings) |
| Loop | On or off; returns the cursor to its start for seamless looping | Off |
| Style | macOS, Tahoe, Tahoe Inverted, Dot, Minimal | Tahoe |
| Size | 0.5–10, step 0.05 | 2.5 |
| Click Effect | Off, Spotlight, Ripple, Echo | Off |
| Effect colour (Advanced) | 8 swatches plus a picker | `#2563EB` |
| Effect size (Advanced) | 0.5–2, step 0.05 | 1 |
| Effect opacity (Advanced) | 0–1, step 0.01 | 1 |
| Effect duration (Advanced) | 120–1200 ms, step 10 | 600 |
| Click Bounce | 0–5, step 0.05 | 3.5 |
| Bounce Speed | 60–500 ms, step 5 | 350 |
| Sway | Shown as 0–1 (real value 0–2), step 0.025 | Real value 0.4 |

**Effect swatches:** `#2563EB #EF4444 #F59E0B #22C55E #A855F7 #EC4899 #14B8A6 #F97316`

Size, Bounce and Bounce Speed don't survive a reload unless they match a preset (see section 58).

## 10. Zooms

- **Add:** Z key, toolbar button, or the hover ghost on the zoom row
- **New zoom defaults:** 1.5×, auto mode, centred focus, 1,000 ms long
- **Placement rules:** must start inside a clip, must not overlap another zoom, and needs at least 1,000 ms before the next zoom or the clip's end
- **Depth:** 1.25×, 1.5×, 1.8×, 2.2×, 3.5×, 5×
- **Auto mode:** the camera follows the cursor, recentring when it nears the edge of the view
- **Manual mode:** a fixed focus. Pause, select the zoom, then click or drag on the preview; a focus rectangle shows the zoomed area.
- **Classic Animation:** a per-project switch that turns off the spring and cursor following
- **Suggest Zooms (wand):** adds zooms around clusters of clicks
- **Auto-apply on new recordings:** adds suggested zooms automatically when opening a new recording, unless the source is close to square or portrait (aspect below 1.2). On by default.
- **Connect Zooms:** a global switch that turns nearby zooms into one continuous camera move. On by default.
- **Select all:** Cmd/Ctrl+A selects all zooms, and Delete then removes them

## 11. Webcam overlay panel

| Control | Range or options | Default |
|---|---|---|
| Show | On or off | Follows the recording |
| React to zoom | Bubble shrinks as the camera zooms in | On |
| Mirror | On or off | On |
| Width and Height | 10–100%, step 1 | 40 |
| Crop | Inline live editor: move, 4 corners, 8% minimum, thirds grid, arrows 1%, Shift+arrows 5% | Full frame |
| Position | 3×3 preset grid | Bottom right |
| Custom position | Switch, then horizontal and vertical 0–100% | 100, 100 |
| Margin | 0–96 px | 24 |
| Roundness | 0–160 px | 90 |
| Shadow | 0–1 | 0.67 |
| Footage | Upload, Replace, Remove | Linked automatically |

The bubble can't be dragged on the preview, and there's no control for its time offset.

## 12. Captions

### Generating

- Local transcription with whisper.cpp; nothing leaves the machine
- Download Model (`ggml-small.bin` from Hugging Face), Delete Model, or Select Model to use your own `.bin`
- Language: auto, en, es, fr, de, it, pt, zh, ja, ko
- Generate or Regenerate, with progress; Clear Captions

### Styling

| Control | Range or options | Default |
|---|---|---|
| Show | On or off | Off, until captions are generated |
| Animation | Off, Fade, Rise, Pop | Fade |
| Hover to add on timeline | On or off | On |
| Text colour | Colour picker | `#FFFFFF` |
| Font size | 16–72 | 30 |
| Rows | 1–4 | 1 |
| Bottom offset | 0–30% | 3 |
| Max width | 40–95% | 62 |
| Box radius | 0–40, step 0.5 | 17.5 |
| Background opacity | 0–1 | 0.9 |

### Editing

- Click a caption in the preview to edit it inline. Enter commits, Shift+Enter adds a line, Esc cancels.
- Select a caption on the timeline to edit its text and its `m:ss.mmm` start and end times
- Split at the playhead (at the nearest word boundary), Merge with next, Delete
- Hover on the caption row and click to add one (up to 1,500 ms)
- Drag and resize on the timeline; a drop that would overlap is rejected

### Output

- Burned into the video
- Optional `.srt` and `.vtt` files next to an MP4 export

## 13. Annotations

Add with the A key or Add Layer. New annotations last 1,000 ms and appear centred at 30×20%.

| Type | Properties |
|---|---|
| Text | Multi-line text, font, size, bold, italic, underline, alignment, text colour, background colour |
| Image | JPG, PNG, GIF or WebP, embedded in the project |
| Arrow | 8 directions, stroke 1–6 (default 4), colour (default `#2563EB`) |
| Blur | Strength 1–100 (default 20), solid fill for censoring: None, Black, White or Custom |

- **Fonts:** Classic (system-ui), Editor (Georgia), Strong (Impact), Typewriter (Courier New), Deco (Brush Script MT), Simple (Arial), Modern (Verdana), Clean (Trebuchet MS), plus custom
- **Text sizes:** 12, 14, 16, 18, 20, 24, 28, 32 (default), 36, 40, 48, 56, 64, 72, 80, 96, 128
- **Text defaults:** white, bold, centred, transparent background
- **Colour picker:** 16 swatches plus hex
- **Custom fonts:** an "Add Google Font" dialog takes a Google Fonts URL. The font is load-tested with a 5-second timeout, and duplicates are rejected. There's no way to remove one.
- **On the preview:** click to select; drag and resize when selected. Positions are percentages of the recording area, and annotations move with zooms.
- **Layering:** by creation order, with the selected item on top. No reorder control.
- **Overlapping items:** Tab and Shift+Tab cycle through annotations at the playhead, or click the selected one to advance
- **Multiple tracks:** annotations can move between existing annotation rows and may overlap each other

## 14. Timeline

### Rows, top to bottom

1. Clip (always shown)
2. Source audio, one row per track, only when a clip has "Separate clip from audio" on
3. Zoom (always shown)
4. Captions (when captions are enabled or exist)
5. Annotation rows, only for tracks with items
6. Audio rows, only for tracks with items

Rows have no labels, lock or hide controls.

### Editing

- Drag item bodies to move them and edge caps to resize, with a tooltip showing start and end times
- Minimum item length is 100 ms
- The view scrolls automatically near the edges
- Single selection. Clicking an empty area clears the selection and seeks.
- **Overlap rules:**

| Item | Overlap | Moving |
|---|---|---|
| Annotation | Allowed | Can change annotation row |
| Audio | Not on the same row; clamps to neighbours | Can change audio row |
| Zoom | No; clamps to neighbours | Fixed row |
| Clip | No; clamps, or jumps past a neighbour once its centre crosses | Fixed row; can drag past the end to grow the timeline |
| Caption | No; the drop is rejected | Fixed row |

- No magnetic snapping between items

### Hover ghosts

- **Zoom row:** a 1,000 ms ghost, shown only where placement is valid. Click to add.
- **Caption row:** up to 1,500 ms, limited by the next caption or the end. Click to add and focus the text field.

### Keyframes

- F adds a yellow diamond at the playhead
- They act as markers only: the playhead snaps to them within ±150 ms when dragged, and the skip buttons jump to them
- They aren't saved and can't be undone

### Navigation

- Cmd/Ctrl+wheel zooms the view toward the pointer, down to a 300 ms visible range
- Shift+wheel, horizontal wheel or trackpad pans. A vertical wheel also pans if the rows don't overflow.
- The view resets whenever the total duration changes
- The playhead is a blue line with a diamond. Drag to scrub.
- The ruler targets about 12 labels, with minor ticks

### Waveforms

Shown on audio items and source audio rows, not on clips.

## 15. Clips

A clip marks part of the recording to keep, with a playback speed.

- A full-length clip is created automatically when a recording opens
- **Split:** C key or scissors button, at the playhead
- **Trim:** drag a clip's edges in or back out. Zooms, annotations, speed regions and audio regions overlapping the removed part are **deleted**, even if they overlap only partly.
- **Move:** drag a clip. Overlapping zooms shift with it. Annotations and audio don't, and any overlapping the vacated area are deleted.
- **Speed:** 0.25, 0.5, 0.75, 1, 1.25, 1.5, 2, 2.5, 3, 4, 5, 8, 10, 15, 20, 30×, with no custom value
  - Speed changes the clip's length. Later clips never shift.
  - Blocked with a message if it would overlap the next clip or rescaled zooms
  - Zooms that start inside the clip are rescaled. Annotations, audio and captions aren't.
- **Mute:** silences that clip's source audio
- **Separate clip from audio:** shows read-only waveform rows under the clip, but nothing is actually detached
- **Per-track volume:** one card per source track with Normalize, Volume 0–1 and Reset
- **Delete:** the panel button only; the keyboard doesn't delete clips
- **No ripple:** deleting leaves a gap, and playback skips it
- **No true reorder:** a clip has no separate source position, so moving it changes which footage it shows

## 16. Audio editing

- **Add audio:** Add Layer opens a file picker for mp3, wav, aac, m4a, flac, ogg or any file. The item lands at the playhead on a new track, limited to the file's length or the remaining time.
- **Per item:** Volume 0–1, Normalize, Reset, Delete
- **Normalize** is a fixed ×1.35 gain, not loudness analysis
- **Audio always plays from the start of its file.** Trimming the left edge moves the item rather than cutting into the audio.
- **Source tracks:** labelled Source System, Source Mic, or Source when mixed

## 17. Playback

- Current time, Skip Back, Play/Pause, Skip Forward, total time
- Skip jumps to the previous or next keyframe, otherwise ±5 seconds
- Mute toggles between 0 and 1, without remembering the previous level
- Preview volume 0–1, not saved
- No playback speed control, no loop and no fullscreen preview

## 18. Undo and redo

- Header buttons and keyboard shortcuts, with 100 levels
- **Tracked:** zoom, clip, speed, annotation and audio regions, captions, and the selected zoom, clip, annotation and audio item
- **Not tracked:** background, frame, crop, cursor and webcam settings, aspect ratio, caption styling, source track settings, keyframes, export settings
- History resets when a project loads or the source changes

## 19. Presets

The header's Presets menu saves the current look under a name, then applies or deletes it. Names are case-insensitive unique, and the active preset is detected by matching its values.

- **Includes:** background, frame, cursor and zoom motion, webcam settings, crop, aspect ratio, export settings, caption settings, Whisper paths
- **Excludes:** all timeline items

**Motion presets** are separate and live in Settings:

| Value | Focused (default) | Smooth |
|---|---|---|
| Zoom in / out | 200 / 200 ms | 1522.575 / 1015.05 ms |
| Cursor spring stiffness × | 1.35 | 0.92 |
| Cursor spring damping × | 0.79 | 1.36 |
| Shared | Size 2.5, smoothing 0.67, mass ×1.29, bounce 3.5, bounce speed 350 | Same |

## 20. Projects

- **File:** `<name>.recordly`, plain JSON that references media by absolute path. Media isn't copied.
- **Save** (Cmd+S, macOS only): writes to the current path with a thumbnail and a toast. With no path yet, it asks for a name.
- **Save As** (Cmd+Shift+S, macOS only): an in-app name dialog that always saves into the Projects folder, with no folder choice.
- **No manual save on Windows or Linux:** both commands exist only as macOS menu items, and those platforms have no app menu (`electron/main.ts:416-420`). There's no Save button or key handler either. Saving there happens only through autosave, once a project has a file, or by choosing Save in an unsaved-changes prompt.
- **Rename:** click the title. This replaces the file and removes the old file, thumbnail and backup.
- **Autosave:** silent, one second after a change, but only once the project has a file. New unsaved recordings don't autosave.
- **Backups:** the previous save is kept as `.recordly.bak`
- **Unsaved changes:**
  - Opening something else asks: Cancel, Discard changes, Save project
  - Closing the window asks: Save & Close, Discard & Close, Cancel
- **Name clashes:** a named save refuses to overwrite a different project
- **Project browser** (folder button, or Cmd+O on macOS):
  - A 280 px panel showing the newest 24 projects, with thumbnails and a "Current" badge
  - Has an Import button for opening other videos or projects
  - No search, sort, delete, rename or reveal
- **Library:** `<recordings folder>/Projects` plus up to 16 recent projects
- **Importing an existing video:** webm, mp4, mov, avi, mkv. A matching webcam file is linked automatically.
- **Legacy files:** `.openscreen` projects open, but need Save As to save
- **Missing video:** a "Project video file not found" message, with no way to relink

## 21. Export

The Export button opens a 360 px panel with settings, exporting, issue and complete states.

| Control | Options | Default |
|---|---|---|
| Format | MP4, GIF | MP4 |
| Quality (MP4) | Low (×0.6), Medium (×0.75), High (×0.9), Original (×1.0), each showing the final size | Original |
| Encoding (MP4) | Fast, Balanced, Quality | Balanced |
| Frame rate (MP4) | 24, 30, 60 | 30 |
| Pipeline (MP4) | Legacy, Lightning (Beta) | Lightning |
| NVIDIA acceleration (MP4) | Experimental switch; Windows with an NVIDIA GPU only | Off |
| Export captions file (MP4) | Writes `.srt` and `.vtt`; shown only when captions are enabled and exist | Off |
| Frame rate (GIF) | 15, 20, 25, 30 | 15 |
| Size (GIF) | Medium (720p height), Large (1080p), Original | Medium |
| Loop (GIF) | On or off | On |

**Saving**

- The file is named `export-<timestamp>`, and the save dialog opens in Downloads
- Cancelling the dialog keeps the render, so Save Again works without re-rendering
- Success shows a toast with Show in Folder

**Progress**

- A bar with a stage label: preparing, percent complete, rendering audio, muxing or finalizing, opening save dialog
- Render speed in frames per second
- The route used, and any reason native export was skipped
- A Cancel button

**Failure**

- An "Export issue" panel. Lightning errors include the reason, platform, route and suggested actions.
- No retry button, though a few specific decoder errors retry once automatically

**After export**

- Playback position is restored and the preview is rebuilt

## 22. App settings

Found in the editor's Settings tab unless noted.

- Appearance: Light, Dark, System (default), also in the launch bar
- Language: English, Spanish, French, German, Italian, Dutch, Korean, Portuguese (Brazil), Chinese Simplified, Chinese Traditional, also in the launch bar. Defaults to the system language.
- Experimental updates
- Auto-apply fresh recording zooms
- Connect neighbouring zooms
- Motion preset
- Keyboard shortcuts
- Recordings folder (launch bar only)
- Hide HUD from recording (launch bar only)
- Countdown (launch bar only)

## 23. Keyboard shortcuts

### Rebindable

| Action | Default |
|---|---|
| Add Zoom | Z |
| Split Clip | C |
| Add Annotation | A |
| Add Keyframe | F |
| Delete Selected | Cmd/Ctrl+D |
| Play / Pause | Space |

The rebind dialog works like this: click a shortcut and press the new keys. Conflicts offer Swap or Cancel, fixed keys are reserved, and there's Reset to defaults and Save.

### Fixed

| Action | Keys |
|---|---|
| Undo | Cmd/Ctrl+Z |
| Redo | Cmd/Ctrl+Shift+Z; Ctrl+Y on Windows and Linux |
| Delete selected | Delete or Backspace |
| Select all zooms | Cmd/Ctrl+A |
| Cycle annotations | Tab, Shift+Tab |
| Cancel or close | Esc |
| Open projects | Cmd+O (macOS menu only) |
| Save, Save As | Cmd+S, Cmd+Shift+S (macOS menu only) |
| Zoom timeline | Cmd/Ctrl+wheel |
| Pan timeline | Shift+wheel |

Timeline keys only work once the timeline has focus. There are **no global shortcuts**, so nothing starts or stops a recording from outside the app.

## 24. Updates

- **Checks:** at startup, every 6 hours, and when the channel changes. Manual check under Help on macOS only.
- **Channels:** stable, or experimental (prereleases)
- **Update window**, a floating 420×172 toast:
  - **Available:** Update now, Not now
  - **Downloading:** progress bar, percent, MB transferred, speed
  - **Ready:** Restart to update, Not now
  - **Error:** Update now retries, Not now
- **Not now** reminds you after 3 hours; the reminder doesn't survive a restart
- A manual check that finds nothing shows no message

## 25. Announcements

A remote feed that can show messages in the editor. Both the bundled and the remote feed are currently empty.

- **Formats:** a popup carousel with image or video, a toast, or a banner under the header
- **Targeting:** audience, start and end dates, minimum and maximum app version, priority, impression limit
- **Actions:** open an HTTPS link or jump to an editor section
- **Controls:** close, dismiss, action, navigation and indicators can each be turned off

## 26. Help and feedback

- Discord button
- Feedback dialog with email, X and "Report issue" links
- A "report bugs" link during Lightning exports
- A read-only shortcuts list in Settings, with Customize
- macOS Help menu: Check for Updates

---

# Part 2: Behaviour to match

## 27. Capture engines

### How the engine is chosen

- **macOS:** always the ScreenCaptureKit helper, with no fallback. A failure shows an error and stops.
- **Windows:**
  1. Use the WGC helper if the OS is build 19041 or later and `wgc-capture.exe` exists
  2. If unavailable, show a toast and use browser capture with the cursor hidden
  3. Otherwise start the helper and wait up to 12 seconds for `Recording started`
  4. On a start failure, show a warning and use browser capture with the cursor **visible**
- **Linux:** always browser capture, through `getDisplayMedia` and the portal, or `getUserMedia` with a desktop source.

### Video settings

| | macOS native | Windows native | Browser |
|---|---|---|---|
| Resolution | Points × backing scale, uncapped | Item size at start, floored to even, uncapped | Up to 3840×2160 |
| Frame rate | 60 (queue depth 6) | 60; drops frames arriving under 70% of the interval, repeats the last frame to fill gaps | Ideal and max 60, min 30 |
| Codec | `AVOutputSettingsAssistant` 4K preset with size overridden; assumed H.264 | H.264 through Media Foundation, hardware allowed | WebM preference: h264, vp9, plain, vp8, av1 |
| Bitrate | From the assistant | 4K+ 60.75, QHD+ 37.8, otherwise 24.3 Mbps | 4K+ 76.5, QHD+ 47.6, otherwise 30.6 Mbps |
| Pixels | BGRA captured | BGRA converted to NV12 on the CPU | Encoder default |
| Colour | Tagged BT.709 | BT.709 limited range (see below) | Not tagged |
| Container | `.mp4` | `.mp4` | `.webm`, 250 ms chunks, duration repaired |

**Windows colour conversion** (`electron/native/common/bt709_video.h:34,59`): `Y = ((47R + 157G + 16B + 128) >> 8) + 16`; Cb coefficients −26, −87, 113; Cr coefficients 112, −102, −10; chroma averaged over 2×2 pixels.

**Other details**

- macOS keeps only complete frames and adds a tail frame on stop, waiting up to 1 second for the queue. A single window's shadow is ignored.
- On Windows, a resized window keeps the original output size; content is cropped or padded with black, not scaled.
- The source list has a 1,200 ms cache. On macOS, windows come from a helper with a 5-second cache and are filtered: layer 0, at least 50×50, excluding system items such as the Dock and Control Centre.

### Audio by platform

**macOS native** (AAC, 48 kHz stereo, the app's own audio excluded):

| Toggles | Audio inside the MP4 | Separate files |
|---|---|---|
| System only | System, 192 kbps | `.system.m4a`, 160 kbps |
| Mic only | Mic, 192 kbps | None |
| Both | System only, 192 kbps | `.system.m4a` 160 kbps, `.mic.m4a` 128 kbps |

- Mic capture uses private keys (`captureMicrophone`, `microphoneCaptureDeviceID`, output type 2), checked at runtime. If unsupported, it falls back to the browser mic.
- All tracks treat the first video frame as time zero. Earlier audio is dropped, and late audio gets leading silence.

**Windows native:**

- System audio is WASAPI loopback of the default output, shared mode, 20 ms buffer, written as 16-bit WAV at the device rate. There's no device choice.
- The mic defaults to the browser path. Native mic needs `RECORDLY_WINDOWS_MIC_CAPTURE`.
- Timing repair (`wgc-capture/src/wasapi_loopback.cpp:13-18`):
  - Fills silence when more than 50 ms is missing (140 ms if flagged as a discontinuity)
  - Compacts smaller discontinuity gaps
  - Drops silent discontinuity packets of 40 ms or less
  - Applies a 5 ms fade-in after a gap or resume
- A `.wav.json` file records start delay, duration, sample rate, channels and repair counts.

**Browser mic file** (Windows default):

- Captured as Opus at 128 kbps, then converted to mono 48 kHz 16-bit `.mic.wav` by FFmpeg
- Default filter chain: `adeclip=threshold=1,adeclick=w=40:o=75:t=3:b=2,highpass=f=85,lowpass=f=9500,afftdn=nr=10:nf=-45:tn=1,alimiter=limit=0.92:level=0`, followed by `aresample=async=1:first_pts=0`
- `no-agc` profile replaces the limiter with `speechnorm=p=0.92:e=12:c=2:r=0.0005:f=0.001,alimiter=limit=0.92:level=0`
- Mic profiles: `processed` (default; echo cancel, noise suppression and AGC on), `no-agc`, `no-echo`, `no-noise-suppression`, `raw`

**Browser capture of everything** (Linux, Windows fallback): one mixed track. System plus mic mixes at 48 kHz with mic gain 1.4, at 192 kbps. System alone is 192 kbps; mic alone is 128 kbps.

**Mic device choice:** preferred, then current, then the first non-default, then the first, then default.

**Level meter:** FFT size 256, smoothing 0.8, `level = min(100, rms/255 × 200)`. Five bars at thresholds 10, 25, 45, 65 and 85, coloured blue, then yellow, then red.

### Webcam

- `getUserMedia` ideal 1280×720, ideal and max 30 fps, no audio, 8 Mbps
- MIME preference: mp4 `avc1.42E01E`, avc1, h264, plain mp4, then webm h264, vp9, plain, vp8
- `timeOffsetMs = webcamStart − mainStart`, saved in the session file
- Pauses and resumes with the main recording
- Floating preview: separate 320×320 stream at ideal 24 fps, 6 px drag threshold

### Countdown and start

With native capture and a delay above zero, the app warm-starts: launch the helper, wait for the first frame, pause, count down, resume. This removes the startup lag from the video. Browser capture counts down first, then acquires the stream.

### Stop

The app sends `stop` and waits for the output path. Windows has a 45-second timeout followed by a kill; macOS has no timeout. The editor opens immediately while webcam and mic files finish in the background.

## 28. Cursor data

- **Sampling:** a drift-corrected timer every 33 ms (about 30 Hz) in the main process, up to 108,000 samples
- **Fields:** `timeMs`, `cx`, `cy` (normalised 0–1), `interactionType` (`move`, `click`, `double-click`, `right-click`, `middle-click`, `mouseup`), `cursorType`
- **Normalised to:** window bounds (polled every 250 ms), otherwise the selected display, otherwise the nearest display
- **Double-click:** within 350 ms and normalised distance 0.04
- **Click capture:**
  - Windows and Linux use `uiohook-napi`
  - macOS deliberately skips uiohook, because `start()` can hang the main thread (`electron/ipc/cursor/interaction.ts:180`), and uses a listen-only `CGEventTap` in the cursor helper
- **Cursor types:** arrow, text, pointer, crosshair, open-hand, closed-hand, resize-ew, resize-ns, not-allowed
- **macOS type detection:** polls every 50 ms, comparing a 32×32 alpha signature of the current system cursor against known cursors. Thresholds: default 12,000, text 28,000, crosshair 32,000, hands 3,200. Accessibility heuristics run only if that fails. (`electron/native/NativeCursorMonitor.swift`)
- **Windows type detection:** polls `GetCursorInfo` every 50 ms and maps handles. `IDC_SIZEALL` becomes open-hand and wait becomes arrow; closed-hand never appears.
- **Linux:** always arrow; uses hook mouse moves when under 1,000 ms old
- **Hiding the real cursor:** `showsCursor=false` on macOS, `IsCursorCaptureEnabled(false)` on Windows, `cursor:"never"` and `googCaptureCursor:false` in the browser path. The Windows browser path also tries `ShowCursor` through Python or PowerShell.
- **Keystrokes are not captured.** No keyboard hook exists.

## 29. Files on disk

Default recordings folder: `<userData>/recordings`.

| File | Written by |
|---|---|
| `recording-<ms>.mp4` or `.webm` | Native or browser capture |
| `recording-<ms>-webcam.mp4` or `.webm` | Webcam |
| `<video>.cursor.json` | Cursor data: `{version:2, samples:[...]}` |
| `.system.m4a`, `.mic.m4a` | macOS |
| `.system.wav` and `.json` | Windows |
| `.mic.wav` and `.json` | Browser mic or native Windows mic |
| `.mic.source.webm` | Only with an env flag |
| `.recordly-session.json` | Webcam link: `{version:2, videoFileName, webcamFileName, timeOffsetMs}` |
| `.recording-diagnostics.json` | Windows native and browser mic |
| `Projects/<name>.recordly` | Project |
| `<project>.recordly.preview.png` | 320×180 thumbnail |
| `<project>.recordly.bak` | Previous save |

**Automatic deletion** runs after each recording is finalised (`electron/ipc/recording/prune.ts`):

- Targets `recording-*.mp4`, `.mov` and `.webm` files, including webcam files
- Deletes anything older than **14 days** or beyond the newest **20**
- Spares the current video, the file just finalised, files with a sibling project file, and any video or webcam referenced by a project in `Projects/`
- Also removes the cursor file and `.system`/`.mic` audio
- Leaves `.wav.json`, session, diagnostics and `.mic.source.webm` files behind

## 30. Native helper programs

Each runs as a separate process, controlled over stdin and stdout, with config passed as JSON in argv.

**`recordly-screencapturekit-helper` (macOS)**

- Config: `fps`, `displayId` or `windowId`, `outputPath`, `capturesSystemAudio`, `capturesMicrophone`, `systemAudioOutputPath`, `microphoneOutputPath`, `microphoneDeviceId`, `microphoneLabel`
- stdin: `pause`, `resume`, `stop`
- stdout: `Recording started` (after the first frame), `Recording paused`, `Recording resumed`, `WINDOW_UNAVAILABLE`, `Recording stopped. Output path: …`
- Polls every 500 ms for the window disappearing

**`wgc-capture.exe` (Windows)**

- Config: `outputPath`, `fps`, `displayId`, `displayX/Y/W/H` or `windowHandle`, `captureSystemAudio`, `audioOutputPath`, `captureMic`, `micOutputPath`, `micDeviceName`
- Same stdin commands, and stops and finalises on EOF
- stdout also has `Audio path:` and `Mic path:`; stderr has `WARNING:` and `ERROR:`

**Others**

| Helper | Purpose |
|---|---|
| `recordly-window-list` (macOS) | Prints windows as JSON |
| `recordly-native-cursor-monitor` (macOS) | Prints `STATE:<type>` and `INTERACTION:<mousedown or mouseup>:<button>`; stdin `stop` |
| `cursor-monitor.exe` (Windows) | Prints `STATE:` only; stdin `stop` |
| `recordly-gpu-export.exe` (Windows) | Direct3D 11 export compositor |
| `recordly-nvidia-cuda-compositor.exe` (Windows) | NVIDIA export compositor |
| `whisper-cli` | Transcription; whisper.cpp v1.8.4 built from source |
| `ffmpeg`, `ffprobe` | Audio conversion, muxing and probing |

Also used when present: `xwininfo`, `wmctrl`, `xdotool`, `osascript`, `powershell.exe`, Python. The macOS and Linux `ffprobe-static` binaries are excluded from packages, so those builds rely on a system `ffprobe`.

## 31. Compositing and layout

### Draw order

- **Preview, back to front:**
  1. Background (CSS, with blur)
  2. Pixi canvas with a CSS shadow: `stage`, `cameraContainer`, then `videoEffectsContainer` holding the masked video, then the cursor layer
  3. DOM overlays: focus indicator, webcam, caption, annotations
- **Preview annotations** use the same camera transform, so they zoom with the video.
- **Legacy export:** background, video and cursor with shadow, webcam, annotations, captions.
- **Lightning export:** background, then shadow, video, cursor and annotations inside the camera, then webcam and captions outside the camera, so those two don't zoom.

These three orders disagree, so captions and annotations can stack differently between preview and export. Pick one order for the rewrite.

### Layout (`src/components/video-editor/videoPlayback/layoutUtils.ts:41-142`)

- `padFrac = pct / 100 × 0.2`, so the default of 20 gives 4% per side
- `scale = min(availW / croppedW, availH / croppedH)`
- Unlinked vertical padding: `offset = travel/2 + ((top − bottom)/250) × travel/2`, clamped to the available travel
- Corner radius in pixels: `r × min(w/1920, h/1080)`

### Corners (`src/lib/geometry/squircle.ts`)

A polygon approximating a superellipse, exponent 4.5, with 10 segments per corner. Each point is `c + sign(cos θ) · r · |cos θ|^(2/4.5)`, likewise for sin. Radius is clamped to `min(w,h)/2`, and a radius of 0.5 or less gives a plain rectangle.

### Shadow

Three black layers scaled by intensity `i`:

| Y offset | Blur | Alpha |
|---|---|---|
| `12i` | `48i` | `0.7i` |
| `4i` | `16i` | `0.5i` |
| `2i` | `8i` | `0.3i` |

Native routes differ: FFmpeg uses the same profile through gblur, Direct3D 11 uses an SDF at `intensity × 64`, and the NVIDIA route bakes a box blur. That's several different looks for one setting.

### Background

- Images use cover fit. The list also accepts hex colours, `linear-gradient`, `radial-gradient`, data URLs and video.
- Export blur is `backgroundBlur × 3` pixels, while preview uses the raw value, so they differ
- Preview extends the background by `ceil(2 × blur)` pixels to avoid a dark edge
- At export, linear gradients always run top to bottom, ignoring the angle
- Video wallpaper loops; time is `t % duration`, with playback rate correction (tolerance 0.02, window 1.5 s, max ±0.12) and a seek on jumps over 0.25 s or drift over 0.35 s

### Bundled assets

- **Wallpapers:** 39 files, of which 25 are offered (24 images and `wispysky.mp4`)
- **Unlisted (14):** bluerays, cherrypop, farmvalley, lemonade, luisdelrio, mountaintrees, wallpaper1, 2, 7, 9, 11, 12, 13, 15
- **Cursors:** `macos` and `tahoe` sets of 42 files each, with 9 used per set, plus `minimal-cursor.svg`
- **Fonts:** none bundled. The stack is `"SF Pro Text", "SF Pro Display", "Helvetica Neue", sans-serif`.

## 32. Zoom engine

### Transform (`videoPlayback/zoomTransform.ts:421-452`)

- `focusPx = mask.xy + focus × mask.wh`
- `scale = 1 + (z − 1) × p`
- `x = (stageCx − focusPx.x × z) × p`, likewise for y
- Applied to `cameraContainer`

### Strength curve (`videoPlayback/zoomRegionUtils.ts:37-72`)

- Time is shifted: `t' = t − 200`
- `leadIn = start + 1000 − 1522.575`
- `zoomInEnd = leadIn + zoomInDuration`
- `zoomOutStart = end − 500`; if the two cross, both move to their midpoint
- `leadOutEnd = zoomOutStart + zoomOutDuration`
- Curve is `cubicBezier(0.16, 1, 0.3, 1)` in, and `1 −` the same curve out (`mathUtils.ts:61`). The solver uses 8 Newton steps and 10 bisection steps.

### Spring on top (`VideoPlayback.tsx:2099-2133`, `motionSmoothing.ts:281-319`)

- Scale, x and y are each sprung toward their target; skipped when paused, seeking or in Classic mode
- `s = 2 × smoothness`, `k = 100/s`, `c = 21`, `m = s`, rest delta 0.0005, rest speed 0.015
- Camera multipliers default to ×1, ×1.13, ×1.12, clamped to 0.25–3, giving k 100, c 23.73, m 1.12, damping ratio about 1.12
- Solver is closed form for under, critical and over damping. `dt` is clamped to 1–80 ms, the first step snaps to target, and over-damped crossings clamp to the target with zero velocity (`motionSmoothing.ts:90-217`).
- With the default Focused preset (200 ms eases), the spring dominates how zooms feel.

### Connected zooms (`zoomRegionUtils.ts:78-167`)

- Applies when the gap between zooms is 1,350 ms or less, including negative gaps
- Handoff is at `current.end + 200`. The outgoing zoom holds full strength from `end − 300`, and the incoming zoom is immediately at full strength.
- There's no interpolation: the target jumps and **the spring creates the pan**. In Classic mode or when paused, it's a cut.

### Auto camera (`videoPlayback/cursorFollowCamera.ts:84-201`)

- `halfSpan = 1/(2z)`; safe zone inset is `2 × halfSpan × 0.25`
- When the cursor leaves the safe zone on an axis, focus on that axis jumps to the cursor (a full recentre), then clamps to `[1/(2z), 1 − 1/(2z)]`
- Reaching 0.99 strength latches; below that afterwards, focus freezes for the zoom out
- Below 0.01 strength it resets; seeking backward or pausing also resets
- Uses raw full-frame cursor coordinates, not crop-adjusted ones

### Motion blur (`zoomTransform.ts`)

- Only while playing; amount is fixed at the default 0.35
- `fpsScale = (1 / max(1/240, dt)) / 60`
- **Pan:** `velocity = centerDelta × amount × fpsScale × 4 × (maxDirectionalBlurPx / 41.8)`, kernel 13, `offset = −mag/32`, off below magnitude 0.01
- **Zoom:** `strength = |1 − diag/prevDiag| × amount × fpsScale × 4`, centre at the intersection of corner trajectories, max kernel 13
- Affects the video only, not the cursor

## 33. Auto-zoom suggestions

`src/components/video-editor/timeline/zoomSuggestionUtils.ts:359-439`

1. Clean the samples: drop non-finite values, sort, clamp
2. Keep **explicit clicks only**: click, double, right, middle
3. Group clicks with gaps of 2,500 ms or less
4. Focus on the strongest click in the group; the first wins ties
5. Region runs from `first − 500` to `last + 500`, clamped to the total duration
6. Skip anything overlapping an existing zoom
7. Add at depth 2 (1.5×), auto mode, not selected

**Click strength:** double-click 1500, text selection 1300, dropdown open 1200, text field click 1100, plain click 900.

**Click classification** (`:450-521`):

- A mouseup within 3 s, lasting at least 200 ms, with `dx > 0.03` and `dx > 1.8 dy`: selection
- Move samples from +100 to +2000 ms: fewer than 3, or max distance under 0.02: text field
- `netDy > 0.03` and `ΣDy > 1.5 ΣDx`: dropdown
- `ΣDx > 0.03` and `ΣDx > 1.8 ΣDy`: selection
- Otherwise: plain click

**Dwell detection** (`:173-217`) doesn't drive suggestions. It only infers cursor types for samples missing one: runs split at distance over 0.02, lasting 450–2600 ms.

**Fresh recordings** (`VideoEditor.tsx:3369-3389`, `3833-3898`): cursor data is retried 12 times at 350 ms intervals, then debounced 500 ms. Runs once per file, and only with no existing zooms.

## 34. Cursor engine

### Position

Linear interpolation between samples, then a spring.

### Spring from smoothing factor `f` (`motionSmoothing.ts:219-279`)

| Factor | Stiffness | Damping | Mass |
|---|---|---|---|
| 0 | 1000 (state snaps) | 100 | 1 |
| Up to 0.5, `n = f/0.5` | `(760 − 420n) × 1.12` | `34 + 24n` | `0.85 + 0.55n` |
| Above 0.5, `n = (f − 0.5)/1.5` | `(340 − 180n) × 1.12` | `58 + 22n` | `1.35 + 0.45n` |

Multipliers from the motion preset apply on top, clamped to 0.25–3. The Focused preset works out to about k 483, c 47.8, m 1.81, damping ratio 0.81. Smooth has a damping ratio of about 1.69.

### Rules

- Snaps when paused, seeking, or on a time jump over 100 ms
- No idle hiding
- Hidden when outside the crop; click effects still draw if the click is in frame
- Cursor type: the latest sample with a type, skipping click samples, defaulting to arrow

### Size

- `height = 28 × cursorSize × (mask.width/1920) × styleMult × bounce`
- The macOS style multiplier is about 1.717; others are 1
- The cursor sits inside the camera, so it scales with zoom and isn't clipped by the mask
- Alpha 0.95

### Shadow

A black duplicate at alpha 0.35, offset (0, 2), BlurFilter 3, quality 4. The Minimal style has none.

### Motion blur

Applied to the whole overlay, including click rings: `velocity = Δpx × 0.6 × fpsScale × 2`, kernel 13.

### Click bounce

`progress = 1 − age/duration`; `scale = max(0.72, 1 − sin(progress × π) × 0.08 × bounce)`.

### Sway (`videoPlayback/cursorSway.ts`)

`rot = clamp((dx + 0.65dy)/dist, −1, 1) × min(1, speed/1400) × (π/18) × sway × 3`, sprung with its own softer settings. A distance under 0.01 gives zero.

### Click effects (`videoPlayback/cursorRenderer.ts:825-893`)

Drawn at the click position, delayed by half the bounce duration. `p` runs from 1 to 0; `h` is cursor height before bounce; `base = max(12, h × 0.55 × scale)`; `sw = max(2, h × 0.08)`; `a = p × opacity`.

- **Ripple:** `r = max(0.5, (1 − p³) × h × 1.95 × scale)`, alpha `p³ × 0.6 × opacity`, stroke `max(1, 2p³)`
- **Spotlight:**
  - Glow radius `base + (1 − p) × h × scale`, stroke `max(1.25, 0.68sw)`, alpha `0.28a`
  - Inner radius `max(0.72base, 0.76glow)`, stroke `max(1.5, 0.75sw)`, alpha `0.5a`
- **Echo:**
  - Outer radius `base + (1 − p) × h × 1.22 × scale`, stroke `sw`, alpha `0.72a`
  - Inner radius `max(0.58base, 0.62outer)`, stroke `max(1.4, 0.72sw)`, alpha `0.42a`
  - Filled dot of radius `max(3, 0.18base)`, alpha `0.14a`

### Cursor assets

- **Loading:** SVG, then canvas, then alpha trim (shifting the hotspot), then PNG texture with mipmaps
- **Tahoe Inverted:** per-pixel RGB inversion of Tahoe
- **Dot:** a 112² canvas, radius 26, white fill, 10 px stroke `rgba(15,23,42,0.88)`, centre anchored
- **Minimal:** `minimal-cursor.svg`, hotspot (40, 22) px

**Hotspots, in percent** (`videoPlayback/uploadedCursorAssets.ts:57-77`):

| Type | macOS | Tahoe |
|---|---|---|
| arrow | 34, 24 | 14, 6 |
| text | 50, 50 | 50, 44 |
| pointer | 39, 26 | 40, 10 |
| crosshair | 50, 50 | 50, 50 |
| open-hand | 50, 50 | 55, 57 |
| closed-hand | 50, 50 | 50, 46 |
| resize-ew | 50, 50 | 50, 50 |
| resize-ns | 50, 50 | 50, 49 |
| not-allowed | 23, 0 | 23, 0 |

### Loop mode (`videoPlayback/cursorLoopTelemetry.ts:145`)

Used in preview and export:

1. Freeze window of up to 670 ms at the end
2. Real motion, up to the last sample moving more than 0.0015, is time-compressed into the rest
3. 21 samples return to the first position over 550 ms, easing out quint
4. 120 ms settle with 4 samples at the first position and starting cursor type

## 35. Webcam overlay engine

`src/components/video-editor/webcamOverlay.ts`, `VideoPlayback.tsx:785-906`

- **Size:** `pct × dim × (react ? 1/appliedScale : 1)`, clamped to `[56, max(56, dim − 2 × margin)]`, using `dim = min(containerW, containerH)` and the sprung zoom scale
- **Position:** `margin + (container − size − 2 × margin) × preset`
- **Shadow:** `drop-shadow(0 round(0.06s) round(0.22s) rgba(0,0,0,shadow))`, where `s` is the smaller dimension
- **Mirror:** `scaleX(-1)`
- **Crop:** normalised; x and y 0–0.99, width and height minimum 0.01, cover fit. If width% is close to height% and the crop isn't default, height% becomes `width% / cropAspect`.
- **Sync:** `target = t − offset/1000`, with a seek on jumps over 0.25 s or drift over 0.35 s while playing, or 0.01 s while paused
- **Audio:** muted in preview

## 36. Caption engine

### Transcription

- **Audio source:** the recording, then the linked webcam file. Only the first audio stream is used, extracted with `ffmpeg -map 0:a:0 -vn -ac 1 -ar 16000 -c:a pcm_s16le`. **Separate mic files aren't consulted, and there's no source selector.**
- **Native Windows recordings have no audio to transcribe:** that path moves system and mic audio to `.system.wav` and `.mic.wav` sidecars instead of muxing them into the MP4 (`electron/ipc/recording/windows.ts:211-240`). The webcam file is recorded with no audio too, so generation fails with "No audio was found to transcribe".
- **Whisper:** `-m -f -osrt -of -l -np -ojf`, 30-minute timeout, retrying without `-ojf` if it's unsupported
- **Executable search order:** chosen path, bundled, `WHISPER_CPP_PATH`, Homebrew, `which` or `where`
- **Model download:** `.download` temp file then rename, 5 redirects, 30-second timeout, progress events, no checksum
- **Parsing:** JSON with token offsets, falling back to SRT. Tokens without a leading space join the previous word.
- **Silence:** `silencedetect=noise=-30dB:d=0.5`

### Segmentation (`electron/ipc/captions/segment.ts`)

- Drop words wholly inside a silence of 1,500 ms or more
- Break on sentence ends `[.?!…。！？]`, ignoring abbreviations (mr, mrs, ms, dr, prof, sr, jr, st, vs, etc) and initials
- Also break on a gap of 700 ms or more, a long silence inside a gap, or a phrase reaching 12,000 ms
- Merge neighbours when both are under 800 ms, the gap is 400 ms or less, and the total is at most 2,500 ms and 80 characters
- Pad by up to 80 ms, limited to half the gap
- Without word timing: uses silence regions, then splits sentences by character share

### Layout (`src/components/video-editor/captionLayout.ts`, `captionStyle.ts`)

- `font = max(14, size × (W × maxW%) / (1920 × 0.62))`, weight 400, line height 1.32
- Padding `1.1f` and `0.78f`; radius `r × f/30`
- Line wrapping is dynamic programming over all words, with forced breaks at cue boundaries and newlines
- Cost: last line `slack² × 0.18`; others `slack² + (fullness < 0.72 ? (0.72 − fullness) × 26000 : 0)`
- Lines are split into pages of `maxRows`; gaps under 500 ms are bridged

### Animation

180 ms in and out, with `v = min(enter, exit)`, re-triggered per page.

| Style | Opacity | Translate Y | Scale |
|---|---|---|---|
| None | 1 | 0 | 1 |
| Fade | `0.3 + 0.7v` | 0 | 1 |
| Rise | `0.25 + 0.75v` | `18(1 − v)` | `0.985 + 0.015v` |
| Pop | `0.35 + 0.65v` | `8(1 − v)` | `0.94 + 0.06v` |

### Editing behaviour

- Edited text is redistributed across affected cues by duration, with even word timing
- Retiming rescales words, with at least 1 ms per word and no collision check against neighbours

### Caption files

UTF-8 `.srt` (numbered, `,mmm`) and `.vtt` (`WEBVTT` header, `.mmm`) beside the video, written best-effort.

## 37. Preview playback and sync

- **Renderer:** PixiJS, WebGL then WebGPU, with an 8-second timeout each. The canvas renderer is rejected. 60 fps cap, device pixel ratio, antialiased. If both fail, plain video plays with a notice and no overlays.
- **Master clock:** the `<video>` element, using `requestVideoFrameCallback` `mediaTime`, with `requestAnimationFrame` as a fallback
- **Decoding:** Chromium's video element; no WebCodecs in preview. It's kept 1 px and invisible, not hidden, so frames keep arriving.
- **Speed:** `playbackRate` with pitch preserved
- **Clip gaps:** derived trim regions; playing into one jumps to its end, and the seek snaps the springs
- **Added audio:** one `HTMLAudio` per item, seek on drift over 0.2 s, rate correction (tolerance 0.015, window 2 s, max ±0.08)
- **Companion tracks:** fixed rate to avoid flutter, seek on jumps over 0.25 s or drift over 0.9 s playing or 0.01 s paused. Start delay is recorded or inferred (0.025–0.5 s); `.mic.` files get zero delay.
- **Waveforms** (`audio/waveform/`):
  - Whole file decoded, with a worker computing max-abs per bin
  - `bins = clamp(max(2048, seconds × 500))`, up to 200,000
  - Scaled by the 99.5th percentile
  - In-memory LRU of 24 entries, no disk cache, drawn at about 30 fps
- **Recovery:** after exports and most saves, the whole preview is rebuilt by changing its React key
- **Timing difference:** preview springs use wall-clock time, while export uses content time

## 38. Export routing

**"Breeze"** is the UI's name for the FFmpeg-backed path. **"Lightning"** is the modern pipeline; **"Legacy"** is the older one.

### Lightning route choice (`src/lib/exporter/modernVideoExporter.ts:373-583`)

1. **macOS and Windows:** start a Breeze stream (requires even dimensions, `avc1.640034` Annex B support and an FFmpeg session). If that fails, use WebCodecs with mediabunny.
2. **Linux:** WebCodecs first. If the resolved path is software-only, try Breeze. If WebCodecs throws, try Breeze, otherwise fail.
3. **NVIDIA opt-in:** try the native static layout first, then Breeze, then WebCodecs.

One automatic retry happens with a fresh media source when the error contains `readavpacket`, `get_media_info`, `avfoundation`, `failed after 3 attempts`, or `pipeline failed`.

### Native route order, Windows (`electron/ipc/export/native-video.ts:3189-3845`)

1. NVIDIA compositor, if opted in with the wrapper and GPU present. Output is validated: at least 95% of frames, and duration within `min(2, max(0.5, 2%))`.
2. Direct3D 11 compositor on failure
3. FFmpeg static layout, only without webcam, zoom, timeline edits or crop

### Static layout is skipped for

`native-static-api-unavailable`, `odd-output-dimensions`, `unsupported-audio-mode`, `unsupported-background-video`, `unsupported-cursor-click-effect`, `native-timeline-requires-windows-gpu`, `unsupported-native-speed-timeline`, `unsupported-native-trim-timeline`, `native-zoom-requires-windows-gpu`, `unsupported-annotation-overlay`, `unsupported-caption-overlay`, `unsupported-webcam-source`, `unsupported-rectangular-webcam-overlay`, `invalid-crop-region`.

### Legacy route

In Electron, Legacy also tries Breeze first, then WebCodecs, despite its UI hint calling it the stable WebCodecs path (`src/lib/exporter/videoExporter.ts:483`).

### Feature support by route

| Feature | Lightning | Legacy | GIF | NVIDIA | Direct3D 11 | FFmpeg static |
|---|---|---|---|---|---|---|
| Image, colour, gradient background | Yes | Yes | Yes | Yes | Yes | Yes |
| Video wallpaper | Yes | Yes | Yes | No | No | No |
| Background blur | Yes | Yes | Yes | Yes | Yes | Images only |
| Radius and shadow | Yes | Yes | Yes | Yes | Yes | Yes |
| Crop | Yes | Yes | Yes | Yes | Yes | Fails |
| Zoom | Yes | Yes | Yes | Yes | Yes | Fails |
| Zoom motion blur | Yes | Yes | Yes | Dropped | Dropped | No |
| Trim and speed | Yes | Yes | Yes | Yes | Yes | Fails |
| Cursor | Yes | Yes | Yes | Yes | Yes | Dropped |
| Cursor spring, blur, sway | Yes | Yes | Yes | Dropped | Dropped | No |
| Click effects | Yes | **No** | Yes | Skips route | Skips route | Skips route |
| Click bounce | Yes | Yes | Yes | Yes | Yes | No |
| Webcam | Any shape | Yes | Yes | Square only | Square only | Fails |
| Annotations | Yes | Yes | Yes | Skips route | Skips route | Skips route |
| Burned-in captions | Yes | Yes | Yes | Skips route | Skips route | Skips route |
| Classic zoom | Yes | **No** | Yes | Baked in | Baked in | n/a |
| Audio | Yes | Yes | None | Shared mux | Shared mux | Shared mux |

"Dropped" means silently left out of the output. "Skips route" means the app avoids that route. "Fails" means the route throws.

The native route capability check is true only on Windows, with the wrapper present, an NVIDIA GPU reported, and the env var not set to `0`.

## 39. Export output specs

### Dimensions

- **Native aspect:** source × crop, floored to even, minimum 2
- **Other aspects:** fit inside long and short side bounds by orientation (crop ignored)
- The quality scale then applies, floored to even
- Sizes are checked with `VideoEncoder.isConfigSupported`, then binary-searched from 0.001 to 1.000 if unsupported, debounced 200 ms

### Bitrate (`src/lib/exporter/exportBitrate.ts`)

`max(2,000,000, round(base × sqrt(max(1, fps/30)) × modeMultiplier))`

| Pixels | Base, Original quality | Base, other qualities |
|---|---|---|
| Up to 720p | 8 Mbps | 5 Mbps |
| Up to 1080p | 12 Mbps | 8 Mbps |
| Between 1080p and 4K | Linear interpolation | Linear interpolation |
| 4K and above | 45 Mbps | 35 Mbps |

Mode multipliers: Fast 0.5, Balanced 0.8, Quality 1.0. Example: 1080p30 Original Balanced is 9.6 Mbps, and 60 fps multiplies that by 1.414.

### Routes

| Route | Mux | Video | Keyframes | Faststart |
|---|---|---|---|---|
| Lightning WebCodecs | mediabunny MP4 | H.264 ladder `avc1.640033`, `4d4033`, `420033`, `4d401f`, `42001f`; hardware then software | fps × 4, 3 or 2.5 s (Fast, Balanced, Quality) | No |
| Legacy WebCodecs | mediabunny MP4 | `avc1.640033`, hardware then software | Every 150 frames | No |
| Breeze stream | `-f h264 -r fps -i pipe:0 -an -c:v copy -movflags +faststart` | `avc1.640034` Annex B, prefer hardware | Every 300 frames | Yes |
| NVIDIA compositor | Raw H.264, then FFmpeg copy | NVENC High, no B-frames, VBR | fps × 2 | Through shared mux |
| Direct3D 11 | Media Foundation sink writer | H.264 from NV12 | Encoder default | Through shared mux |
| FFmpeg static | FFmpeg `h264_nvenc` | yuv420p, preset p1, VBR, BT.709 TV range | Default | Yes |

**Audio mux** (`buildNativeVideoAudioMuxArgs`, `native-video.ts:3990`): `-c:v copy`; AAC at 192 kbps, or copy for AAC, MP3 or ALAC when in `copy-source` mode; `-t` output duration, or `-shortest`; `+faststart`.

**Colour:** export canvas is sRGB, tagged BT.709 primaries, iec61966-2-1 transfer, rgb matrix, full range. Encoder metadata wins, otherwise BT.709 limited.

**Frame rate:** always constant at the selected rate.

## 40. Export frame rendering

- **Both renderers are live.** `modernFrameRenderer` serves Lightning. `frameRenderer` serves Legacy and every GIF.
- **Backend order:** Lightning uses WebGPU then WebGL; Legacy uses WebGL then WebGPU. 8-second init timeout.
- **No supersampling.**
- **Blur annotations in Lightning:** a frame with an active blur annotation hides the annotation and caption layers, renders, then composites them in Canvas2D.
- **Blur at export:** copies the region with `2 × blur` padding and draws it with `ctx.filter`, clipped by `roundRect` rather than the squircle, then fills `blurColor`. Preview uses CSS `backdrop-filter`.
- **Webcam at export:** a forward frame source, then media element fallback, with 0.08 s drift tolerance in Lightning.
- **Cursor atlas:** built for native routes only, with 9 cursor types; failure skips the route.
- **Failure fallbacks:** a background failure gives black; a missing native wallpaper gives white.

## 41. Export decoding and performance

- **Stack:** web-demuxer (WASM) and WebCodecs `VideoDecoder` in the renderer; mediabunny for muxing only. AV1 prefers software decode.
- **Single forward pass** from 0 to `max(ends) + 0.5 s`; trimmed frames are decoded and thrown away
- **Time mapping:** output frame `i` maps to `start + (i/count) × duration`, with `count = ceil(duration/speed × fps)`. The nearest frame is held, handing off at midpoints, which handles variable frame rates and gaps.
- **Early end:** fails if the stream ends more than 1 second short and frames are missing

**Queue profiles** (`src/lib/exporter/exportTuning.ts`), measured against 720p60; heavy is 1.5× or more, extreme is 3× or more:

| Profile | When | Decode / pending / native writes |
|---|---|---|
| breeze-conservative | 4 cores or fewer, or extreme | 8 / 16 / 2 |
| breeze-balanced-plus | 8+ cores, not heavy | 14 / 40 / 8 |
| breeze-balanced | Otherwise | 12 / 28 / 4 |
| webcodecs-conservative | 4 cores or fewer, or extreme | 8 / 20 / 1 |
| webcodecs-balanced-plus | 8+ cores, not heavy | 12 / 32 / 1 |
| webcodecs-balanced | Otherwise | 10 / 24 / 1 |

**Other limits**

- Encode queue is `fps × 1.25`, `2` or `2.4`, clamped to 36–96, 72–120 or 96–180
- Breeze encoder queue 64; IPC batches at 12 chunks or 2 MiB
- Main-process stdin high-water mark 32 MiB, drain timeout 15 s
- First 1.25 s: decode at most 12, pending at most 28
- Preview rendering is suspended during Lightning export

## 42. Export audio

**Mode selection**

| Mode | Used when |
|---|---|
| `none` | No embedded audio, separate files or audio items |
| `edited-track` | Speed changes, audio items, more than one source, timed delay, non-default volume or normalize, or a muted clip |
| `trim-source` | Trims only |
| `copy-source` | Otherwise |

**Fast path** (`src/lib/exporter/editedTrackStrategy.ts`): a filtergraph of `atrim`, `asetpts`, `atempo`, concat, `apad`, `atrim=duration`, encoded as AAC 192 kbps with pitch preserved. Requires a primary source, known sample rate, all speeds within 0.5–2, and no audio items, extra sources or delay. Lightning also requires no volume, normalize or mute settings.

**Offline render** (`src/lib/exporter/audioEncoder.ts:698-1001`):

- 48 kHz, up to 2 channels, each source fully decoded to memory
- Rendered in 30-second `OfflineAudioContext` chunks
- Speed uses custom WSOLA: 40 ms Hann window, 50% hop, ±15 ms search, 100 ms padding
- Soft limiter: threshold 0.9, ceiling 0.985, tanh knee
- Output is 16-bit WAV to FFmpeg, or AAC 128 kbps on the mediabunny path

**Routing** (`src/lib/exporter/audioRoutingEngine.ts`):

- Tracks are classified by `.mic.` or `.system.` in the filename
- Embedded audio is included only without a system or mixed file
- Track gain is `volume × (normalize ? 1.35 : 1)`, clamped 0–2. Item gain is the same, clamped 0–1.
- Audio items are placed in output time at 1×, unstretched
- Companion delay is the recorded value, or a duration difference of 25–500 ms

## 43. GIF export

- **Encoder:** gif.js, quality 10, Floyd-Steinberg dither, black background, no transparency, per-frame palette
- **Workers:** `max(1, min(8, cores − 1))`
- **Size:** from video dimensions only, ignoring aspect ratio and crop. Height is capped at 720 or 1080, and odd values round up.
- **Timing:** delay `round(1000/fps)`, which the encoder rounds to centiseconds, giving effective rates of 14.3, 20, 25 and 33.3 fps
- **No limits:** no frame, duration or memory limits; all frames are copied into memory before encoding
- **Saving:** through a 16 MiB chunked temp stream
- No audio and no caption files

## 44. Export temp files, timeouts and cancel

**Temp files**

- Stream dirs are mode 0700 and files 0600, created exclusively, in the system temp folder
- Moving or discarding a temp file requires it to be in an owned-path registry and inside the temp folder

**Final save** (`electron/ipc/register/export.ts:71`): rename; on a cross-device error, copy to `.recordly-partial-*` next to the destination, then rename, with a backup swap on conflict.

**Disk space:** no preflight check or specific handling.

**Timeouts**

| Stage | Limit |
|---|---|
| Finalisation | 10 minutes |
| Audio workload | 10 min + 0.5 s per output second, up to 45 min |
| Idle watchdog | 25% of total, clamped 90 s to 5 min |
| FFmpeg audio mux | 15 minutes |
| Static FFmpeg | 15 minutes each; background 2 minutes |
| Source proxy | `max(5 min, 2 × source)` |
| NVIDIA | `max(20 min, 2 × duration)`, 120 s stall |
| Direct3D 11 | `max(15 min, 1 × duration)`, 120 s stall |
| Metadata probe, ffprobe | 30 s, 120 s |

**Cancel:** sets flags, cancels decoders and audio, closes the encoder, sends SIGKILL to FFmpeg or the compositor, and aborts streams. Quitting kills all sessions.

## 45. Project file format

Pretty-printed JSON; a byte order mark is tolerated.

```text
{
  version: 1,
  projectId?: uuid,        // assigned on first save, regenerated on copy
  videoPath: absolute path,
  editor: { ... }
}
```

**`editor` fields**

- **Look:** `wallpaper` (bundled path, hex, CSS gradient, data URL or video path), `shadowIntensity`, `backgroundBlur`, `borderRadius`, `padding{top,bottom,left,right,linked}`, `cropRegion{x,y,width,height}`, `aspectRatio`
- **Zoom motion:** `zoomMotionBlur`, `zoomTemporalMotionBlur`, `zoomMotionBlurSampleCount`, `zoomMotionBlurShutterFraction`, `connectZooms`, `zoomInDurationMs`, `zoomInOverlapMs`, `zoomOutDurationMs`, `connectedZoomGapMs`, `connectedZoomDurationMs`, `zoomInEasing`, `zoomOutEasing`, `connectedZoomEasing`, `zoomSmoothness`, `zoomClassicMode`
- **Cursor:** `showCursor`, `loopCursor`, `cursorStyle`, `cursorSize`, `cursorSmoothing`, `cursorSpring{Stiffness,Damping,Mass}Multiplier`, `cameraSpring{Stiffness,Damping,Mass}Multiplier`, `cursorMotionBlur`, `cursorClickEffect`, `cursorClickEffectColor`, `cursorClickEffectScale`, `cursorClickEffectOpacity`, `cursorClickEffectDurationMs`, `cursorClickBounce`, `cursorClickBounceDuration`, `cursorSway`
- **Webcam:** `webcam{enabled, sourcePath, timeOffsetMs, mirror, cropRegion, corner, positionPreset, positionX, positionY, size, width, height, reactToZoom, cornerRadius, shadow, margin}`
- **Regions:**
  - `zoomRegions[{id, startMs, endMs, depth, focus{cx,cy}, mode}]`
  - `trimRegions[{id, startMs, endMs}]`
  - `clipRegions[{id, startMs, endMs, speed, muted, showSourceAudio}]`
  - `speedRegions[{id, startMs, endMs, speed}]`
  - `annotationRegions[{id, startMs, endMs, type, content, textContent, imageContent, position, size, style, zIndex, trackIndex, figureData, blurIntensity, blurColor}]`
  - `audioRegions[{id, startMs, endMs, audioPath, volume, normalize, trackIndex}]`
- **Captions:** `autoCaptions[{id, startMs, endMs, text, words?[{text, startMs, endMs, leadingSpace?}]}]`, `autoCaptionSettings`
- **Export:** `exportEncodingMode`, `exportBackendPreference`, `exportPipelineModel`, `exportQuality`, `mp4FrameRate`, `exportFormat`, `gifFrameRate`, `gifLoop`, `gifSizePreset`
- **Audio:** `sourceAudioTrackSettingsByClip`, `defaultSourceAudioTrackSettings`

**Load limits:** durations 60–4000, connected gap 0–5000, spring multipliers 0.25–3, sway 0–2, zoom motion blur 0–2, background blur 0–8, size 1–200%. Empty caption cues are dropped.

**Legacy conversions on load** (`version` is only type-checked):

- `.openscreen` extension accepted
- `motionBlurEnabled` becomes 0.35
- `showBlur` becomes `backgroundBlur` 2
- Numeric `padding` becomes the linked object
- `webcam.zoomScaleEffect` becomes `reactToZoom`
- `webcam.corner` becomes `positionPreset`
- `webcam.size` becomes width and height
- Cursor `mono` becomes `tahoe-inverted`
- Click effect `burst` becomes `echo`
- No `clipRegions` with `trimRegions`: clips are derived as the complement, and speed regions are cleared

**Saving atomically** (`electron/ipc/project/atomicSave.ts`):

1. Write an exclusive temp file `.recordly-project-<pid>-<uuid>.tmp` in the same folder and fsync
2. Copy the old file to a temp, fsync, and rename it to `.bak`
3. Rename the temp over the target, then fsync the folder

Each path has a write queue, and file mode is preserved. Names are stripped of `<>:"/\|?*`, control characters, and trailing dots and spaces. Writes are allowed only to `.recordly` paths that are current or inside Projects.

**Dirty tracking:** deep comparison with the last save, ignoring webcam `enabled`, `sourcePath` and `timeOffsetMs`. No baseline counts as dirty.

**Embedded versus referenced:** annotation images and custom image wallpapers are embedded as data URLs. Video, webcam, audio and video wallpapers are referenced by absolute path.

## 46. Where settings are stored

`<userData>` on macOS is `~/Library/Application Support/Recordly`.

| Store | Keys and defaults |
|---|---|
| `app-settings.json` | `recordly.theme` (system), `experimentalUpdatesEnabled` (false), `dismissedAnnouncementIds` (last 200), `announcementImpressionCounts`, `recordly.editor.preferences`, `recordly.editor.presets`, `recordly.export.experimentalNvidiaCuda` (false) |
| `hud-overlay-settings.json` | `hiddenFromCapture` (true) |
| `recordings-settings.json` | `microphoneEnabled` (false), `systemAudioEnabled` (false), `microphoneDeviceId`, `recordingsDir` |
| `countdown-settings.json` | `delay` (3) |
| `shortcuts.json` | The six rebindable actions |
| `recent-projects.json` | Up to 16 |
| localStorage | `recordly.locale`, `recordly_custom_fonts` (legacy `openscreen_custom_fonts`) |

**Other folders:** `<recordingsDir>/Projects/`, `wallpaper-thumbs/`, `whisper/ggml-small.bin`, `native-tools/`, `native-export-diagnostics/`, `updater.log`.

**Global versus per-project**

- **Global, saved continuously:** wallpaper, custom wallpapers, shadow, blur, radius, padding, zoom timings and motion blur, connect zooms, auto-apply, all cursor settings, camera springs, webcam settings, aspect, export settings, Whisper paths
- **Project only:** all regions and captions, source track settings, crop, caption settings, Classic mode, zoom smoothness
- **Both:** most global controls are also saved in the project; new sources inherit global values
- **Not saved anywhere:** keyframes, preview volume, active rail section, timeline view, caption file toggle, selected source, webcam on/off and device, bar position

## 47. App windows

| Window | Size and placement | Notes |
|---|---|---|
| Launch bar | Idle on macOS and Windows: the whole work area. Recording: 860×160 bottom centre, or 860×540 with the floating webcam. Linux: always 860×160. | Frameless, transparent, always on top. Hidden, not closed, when the editor opens. |
| Editor | macOS: 85% of the work area. Windows and Linux: full work area. Minimum 800×600. | Only one at a time. macOS uses inset traffic lights at 12,12. |
| Source selector | 620×420, centred | Frameless. Opens only after a macOS window-unavailable event; see section 58. |
| Source highlight | Source bounds + 6 px per side | Ignores the mouse. Conic sweep in `rgb(99,96,245)`, 3 px border with glow. Closes at 1,700 ms. |
| Countdown | 200×200, centred | On all workspaces and over fullscreen |
| Update toast | 420×172, bottom centre, 16 px margin | Over fullscreen. Hides the launch bar while shown. |

**Launch bar pass-through:** on macOS and Windows the window ignores the mouse but forwards movement. Hovering an interactive element makes it clickable, and an open popover makes the whole window clickable. Windows re-applies the flag after 50 ms on focus, show and highlight changes, to work around Windows 11 corruption of `WS_EX_TRANSPARENT`.

**Capture protection:** uses `setContentProtection`. On macOS this is the only thing hiding the bar, because the display filter excludes nothing.

**Design tokens:** brand `#2563eb`, record `#f43f5e`, active `#3d8bff`, paused `#fbbf24`, resume `#34d399`.

## 48. Startup and quit

**Startup**

1. Dev mode uses a separate `Recordly-dev` data folder
2. GPU switches, navigation hardening, single-instance lock (a second launch focuses the app)
3. Permission handlers
4. On macOS, prompts for camera and mic if not granted, on every launch
5. Dock, tray, menu, recordings folder, local servers, IPC
6. Launch bar and updater (or just the editor in test modes)
7. The bar checks permissions. On macOS, missing Screen Recording or Accessibility opens System Settings and alerts the user to quit and reopen.

**macOS permissions**

- Screen Recording and Accessibility are both required to start recording
- Denials from the helper show a dialog with Open System Settings and Cancel
- Windows and Linux report permissions as granted

**Closing the editor:** nothing brings the launch bar back. The app stays running with the bar hidden. You return through the Dock, the tray, or by launching again.

**Quit:** kills the Windows capture process, restores the cursor, cleans up export sessions, and clears update timers. There's **no warning when quitting during a recording**.

## 49. Menus, tray and Dock

**macOS menu**

- **Recordly:** About, Services, Hide, Hide Others, Show All, Quit
- **File:** Open Projects, Save Project, Save Project As
- **Edit:** Undo, Redo, Cut, Copy, Paste, Select All
- **View:** Reload, Force Reload, Toggle DevTools (also in production), Actual Size, Zoom In, Zoom Out, Toggle Full Screen
- **Window:** Minimize, Zoom, Bring All to Front
- **Help:** Check for Updates

**Windows and Linux** have no menu.

**Linux tray:** a 24×24 icon. Idle offers Open and Quit. Recording changes the icon and tooltip, and offers Show Controls and Stop Recording.

**Dock:** a custom icon, with no Dock menu or badge. Windows sets AppUserModelId `dev.recordly.app`.

## 50. Security model and local servers

- **Renderer server:** serves `dist` on `127.0.0.1:<random port>`, editor only, packaged builds only
- **Media server:** `/video?path=` on `127.0.0.1:<random port>` with Range support, needed because Chromium blocks `file://` video from an HTTP-origin page. Only approved paths are served. CORS is `*`, with no token or Host check.
- **Media allowlist:** mp4, webm, mov, mkv, avi, wav, mp3, ogg, png, jpg, jpeg
- **Navigation:** reload only. External http(s) links open in the browser, redirects are blocked, and `window.open` is denied.
- **Permissions:** only the launch bar's main frame gets media and display capture. HID, serial and USB are denied.
- **`webSecurity:false`** on the launch bar and editor windows
- **`read-local-file`** has no allowlist (`electron/ipc/register/assets.ts:115`)
- **GPU switches:**
  - All platforms: `ignore-gpu-blocklist`, `enable-unsafe-webgpu`, `enable-gpu-rasterization`
  - macOS: `use-angle=metal`
  - Windows: `use-angle=d3d11`
  - Linux: `use-gl=egl` unless on Wayland, VAAPI disabled
- **macOS entitlements:** allow-jit, unsigned-executable-memory, disable-library-validation, camera, audio-input

## 51. Network and privacy

**There's no telemetry, analytics or crash reporting.** "Cursor telemetry" means local JSON files.

| Contacted | When |
|---|---|
| GitHub Releases, `webadderall/Recordly` | Automatically, packaged builds only |
| `raw.githubusercontent.com/webadderallorg/Recordly/main/announcements.json` | Automatically when the editor loads; 5-second timeout, 1 MB limit, 6-hour cache |
| `huggingface.co` Whisper model | When the user downloads it |
| `fonts.googleapis.com` | Only for added fonts |
| GitHub issues, Discord, X, announcement links | When clicked, in the browser |

Your installed build strips the update feed, but it still fetches announcements from upstream unless `RECORDLY_ANNOUNCEMENTS_URL=off` is set.

## 52. Accessibility

- Recording controls have ARIA labels; idle buttons have only titles
- The update toast uses `aria-live="polite"`
- Dialogs have Radix keyboard handling
- `:focus-visible` shows a 2 px outline
- **Gaps:** the launch bar isn't focusable on macOS and Linux, so it has no keyboard access. No reduced-motion or high-contrast support. No right-to-left layout.

## 53. Build and distribution

| OS | Output | Signing |
|---|---|---|
| macOS x64, arm64 | `Recordly-${arch}.dmg` and `.zip` | Hardened runtime, Developer ID and notarisation through CI secrets |
| Windows x64 | `Recordly-windows-x64.exe` (NSIS) | Authenticode if secrets exist, otherwise unsigned with a warning |
| Linux x64 | `Recordly-linux-x64.AppImage` | None |

- **App ID:** `dev.recordly.app`
- **Project type on macOS:** `dev.recordly.app.project`, `application/x-recordly-project`
- **Update feeds:** `latest.yml`, `latest-linux.yml`, merged `latest-mac.yml`; prereleases use `beta*.yml`
- **Stores:** Homebrew tap, winget `Webadderall.Recordly`, AUR `recordly-bin` (external repo)
- **CI:**
  - `quality.yml`: types, lint, tests
  - `build.yml`: manual builds
  - `release.yml`: builds, smoke tests, checksums, attestations
  - `macos-release-candidate.yml`, `homebrew-tap.yml`, `winget-releaser.yml`
  - `auto-translate-zh.yml`: translates root Markdown files to Chinese
- **Scripts:** native helper builds, `build-whisper-runtime`, `native-helper-manifest`, `windows-cmake-generators`, `install-ffmpeg-static`, `normalize-electron-main-cjs`, `smoke-electron-main-cjs`, `smoke-packaged-binaries`, `verify-macos-distribution`, `write-release-checksums`, `create-release`, `i18n-check`, `benchmark-export-queues`

## 54. Environment variables

| Area | Variables |
|---|---|
| Updates | `RECORDLY_DISABLE_AUTO_UPDATES`, `RECORDLY_UPDATE_FEED_URL`, `RECORDLY_UPDATER_LOG_PATH`, `RECORDLY_DEV_PREVIEW_UPDATE`, `RECORDLY_DEV_PREVIEW_EXPERIMENTAL_UPDATE` |
| Announcements | `RECORDLY_ANNOUNCEMENTS_URL` (or `off`) |
| Dev | `VITE_DEV_SERVER_URL`, `RECORDLY_DEV_OPEN_RECORDING_INPUT`, `RECORDLY_DEV_OPEN_RECORDING_WEBCAM` |
| Test export | `RECORDLY_SMOKE_EXPORT`, plus `RECORDLY_SMOKE_EXPORT_` + `INPUT`, `OUTPUT`, `PROJECT`, `USE_NATIVE`, `ENCODING_MODE`, `QUALITY`, `FPS`, `SHADOW_INTENSITY`, `WEBCAM_INPUT`, `WEBCAM_SHADOW`, `WEBCAM_SIZE`, `PIPELINE`, `BACKEND`, `RENDER_BACKEND`, `MAX_ENCODE_QUEUE`, `MAX_DECODE_QUEUE`, `MAX_PENDING_FRAMES` |
| Audio | `RECORDLY_BROWSER_MIC_PROFILE`, `RECORDLY_WINDOWS_MIC_CAPTURE`, `RECORDLY_KEEP_RECORDING_AUDIO_SIDECARS` |
| NVIDIA export | `RECORDLY_EXPERIMENTAL_NVIDIA_CUDA_EXPORT`, `RECORDLY_NVIDIA_CUDA_ALLOW_AUDIO_EXPORT`, `RECORDLY_NVIDIA_CUDA_FORCE_VIDEO_ONLY`, `RECORDLY_NVIDIA_CUDA_AUTO_STALL_TIMEOUT_MS`, `RECORDLY_NVIDIA_CUDA_EXPORT_DIAGNOSTICS`, `RECORDLY_NVIDIA_CUDA_EXPORT_SCRIPT`, `RECORDLY_NVIDIA_CUDA_EXPORT_EXE`, `RECORDLY_NVIDIA_CUDA_NODE_EXE`, `RECORDLY_NVIDIA_CUDA_EXPORT_HIGH_PRIORITY`, `RECORDLY_NVIDIA_CUDA_PREWARM_MS`, `RECORDLY_NVIDIA_CUDA_SAMPLE_GPU` |
| Other export | `RECORDLY_NATIVE_EXPORT_DIAGNOSTICS`, `RECORDLY_NATIVE_GPU_STALL_TIMEOUT_MS`, `RECORDLY_WINDOWS_GPU_EXPORT_EXE`, `RECORDLY_FFMPEG_EXE`, `RECORDLY_FFPROBE_EXE`, `WHISPER_CPP_PATH` |
| Linux | `OZONE_PLATFORM`, `ELECTRON_OZONE_PLATFORM_HINT`, `XDG_SESSION_TYPE`, `WAYLAND_DISPLAY`, `DISPLAY` |
| Build | `RECORDLY_BENCH_EXPORT_*`, `RECORDLY_NVIDIA_VIDEO_CODEC_SDK_ROOT`, `PACKAGED_SMOKE_ARCH_TAGS`, `FFMPEG_STATIC_INSTALL_*`, `WHISPER_RUNTIME_ARCHS`, `WHISPER_RUNTIME_ALLOW_MISSING` |

There are no CLI arguments, deep links or protocol handlers.

## 55. Diagnostics and test harness

- **Recording diagnostics:** kept in memory and in `.recording-diagnostics.json`, with phases `stop`, `mux-start`, `mux-complete`, `mux-error`, `mic-sidecar`, media probes, and output truncated to 12,000 characters
- **Export metrics:** load, init, decode, render, encode wait, write and finalisation timings, plus backend, encoder and skip reasons; logged as `[VideoExporter] Final metrics`
- **Native export diagnostics:** on by default in `userData/native-export-diagnostics/`, with no pruning
- **Headless export test:** `RECORDLY_SMOKE_EXPORT=1` opens the editor, exports automatically and quits, writing `<output>.report.json` with `success`, `phase`, `format`, `pipelineModel`, `backendPreference`, `encodingMode`, `elapsedMs`, `outputPath`, `error`, `progressSamples` and `metrics`. This works without screen permission and is a useful way to compare your rewrite's output.
- **Benchmark:** `scripts/benchmark-export-queues.mjs`

## 56. Features that come from libraries

These are capabilities Electron and its dependencies provide today, which a Rust version has to replace or rebuild.

| Capability | Provided by |
|---|---|
| Transparent, click-through, always-on-top windows; content protection | Electron `BrowserWindow` |
| Screen capture on Linux and the Windows fallback; the Wayland portal; `MediaRecorder` | Chromium |
| Webcam and mic capture; echo cancellation, noise suppression, gain control | Chromium `getUserMedia` |
| Preview video decoding and audio with pitch-preserved speed | `HTMLVideoElement`, `HTMLAudioElement` |
| GPU scene graph, masks, blur filters | PixiJS 8, pixi-filters |
| Hardware H.264 encoding and video decoding | WebCodecs |
| Container demuxing | web-demuxer (WASM) |
| MP4 muxing | mediabunny |
| GIF encoding | gif.js with web workers |
| Audio decode and offline mixing | Web Audio `decodeAudioData`, `OfflineAudioContext` |
| Text layout and fonts, including Google Fonts | Chromium, Canvas2D, CSS |
| Background and annotation blur | CSS `blur()`, `backdrop-filter`, `ctx.filter` |
| Timeline drag and drop | dnd-timeline |
| On-canvas drag and resize | react-rnd |
| UI controls and colour pickers | shadcn/ui components (`src/components/ui`, `components.json`) over Radix UI, @uiw/react-color-block, Tailwind CSS 3 with tailwindcss-animate |
| Toast notifications | sonner |
| Icons | @phosphor-icons/react, plus one react-icons import. `components.json` names lucide as the icon library, but it isn't installed or imported. |
| Mouse hook | uiohook-napi |
| Audio conversion, muxing, probing | ffmpeg-static, ffprobe-static |
| Transcription | whisper.cpp |
| Auto-update | electron-updater |
| Packaging | electron-builder |

---

# Part 3: What isn't really there

## 57. Present in the code, but not a working feature

### Removed or replaced

- **Extensions:** the docs, tab and `ext:*` section type remain, but there's no host code
- **Trim regions:** rewritten from clip gaps and never editable, though still saved
- **Speed regions:** no UI, but old projects still apply them invisibly, taking precedence on overlap
- **Device frames:** `frameInsets` is always null
- **Fullscreen preview:** the code allows for it, but it's never called

### Saved but never used

- `zoomInEasing`, `zoomOutEasing`, `connectedZoomEasing`
- `zoomInOverlapMs` (a hard-coded 1,000 ms is used instead)
- `connectedZoomGapMs` (hard-coded 1,350 ms) and `connectedZoomDurationMs`
- `panResponsePerSecond`, `zoomResponsePerSecond`, `zoomSafeZoneRadiusPx`
- `exportBackendPreference` outside test mode
- `inactiveTextColor`, because active word highlighting is switched off (`getCaptionWordVisualState` is uniform)
- `useModernNativeStaticLayout`, `videoPadding`

### Disabled by a flag

- Temporal zoom motion blur (`TEMPORAL_ZOOM_MOTION_BLUR_ENABLED = false` in both renderers)
- Windows auto static-layout-first (`WINDOWS_AUTO_STATIC_LAYOUT_FIRST_ENABLED = false`)

### Fields with no UI

- Webcam time offset, annotation border radius, caption font family
- Custom aspect ratios (type, parsing and preferences exist; no input)
- Whisper executable picker (handler and prop exist; the panel never uses the prop)
- Skip version and dismiss for updates
- Camera and cursor spring sliders (dev builds only)

### Code nothing reaches

- **Export:**
  - The FFmpeg rawvideo path and encoder ladder (`h264_videotoolbox`, `h264_nvenc`, `h264_qsv`, `h264_amf`, `h264_mf`, `libx264`), because both exporters always send `h264-stream`
  - `nativeFrameCapture.ts`, `VideoFileDecoder`
- **Capture:**
  - The FFmpeg capture backend (`start-ffmpeg-recording`)
  - `electron/native/windows-capture` (the DXGI helper; source only)
  - `recordly-system-cursors`, which has no caller
- **UI components:** `TutorialHelp`, `KeyboardShortcutsHelp`, `PlaybackControls`, `TimelineToolbar`, `FormatSelector`, `GifOptionsPanel`
- **Live components in the same file:** `TutorialHelp.tsx` also holds `FeedbackDialog`, `DiscordLinkButton` and `KeyboardShortcutsDialog`, which *are* in use
- **Updater:** the check-error toast, "Try again" and the native dialogs
- **IPC with no handler:** `get-linux-window-system`, `set-cursor-scale`
- **Preload methods with no caller:** `clearCurrentVideoPath`, `getProjectsDirectory`, `getRecordedVideoPath`, `getSystemCursorAssets`, `loadProjectFile`, `onCursorStateChanged`, `openProjectsDirectory`, `openRecordingsFolder`, `startFfmpegRecording`, `stopFfmpegRecording`
- **Dependencies with no imports:** `capturekit`, `emoji-picker-react`, `react-resizable-panels`, `mp4box`

### Stale text

- "Add Trim (T)", "Add Speed (S)" and the trim tutorial strings
- "Star on GitHub"
- "Drag and drop a video to start editing"
- "Click music icon"
- The Russian locale folder (present, not enabled)
- `TRANSLATION_GUIDE.md`, which lists only 3 locales
- `recordly.rb`, stale at 1.1.14

## 58. Likely bugs: don't copy these

None of these were seen at runtime; they come from reading the code.

### Confirmed in the source by direct re-check

| Issue | Where | Effect |
|---|---|---|
| **macOS mic may be lost** when system audio is also on | `electron/ipc/recording/diagnostics.ts:526-527`; helper writes the mic separately at `ScreenCaptureKitRecorder.swift:93` | The editor gets only the video's audio, which holds system audio, so `.mic.m4a` appears unused. **Worth a quick real test.** |
| **Motion values snap to a preset** | `src/components/video-editor/projectPersistence.ts:891-930` | Custom Cursor Size, Bounce and Bounce Speed revert to Focused on reload |
| **Choosing a recordings folder wipes audio settings** | `electron/ipc/project/manager.ts:247-254` | Rewrites the settings file with only the folder |
| **Caption files aren't adjusted for edits** | `src/components/video-editor/VideoEditor.tsx:660-677` | `.srt` and `.vtt` drift out of sync after trims or speed changes |
| **Lightning burns in captions when Show is off** | Only `src/lib/exporter/captionRenderer.ts:26` checks `enabled` | Hidden captions can appear in exports |
| **Legacy MP4 has no click effects** | No `cursorClickEffect` in `src/lib/exporter/videoExporter.ts` | Preview shows them, the export doesn't |
| **GIF "loop off" plays twice** | `src/lib/exporter/gifExporter.ts:132` returns 1; gif.js uses −1 for no repeat | One extra loop |
| **Cancelling the countdown may hang** | `electron/ipc/register/settings.ts:226-273`: cancel clears the timer that resolves the promise | Record may stay disabled, and a warm-started helper may stay paused |
| **Source selector window would crash** | `src/App.tsx:69` has no provider; `SourceSelector.tsx:354` needs one | A blank window after a macOS window-unavailable event |
| **Email link in Feedback is blocked** | `electron/ipc/register/permissions.ts:9` allows only http(s); `TutorialHelp.tsx:128` sends `mailto:` | The button doesn't work |
| **`.recordly` double-click does nothing useful** | No `open-file` handler in `electron/` | The app opens, but not the project |
| **Closing the editor strands the app** | `electron/main.ts:768-775` | Nothing re-shows the launch bar |
| **Legacy isn't the WebCodecs path its hint claims** | `src/lib/exporter/videoExporter.ts:483` ignores the setting | It tries FFmpeg first |
| **Keyboard can't delete clips** | `onClipDelete` is passed to `SettingsPanel` but not the timeline | Delete does nothing on a clip |
| **No manual save on Windows or Linux** | `electron/main.ts:416-420` removes the app menu; `VideoEditor.tsx:3338` is the only trigger for Save | Ctrl+S does nothing, and a new recording can't be saved until an unsaved-changes prompt appears |
| **Captions fail on native Windows recordings** | `electron/ipc/recording/windows.ts:211-240` keeps audio in sidecars; `electron/ipc/captions/generate.ts:85-105` never reads them | "No audio was found to transcribe" |

### Found in the code review, not re-checked

**Timeline and editing**

- Splitting a clip at speed other than 1× misaligns the right half
- A plain click on a keyframe selects it, then clears it immediately
- Selecting a clip and then another item leaves the clip selected, so Delete does nothing
- The Suggest Zooms wand is disabled when auto-apply is off, and its message wrongly blames cursor looping
- Click times are source time, while zoom regions are stored as timeline time
- Selection changes use up undo steps, and every keystroke records one
- The auto-created clip is recorded, so Undo can empty the clip row

**Projects**

- A project reads as dirty and rewrites itself about a second after opening
- Projects opened at startup have padding, radius, aspect and export settings overwritten by global preferences
- Crop isn't reset when importing a new source
- An unreadable project file can make recording finalisation fail (`electron/ipc/recording/prune.ts:92`)

**Capture**

- Cancelling may leave files, or save a fragment and open the editor
- Webcam finalisation can take the cursor data meant for the main video
- Quitting mid-recording on macOS can orphan the helper and leave an MP4 with no index
- The updater can hide the launch bar during a recording
- `ShowCursor` from a short-lived child process probably doesn't hide the Windows cursor
- On Linux, the default result may show no cursor at all
- On Linux, a second launch during recording could kill the renderer that owns the recorders

**Export**

- Cancelling an export reports as an error, and a GIF cancelled during final encoding never resolves
- Muted clips compare clip time with output time, so they drift after trims
- Legacy ignores source volume and mute when speed edits use the fast path
- Export uses one track-settings object: the selected clip's, or the one at the playhead
- Native routes may freeze cursor and zoom data after trims or speed-ups (needs verifying)
- Lightning positions non-blur annotations before layout exists, so they can disagree with blur frames when padding is non-zero
- Whole WAV over IPC hits a 2 GiB limit near 3.1 hours
- A hard 15-minute FFmpeg kill can undercut the 45-minute audio allowance
- Caption file write failures are invisible to the user
- Empty temp folders are left behind

**App and distribution**

- Language and custom fonts live in per-origin localStorage, but the bar uses `file://` and the editor a random `http://` port, so choices don't carry across or may not persist
- The `homebrew-tap.yml` glob doesn't match the actual artifact names
- The `windows-linux` release scope can never succeed
- The HUD language menu shows a raw `de` because the label is missing

## 59. What Recordly doesn't do

Useful as a starting point for what "better" could mean.

**Capture**

- Area or region recording
- Global hotkeys
- Restarting a recording, or toggling the mic while recording
- Keystroke capture or a key overlay
- Handling of display disconnects
- Crash recovery across app restarts
- Disk space checks
- Cursor shape tracking on Linux

**Editing**

- Dragging files into the app
- Opening projects from the OS, deep links or a CLI
- Clip reordering with footage moving properly
- Ripple delete
- Snapping between items
- Multi-select, except selecting all zooms
- Copy and paste
- Arrow-key nudging, or J/K/L transport
- Row labels, lock or hide
- Transitions
- Playback speed, loop or fullscreen preview
- Dragging the webcam bubble on the preview
- Device frames
- Text animation for annotations
- Editable easing curves

**Audio**

- Real loudness normalisation
- Fades
- Trimming into an audio file
- Noise removal in the editor
- Choosing which track captions transcribe

**Captions**

- Active word highlighting
- Font choice
- Other model sizes without supplying your own file
- Translation

**Projects**

- Relinking missing media
- Packing media into the project
- Project search, sort or delete
- Choosing a folder in Save As

**Export**

- Any codec other than H.264 or GIF (no HEVC, AV1, ProRes or WebM)
- Transparent video
- Batch export
- Render resume or a retry button

**Sharing and accounts**

- Upload or share links
- Accounts (the button is a stub)
