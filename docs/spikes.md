# Go/no-go test results

Results for Milestone 1 of `BUILD_PLAN.md`. The test apps are in `spikes/`.

## T1: Layering

**So far T1 passes on macOS. Windows, X11 and Wayland haven't been run, so T1 stays open.**

Windows matters most, because the plan says to revisit the UI choice if T1 fails there.

| Platform | Result |
|---|---|
| macOS 26.6, Apple M4 Max, 1080p display at 75 Hz | Pass, in 3 runs out of 3 (6 Nov 2026) |
| Windows | Not run. The test app hasn't been compiled there either. |
| X11 | Not run, and needs a code change first. See [Linux](#linux). |
| Wayland | Not run, and not supported by this approach. See [Linux](#linux). |

### What ran

`spikes/t1-layering` plays a 1080p60 H.264 file in a wgpu surface, on its own thread, under a transparent Dioxus webview. The webview shows a header, side panel and timeline around a clear area, as the editor will, plus the vendored Rust/UI Popover and Dialog. Dioxus and wry are the versions Reel pins.

An `ffmpeg` process decodes the frames, which are uploaded as RGBA. Choosing the real decoder is T2's job.

The self-test captures the composited window from the window server and sends mouse events through AppKit, so it checks the pixels on screen and the real path a click takes. All 18 of its checks passed in each of the 3 runs.

### Smooth 1080p60 playback: pass

Over the 3 runs, 5 of 8,568 video frames were dropped and the decoder never fell behind. Presenting held 74.7 to 75.1 fps, which is this display's refresh rate.

| While the page was... | Video fps (lowest run) | Frames dropped (all runs) | Worst frame time, ms |
|---|---|---|---|
| Idle | 59.8 | 2 | 27.2 |
| Showing the popover | 59.9 | 0 | 16.7 |
| Showing the dialog and backdrop | 60.0 | 0 | 16.8 |
| Moving a playhead at 60 Hz | 59.9 | 0 | 17.0 |
| Re-styling 600 elements every frame | 59.9 | 0 | 14.8 |
| Having its main thread blocked for 100 ms, 10 times | 60.0 | 0 | 17.4 |
| Resizing with `set_inner_size` | 59.8 | 0 | 18.4 |
| Resizing through a live drag | 59.7 | 3 | 27.4 |

Frame times are measured when wgpu presents, not when a frame reaches the screen. Rendering on its own thread is what keeps the video independent of the UI, as the blocked-thread row shows.

### DOM controls over the video, receiving clicks: pass

![The Rust/UI popover over the playing video](spikes/t1-popover.png)

- **Clicks:** the popover trigger, a button inside the popover, the dialog trigger and the dialog's close button all took clicks through AppKit.
- **Clear area:** clicks on bare video reach the DOM too, which the drag handles in step 7.5 need. Clicking there also closes the popover.
- **Drawing order:** in captures, the popover and dialog panels hold still while the video around them moves.
- **Translucency:** the dialog's backdrop halves the video's brightness (×0.50), and the video stays live behind it.

### Alignment through resizing: pass, with one visible effect

Once a resize finishes, the video and the clear area match to the pixel at every size tested. While a window edge is being dragged, the moving edge can lag.

![A grey band beside the video, on the moving edge during a resize](spikes/t1-mid-resize.png)

This is the state of each edge across 661 captures from live drags, at up to 785 px/s:

| Edge | Exact | Video continues under the panel, which looks fine | Grey band beside the video |
|---|---|---|---|
| Left, top, bottom (not moving) | 100% | 0% | 0% |
| Right (moving) | 61% | 13% | 25%, typically 14 to 20 px and at worst 28 px |

Percentages don't add up to 100 because of rounding. After each live drag, the edges were exact in a capture taken 400 ms later. After resizing with `set_inner_size`, which allows a closer look, they lined up again within 22 ms.

My call is that this passes, because the band only lasts a few refreshes during a fast drag. It hasn't been looked at by a person yet, so see the hand check in `spikes/t1-layering/README.md`.

### What made resizing work

M6.2 should use these settings:

- **Create the surface in `Config::with_on_window`.** It runs before the webview exists, which puts the Metal layer underneath it.
- **Set the Metal layer's `contentsGravity` to top left, and give it a background colour.** By default, Core Animation stretches the old frame to fit the new size, which moves even the edges that aren't changing.
- **Use `desired_maximum_frame_latency: 1`.** It cost no frame rate here, and lets a new size reach the screen sooner.
- **Report the clear area's rectangle from a `ResizeObserver`.** Reports reached Rust about 2 ms after a resize in most cases, and 22 ms at worst.

From one live-drag run of each setting:

| Frame on the layer | Latency | Left edge exact | Right edge exact | Widest band |
|---|---|---|---|---|
| Stretched (default) | 2 | 12 of 221 | 63 of 221 | 23 px |
| Pinned top left | 2 | 232 of 232 | 25 of 232 | 45 px |
| Pinned top left | 1 | 230 of 230 | 146 of 230 | 23 px |

If the band looks bad by eye, M6.2 could try `presentsWithTransaction` on the layer, which wgpu honours. This test didn't try it.

### Not tested on macOS

- **A real mouse.** Every click and drag was synthesised. They do go through AppKit, and the drags do enter its live-resize loop.
- **Retina, or changing scale.** The one display here is 1080p at 1×, so M6.2's DPI work starts without evidence.
- **Other refresh rates.** Only 75 Hz was available.
- **macOS below 26.6.** The plan's minimum is macOS 14. T5 and step 5.2 cover the known WebKit differences.
- **Long sessions.** Each run lasts about a minute.

### Linux

This comes from reading the code. Nothing has been run on Linux.

- **Dioxus ignores `with_as_child_window` on Linux.** In `dioxus-desktop` 0.7.9, `webview.rs` only honours it on Windows, macOS, iOS and Android. Elsewhere it always packs the webview into the window's GTK box, so T1 as written can't work there without a patch.
- **wry's `build_as_child` only supports X11.** Its docs say it won't work on Wayland, and recommend `WebViewBuilderExtUnix::new_gtk` with a `gtk::Fixed` to support both.

Both feed into D4.

### Windows

Also from reading the code. wry's own wgpu example turns off `WS_CLIPCHILDREN` through winit. tao has no such setting and never sets that style, so no equivalent should be needed. Only running it will show whether WebView2 really does draw over the surface.

### Findings for later steps

- **The first click on an inactive window is lost (T3, M5).** AppKit only activates the window, and the DOM sees nothing. wry can change this with `with_accept_first_mouse`, but Dioxus doesn't expose it. The launch bar will probably want it.
- **Transparency uses a private API (10.9).** wry sets `drawsBackground` on the webview's configuration through key-value coding. That rules out the Mac App Store, which the plan doesn't target, so check it doesn't get in the way of notarisation.
- **Vendored Popover and Dialog work here.** Their inline scripts run, and CSS anchor positioning works on macOS 26.6. T5 still needs to test the rest.

## T2: Decode

**T2 passes on the reference Mac, and D5 is settled: Reel uses native APIs on macOS.** Windows and Linux choose their own decoder behind the same interface in M11 and M12, with an LGPL FFmpeg build as the default there.

`spikes/t2-decode` decodes 4K60 H.264 into wgpu textures, headless. It compares AVFoundation with FFmpeg's library using VideoToolbox. Both produce the same hardware-backed `CVPixelBuffer`, so the rest of the path is identical. Tested on the M4 Max, with three clips.

### Results

"Zero-copy" wraps the frame's IOSurface as a texture. "Upload" copies the planes with `write_texture`, which is the portable path.

| Clip | Decoder | Decode speed | Real-time 4K60 (10 s) | Seek, median | Seek, worst |
|---|---|---|---|---|---|
| 43 Mbps, keyframe every 1 s | AVFoundation | 663 fps | 0 dropped, 0 late | 99 ms | 132 ms |
| | FFmpeg + VideoToolbox | 152 fps | 0 dropped, 0 late | 236 ms | 430 ms |
| 35 Mbps, keyframe every 4 s | AVFoundation | 680 fps | 0 dropped, 0 late | 259 ms | 399 ms |
| | FFmpeg + VideoToolbox | 161 fps | 0 dropped, 0 late | 934 ms | 1,499 ms |
| 122 Mbps of noise, keyframe every 1 s | AVFoundation | 428 fps | 0 dropped, 0 late | 108 ms | 191 ms |
| | FFmpeg + VideoToolbox | 102 fps | 0 dropped, 0 late | 252 ms | 595 ms |

In real time at 4K60, on either decoder:

| Path | CPU | Work per frame, median | Work per frame, worst |
|---|---|---|---|
| Zero-copy | 0.09 cores | 1.5 ms | 6.4 ms |
| Upload | 0.23 cores | 4.3 ms | 9.5 ms |

- **Correctness:** all 240 seeks landed on the exact frame. Pixels that reached the GPU match a reference from the `ffmpeg` CLI at 52 to 55 dB on luma. The neighbouring frames score about 20 dB, so it is the right frame. Overall PSNR is 33 dB, because this saturated test pattern shows up differences in chroma upsampling. Every path scored the same.
- **Software decoding:** FFmpeg on one thread managed 118 to 131 fps on normal clips and 43 fps on the noise clip, so it's only a fallback.

### Why native on macOS

- **Speed:** about four times the decode speed and two to four times faster seeks, which is what scrubbing feels like.
- **Packaging:** nothing to bundle or sign, and no LGPL relinking duties. Homebrew's FFmpeg is a GPL build, so shipping FFmpeg would have needed a custom build anyway.
- **Encoding:** step 3.8 can use AVAssetWriter, which also writes the fragmented MP4 that step 4.4 needs.

### What this costs

- **WebM import needs another route on macOS.** AVFoundation doesn't read WebM, and the plan keeps WebM import. Leave it out until FFmpeg is added for that alone, or until someone asks for it.
- **Two or three decoders instead of one.** `reel-media` needs a decoder interface with a backend per OS, so Windows and Linux aren't shared with macOS.
- **Keyframe spacing matters.** Seeks scale with it, so step 4.3 should record with a keyframe every second or so.

### Not tested

- **Slower Macs.** Only the M4 Max was available, and the plan's reference is whichever Mac this is. Step 10.9 builds for x64 too, and Intel Macs may behave very differently.
- **HEVC, or other non-H.264 imports.**
- **Variable frame rate.** Screen recordings can be, and this test's clips aren't.
- **Rendering to a window.** T1 covered that with a simple uploader, and step 3.1 joins them up.

## T3: Launch bar window

**T3 passes on macOS, apart from two things it couldn't check here: a real drag between monitors, and ScreenCaptureKit.** The approach avoids needing a full-screen window with mouse forwarding.

`spikes/t3-launch-bar` is a frameless, transparent, always-on-top window of 640x72 that grows to 640x400 when a popover opens. All its checks passed.

| Criterion | Result | Evidence |
|---|---|---|
| No flicker on resize | Pass | In 599 captures across 12 grows and shrinks, the bar never moved and the window was never blank. Click to resized took 27 ms. |
| Clicks outside the bar reach other apps | Pass by design | The window only covers the bar, or the bar and an open popover, so there's nothing to forward. Not tested with a real click. |
| Stays out of its own recordings | Partly checked | With `setContentProtection`, `CGWindowListCreateImage` returns a blank image. ScreenCaptureKit needs permission, so step 4.3 has to confirm it. |
| Can be dragged between monitors | Not checked | This Mac has one display. The bar does draw after moving, and `window.drag()` is wired up, but it needs a hand check. |

### What made it work

Steps 5.1 to 5.3 should use all of these:

- **Don't resize the webview.** Pin it at its largest size, anchored to the window's bottom edge, and change only the window's height. Resizing it with the window made the bar jump by the full 328 px for a frame in about half of all transitions, and sometimes go blank. WebKit keeps old content at the top of the taller view until a new layout arrives. Pinned, the page never changes size.
- **Change the window with `setFrame`.** An NSWindow's origin is its bottom-left corner, so keeping the origin grows the window upward in one step.
- **Replace `acceptsFirstMouse:` on `WryWebView`.** Otherwise the first click on the bar is lost while another app is active, which T1 found. Dioxus doesn't expose wry's option, so the test app replaces the method at runtime. An upstream option would be better if Dioxus adds one.

### Things step 5 has to handle

- **Near the top of the screen,** AppKit pushes the window down rather than let it grow past the top, which moved the bar by 83 px in the first run. The bar needs to open its popover downward there, or stay clear of the edge.
- **A popover that is open makes the window's transparent areas clickable.** That's the behaviour wanted: while a popover is open, a click anywhere in the window should close it.

## T4: Timeline speed

**T4 passes on macOS: use DOM items.** They hold the display's full refresh rate with room to spare, so the timeline doesn't need a canvas or wgpu, and Rust/UI can be used all around it.

`spikes/t4-timeline` keeps every item's state in Rust, so each change crosses the webview boundary. Each load ran for 5 seconds on the 75 Hz display, with Rust ticking at 60 Hz. Mouse moves went through AppKit at about 85 per second.

| Load, with 500 items | Page paint rate | Missed refreshes | Latency, median | Latency, worst |
|---|---|---|---|---|
| DOM, playhead moving | 75.2 fps | 0 | 1 ms | 3 ms |
| DOM, dragging an item while playing | 75.0 fps | 0 | 2 ms | 6 ms |
| DOM, zoom moving all 500 while playing | 75.0 fps | 1 | 2 ms | 8 ms |
| Canvas, playhead moving | 75.0 fps | 0 | 0 ms | 4 ms |
| Canvas, dragging an item while playing | 75.0 fps | 0 | 0 ms | 1 ms |
| Canvas, zoom moving all 500 while playing | 75.2 fps | 0 | 0 ms | 4 ms |

Latency for a drag runs from the mouse move in JavaScript to the resulting change on the page. For the playhead, it runs from Rust's tick to the change on the page. Timers in WebKit only have about 1 ms of resolution.

**Headroom:** at 2,000 items, the DOM's worst case was zoom, at 119,600 item changes per second. It missed 5 refreshes in 5 seconds, with a worst paint of 21 ms and playhead lag of 15 ms. That's four times the plan's count, in the worst case, and still usable.

### Why it's this fast

Dioxus desktop sends events with a synchronous XHR. JavaScript waits while Rust handles the event, and DOM changes follow straight away, which explains a round trip of about a millisecond between processes.

This has a cost: **anything that blocks Rust's main thread freezes the page too**, because the page is waiting on it. Long work has to stay off that thread, which the plan already does for rendering and recording.

### Rust's clock

Pace ticks with `tokio::time::interval`, not `sleep`. A `sleep(16 ms)` loop only reached 50 ticks a second, and `interval` holds 60.

### Not tested

- **Real timeline items.** These have a text label each. Waveforms and handles will add weight, which is what the headroom is for.
- **Video playing at the same time.** T1 showed a busy page doesn't disturb wgpu, but not this page specifically.
- **Other webviews.** WebView2 and WebKitGTK may behave differently, and the synchronous XHR is one to watch.

## T5: Rust/UI on real webviews

**T5 has its list for WKWebView, on macOS 26.6. WebView2 and WebKitGTK haven't been run, so T5 stays open.** Nothing here threatens the plan: all 23 components render, in both themes, and most work as they should.

`spikes/t5-components` puts every vendored component on one page and drives it with real clicks, right-clicks and keystrokes through AppKit. Of 33 checks, 27 passed.

### What needs fixing

| Component | Problem | When |
|---|---|---|
| Popover | Leaves 2 `document` listeners behind every time it's unmounted (10 became 20 after 5 remounts). It does still work after a remount. | Step 5.2 |
| Popover | Depends on CSS anchor positioning, which older WebKit lacks. It works on macOS 26.6, but the plan's minimum is macOS 14. | Step 5.2 |
| Dialog, Alert Dialog | Tab moves focus out of the dialog. There's no focus trap. | Step 6.4 |
| Select | Arrow keys do nothing. | Step 6.4 |
| Dropdown Menu | Arrow keys do nothing. | Step 6.4 |
| Context Menu | `close_context_menu()` does nothing on desktop. **This is the only `wasm32`-only branch in any vendored component**, so nothing else silently does nothing. | Step 6.4 |
| Switch | Has no change handler, so Rust can't read or control its value. | Step 6.4 |
| Scroll Area | `ScrollBar` is decorative: an unpositioned bar that doesn't follow scrolling, drawn beside the real one. Leave it out, and rely on `ScrollAreaViewport`'s own scrollbar. | When used |
| Card | `CardHeader` uses `@container`, so a card can't size itself to its content and text overflows. Give cards a width. | When used |

The plan already had steps for most of these, and this adds the Switch, `ScrollBar` and Card items.

### What works

Button, Input, Label, Kbd, Slider, Tabs, Toggle Group, Accordion, Separator, Skeleton, Spinner, Progress and Toast all work. So do opening, choosing, clicking away and Escape on Select, Dropdown Menu, Context Menu, Popover, Dialog and Alert Dialog. A click on an Alert Dialog's backdrop leaves it open, as it should. There were no JavaScript errors.

### Not checked

- **Tooltip's hover.** A synthetic mouse move doesn't set `:hover` in WebKit. Its CSS rule is present, but it needs a hand check.
- **macOS 14.** Apart from anchor positioning, the CSS these components use should be fine in Safari 17, going by published browser support rather than a test: `color-mix`, `oklch`, `@property`, `@layer`, `:has` and `@container`. Only `scrollbar-width` is newer, and it's harmless if ignored.
- **Accessibility beyond keyboard focus,** which step 10.4 covers.

## Running test apps on macOS

These cost time during T1, and will apply to later tests too.

- **Keep a test app's files out of `~/Documents`.** This repo is under it, and macOS protects that folder. Each build is a new ad-hoc identity, so a file read from there shows a permission prompt after every rebuild and blocks until someone answers it. That looks like a slow start and a window that keeps losing focus. T1's `run.sh` works from `~/Library/Caches/reel-spikes/` and copies results back.
- **Rebuild the `.app` rather than overwriting its binary.** Launches failed with error 162, "Codesigning issue", on a bundle whose binary had been overwritten with `cp`. Deleting the bundle, copying the binary into a new one and signing it with `codesign --force --sign -` stopped that, though this didn't separate which part mattered. `launchctl error <n>` translates these codes.
- **An app can capture its own window without the Screen Recording permission.** `CGWindowListCreateImage` returns the real composite, including Metal layers and the webview, with no prompt. The SDK marks it obsolete from macOS 15, but it's still exported, so declare it by hand.
- **Posted mouse events can drive a real live resize.** Call `postEvent:atStart:` from another thread, because the main thread stays inside AppKit's resize loop for the whole drag.
- **Synthetic clicks can't take focus.** A click on a window that isn't key is lost, and macOS may refuse to give focus back to an app that gave it up, so T1's self-test flags such runs as inconclusive.
- **`open -W` always exits 0.** Read the result from a report file instead.
- **Check the screen isn't locked.** While it's locked, a window can't take focus, and captures come back blank at the wrong size, which can look like the app has broken. `ioreg -n Root -d1 | grep CGSSessionScreenIsLocked` tells you.
- **Keep test windows on top.** Otherwise anyone working on the Mac takes focus away and clicks are lost.
