//! A scripted test of the running app, built only with the `self-test` feature. It drives the real UI
//! with clicks through AppKit, measures playback, and compares the screen with a rendered still.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use dioxus::desktop::tao::platform::macos::WindowExtMacOS;
use dioxus::desktop::tao::window::Window;
use dioxus::prelude::*;
use objc2::MainThreadMarker;
use objc2_app_kit::{NSApplication, NSEvent, NSEventModifierFlags, NSEventType, NSWindow};
use objc2_foundation::{NSPoint, NSProcessInfo, NSRect, NSSize, NSString};
use serde_json::{Value, json};

use crate::preview::SHARED;

fn ns_window(window: &Window) -> &NSWindow {
    // SAFETY: tao returns the live NSWindow it owns, and `window` keeps it alive for this borrow.
    unsafe { &*window.ns_window().cast::<NSWindow>() }
}

fn make_key(window: &Window) {
    if let Some(main_thread) = MainThreadMarker::new() {
        #[allow(deprecated)]
        NSApplication::sharedApplication(main_thread).activateIgnoringOtherApps(true);
    }
    ns_window(window).makeKeyAndOrderFront(None);
}

fn send_mouse(window: &Window, x: f64, y: f64, down: bool) {
    let window = ns_window(window);
    let height = window
        .contentView()
        .map_or(0.0, |view| view.frame().size.height);
    let kind = if down {
        NSEventType::LeftMouseDown
    } else {
        NSEventType::LeftMouseUp
    };
    let event = NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(
        kind,
        NSPoint::new(x, height - y),
        NSEventModifierFlags::empty(),
        NSProcessInfo::processInfo().systemUptime(),
        window.windowNumber(),
        None,
        0,
        1,
        if down { 1.0 } else { 0.0 },
    )
    .expect("AppKit refused to build a mouse event");
    window.sendEvent(&event);
}

fn send_key(window: &Window, characters: &str, key_code: u16) {
    let window = ns_window(window);
    let text = NSString::from_str(characters);
    for kind in [NSEventType::KeyDown, NSEventType::KeyUp] {
        let event = NSEvent::keyEventWithType_location_modifierFlags_timestamp_windowNumber_context_characters_charactersIgnoringModifiers_isARepeat_keyCode(
            kind,
            NSPoint::new(0.0, 0.0),
            NSEventModifierFlags::empty(),
            NSProcessInfo::processInfo().systemUptime(),
            window.windowNumber(),
            None,
            &text,
            &text,
            false,
            key_code,
        )
        .expect("AppKit refused to build a key event");
        window.sendEvent(&event);
    }
}

const WINDOW_LIST_INCLUDING_WINDOW: u32 = 1 << 3;
const IMAGE_BOUNDS_IGNORE_FRAMING: u32 = 1 << 0;

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGWindowListCreateImage(
        bounds: NSRect,
        list: u32,
        window: u32,
        image: u32,
    ) -> *mut std::ffi::c_void;
    fn CGImageGetWidth(image: *mut std::ffi::c_void) -> usize;
    fn CGImageGetHeight(image: *mut std::ffi::c_void) -> usize;
    fn CGImageGetBytesPerRow(image: *mut std::ffi::c_void) -> usize;
    fn CGImageGetDataProvider(image: *mut std::ffi::c_void) -> *mut std::ffi::c_void;
    fn CGDataProviderCopyData(provider: *mut std::ffi::c_void) -> *const std::ffi::c_void;
    fn CGImageRelease(image: *mut std::ffi::c_void);
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFDataGetBytePtr(data: *const std::ffi::c_void) -> *const u8;
    fn CFDataGetLength(data: *const std::ffi::c_void) -> isize;
    fn CFRelease(object: *const std::ffi::c_void);
}

struct Capture {
    width: usize,
    height: usize,
    rgba: Vec<u8>,
}

impl Capture {
    fn pixel(&self, x: usize, y: usize) -> [u8; 3] {
        let at = (y * self.width + x) * 4;
        [self.rgba[at], self.rgba[at + 1], self.rgba[at + 2]]
    }

    fn save(&self, path: &std::path::Path) {
        if let Ok(file) = std::fs::File::create(path) {
            let mut encoder = png::Encoder::new(
                std::io::BufWriter::new(file),
                self.width as u32,
                self.height as u32,
            );
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            if let Ok(mut writer) = encoder.write_header() {
                let _ = writer.write_image_data(&self.rgba);
            }
        }
    }
}

fn capture(window: &Window) -> Option<Capture> {
    let number = u32::try_from(ns_window(window).windowNumber()).ok()?;
    let null_rect = NSRect::new(
        NSPoint::new(f64::INFINITY, f64::INFINITY),
        NSSize::new(0.0, 0.0),
    );
    // SAFETY: each pointer is checked before use and comes straight from the call before it. The image
    // and data are released once, after their bytes have been copied out.
    unsafe {
        let image = CGWindowListCreateImage(
            null_rect,
            WINDOW_LIST_INCLUDING_WINDOW,
            number,
            IMAGE_BOUNDS_IGNORE_FRAMING,
        );
        if image.is_null() {
            return None;
        }
        let (width, height) = (CGImageGetWidth(image), CGImageGetHeight(image));
        let stride = CGImageGetBytesPerRow(image);
        let data = CGDataProviderCopyData(CGImageGetDataProvider(image));
        let bytes =
            std::slice::from_raw_parts(CFDataGetBytePtr(data), CFDataGetLength(data) as usize);
        let mut rgba = Vec::with_capacity(width * height * 4);
        for row in bytes.chunks_exact(stride).take(height) {
            for pixel in row[..width * 4].chunks_exact(4) {
                rgba.extend_from_slice(&[pixel[2], pixel[1], pixel[0], pixel[3]]);
            }
        }
        CFRelease(data);
        CGImageRelease(image);
        Some(Capture {
            width,
            height,
            rgba,
        })
    }
}

async fn sleep(ms: u64) {
    tokio::time::sleep(Duration::from_millis(ms)).await;
}

async fn eval(script: &str) -> Value {
    document::eval(script).await.unwrap_or(Value::Null)
}

async fn rect_of(selector: &str) -> Option<(f64, f64, f64, f64)> {
    let value = eval(&format!(
        "const e = document.querySelector({selector:?}); if (!e) return null; const r = e.getBoundingClientRect(); return [r.x, r.y, r.width, r.height];"
    ))
    .await;
    Some((
        value.get(0)?.as_f64()?,
        value.get(1)?.as_f64()?,
        value.get(2)?.as_f64()?,
        value.get(3)?.as_f64()?,
    ))
}

async fn click(window: &Window, x: f64, y: f64) {
    if !ns_window(window).isKeyWindow() {
        make_key(window);
        sleep(200).await;
    }
    send_mouse(window, x, y, true);
    sleep(30).await;
    send_mouse(window, x, y, false);
    sleep(200).await;
}

fn percentile(values: &[f32], fraction: f64) -> f32 {
    if values.is_empty() {
        return 0.0;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(f32::total_cmp);
    sorted[((sorted.len() - 1) as f64 * fraction) as usize]
}

pub async fn run() {
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_secs(120));
        eprintln!("self-test timed out");
        std::process::exit(3);
    });
    let out = PathBuf::from(std::env::var("REEL_OUT").unwrap_or_else(|_| ".".into()));
    std::fs::create_dir_all(&out).expect("couldn't create the output folder");
    let window = dioxus::desktop::window().window.clone();
    window.set_always_on_top(true);
    make_key(&window);
    let mut checks: Vec<Value> = vec![];
    let mut check = |name: &str, pass: bool, detail: String| {
        println!("{} {name}: {detail}", if pass { "PASS" } else { "FAIL" });
        checks.push(json!({ "name": name, "pass": pass, "detail": detail }));
    };
    sleep(1200).await;

    let title_bar = capture(&window).map_or(0, |image| {
        image
            .height
            .saturating_sub(window.inner_size().height as usize)
    });
    let (play_x, play_y, play_w, play_h) = rect_of("#play")
        .await
        .expect("the play button should be on the page");
    let (play_x, play_y) = (play_x + play_w / 2.0, play_y + play_h / 2.0);
    let (_, _, hole_w, hole_h) = rect_of("#preview-hole")
        .await
        .expect("the preview should be on the page");
    check(
        "the preview has room",
        hole_w > 100.0 && hole_h > 100.0,
        format!("{hole_w}x{hole_h}"),
    );

    // Play with the real button, for a good while.
    click(&window, play_x, play_y).await;
    check(
        "the play button starts playback",
        SHARED.is_playing(),
        format!("playing {}", SHARED.is_playing()),
    );
    sleep(500).await;
    SHARED.stats.lock().unwrap().reset();
    let started_at = SHARED.position();
    let started = Instant::now();
    let moving = capture(&window);
    sleep(8000).await;
    let moved_to = SHARED.position();
    let elapsed = started.elapsed().as_secs_f64();
    let later = capture(&window);

    let (present_fps, frames_per_second, sync_p50, sync_p99, sync_max, long) = {
        let stats = SHARED.stats.lock().unwrap();
        let median = percentile(&stats.present_ms, 0.5);
        (
            stats.present_ms.len() as f64 / elapsed,
            stats.distinct_frames as f64 / elapsed,
            percentile(&stats.sync_ms, 0.5),
            percentile(&stats.sync_ms, 0.99),
            percentile(&stats.sync_ms, 1.0),
            stats
                .present_ms
                .iter()
                .filter(|&&ms| ms > median * 1.5)
                .count(),
        )
    };
    let clock_rate = (moved_to - started_at).secs() / elapsed;
    check(
        "the clock runs at normal speed",
        (clock_rate - 1.0).abs() < 0.02,
        format!("{clock_rate:.3}x over {elapsed:.1} s"),
    );
    check(
        "the picture keeps up with the clock",
        sync_p99 < 50.0,
        format!(
            "off by {sync_p50:.1} ms typically, {sync_p99:.1} ms at p99, {sync_max:.1} ms at worst"
        ),
    );
    check(
        "playback is smooth",
        present_fps > 55.0 && long < 8,
        format!(
            "{present_fps:.1} presents a second, {frames_per_second:.1} distinct frames a second, {long} missed refreshes in {elapsed:.1} s"
        ),
    );
    if let (Some(a), Some(b)) = (&moving, &later) {
        let (w, h) = (a.width.min(b.width), a.height.min(b.height));
        let mut difference = 0_u64;
        for y in (title_bar + 60..h.saturating_sub(70)).step_by(23) {
            for x in (10..w).step_by(23) {
                let (p, q) = (a.pixel(x, y), b.pixel(x, y));
                difference += p
                    .iter()
                    .zip(&q)
                    .map(|(m, n)| u64::from(m.abs_diff(*n)))
                    .sum::<u64>();
            }
        }
        check(
            "the picture really is moving on screen",
            difference > 1000,
            format!("{difference} total difference between two captures"),
        );
        b.save(&out.join("1-playing.png"));
    }

    // Space pauses, with the focus on the page.
    eval("document.querySelector('.shell').focus(); return true;").await;
    sleep(50).await;
    send_key(&window, " ", 49);
    sleep(300).await;
    check(
        "Space pauses",
        !SHARED.is_playing(),
        format!("playing {}", SHARED.is_playing()),
    );

    // Clicking along the scrub bar moves playback there.
    if let Some((x, y, w, h)) = rect_of(".shell-scrub input").await {
        let fraction = 0.5;
        click(&window, x + w * fraction, y + h / 2.0).await;
        sleep(600).await;
        let at = SHARED.position().secs();
        let total = crate::preview::duration().secs();
        check(
            "clicking the scrub bar moves playback",
            (at / total - fraction).abs() < 0.05,
            format!("{at:.2} s of {total:.2} s"),
        );
    }

    // The picture on screen against a still from the export path, at this exact time.
    let paused = SHARED.position();
    sleep(400).await;
    if let (Some(image), Some(scene)) = (capture(&window), scene_for_comparison()) {
        image.save(&out.join("2-paused.png"));
        let still =
            reel_render::export::render_still(&scene, paused).expect("a still should render");
        let (hole_x, hole_y, hole_w, hole_h) = rect_of("#preview-hole")
            .await
            .expect("the preview should be on the page");
        let (stage_w, stage_h) = (f64::from(scene.stage.0), f64::from(scene.stage.1));
        let scale = (hole_w / stage_w).min(hole_h / stage_h);
        let (shown_w, shown_h) = (stage_w * scale, stage_h * scale);
        let (shown_x, shown_y) = (
            hole_x + (hole_w - shown_w) / 2.0,
            hole_y + (hole_h - shown_h) / 2.0,
        );

        let dpi = window.scale_factor();
        let mut squared = 0.0_f64;
        let mut count = 0_u64;
        let mut mean = [0.0_f64; 3];
        for row in 2..78 {
            for column in 2..158 {
                let (u, v) = (f64::from(column) / 160.0, f64::from(row) / 80.0);
                // Captures are in physical pixels, and page positions are in CSS pixels.
                let screen = image.pixel(
                    ((shown_x + u * shown_w) * dpi) as usize,
                    ((shown_y + v * shown_h) * dpi) as usize + title_bar,
                );
                let (sx, sy) = ((u * stage_w) as usize, (v * stage_h) as usize);
                let at = (sy * scene.stage.0 as usize + sx) * 4;
                for channel in 0..3 {
                    let difference = f64::from(screen[channel]) - f64::from(still[at + channel]);
                    squared += difference * difference;
                    mean[channel] += difference;
                }
                count += 3;
            }
        }
        let psnr = 10.0 * (255.0_f64 * 255.0 / (squared / count as f64).max(1e-9)).log10();
        let per = (count / 3) as f64;
        check(
            "the screen matches the export path's still",
            psnr > 22.0,
            format!(
                "{psnr:.1} dB at {:.3} s, mean difference r{:+.1} g{:+.1} b{:+.1}, from sampling scaled and unscaled images, at {dpi}x",
                paused.secs(),
                mean[0] / per,
                mean[1] / per,
                mean[2] / per
            ),
        );
    }

    // A window resize keeps going, and the video keeps drawing.
    use dioxus::desktop::tao::dpi::LogicalSize;
    window.set_inner_size(LogicalSize::new(960.0, 640.0));
    sleep(500).await;
    let resized = rect_of("#preview-hole")
        .await
        .expect("the preview should be on the page");
    check(
        "the preview follows a resize",
        resized.2 < hole_w,
        format!("{} wide, was {hole_w}", resized.2),
    );
    if let Some(image) = capture(&window) {
        image.save(&out.join("3-resized.png"));
    }

    let passed = checks.iter().all(|entry| entry["pass"] == true);
    let report = json!({
        "passed": passed,
        "checks": checks,
        "present_fps": present_fps,
        "video_fps": frames_per_second,
        "sync_ms": { "p50": sync_p50, "p99": sync_p99, "max": sync_max },
    });
    std::fs::write(
        out.join("report.json"),
        serde_json::to_vec_pretty(&report).unwrap(),
    )
    .expect("couldn't write the report");
    std::process::exit(i32::from(!passed));
}

fn scene_for_comparison() -> Option<reel_render::export::Scene> {
    let path = std::env::args().skip(1).find(|arg| !arg.starts_with('-'))?;
    let text = std::fs::read_to_string(&path).ok()?;
    let project = reel_core::format::from_str(&text).ok()?;
    reel_render::export::Scene::new(project, std::path::Path::new(&path).parent()).ok()
}
