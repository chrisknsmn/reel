//! T3 from BUILD_PLAN.md: a small, transparent, always-on-top launch bar that grows upward when a
//! popover opens. The window only ever covers what's visible, so clicks anywhere else reach other
//! apps without any of the mouse forwarding Recordly's full-screen window needs.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use dioxus::desktop::tao::dpi::{LogicalPosition, LogicalSize};
use dioxus::desktop::{Config, WindowBuilder};
use dioxus::prelude::*;
use serde_json::{Value, json};

mod macos;
use macos::Capture;

const BAR_WIDTH: f64 = 640.0;
const BAR_HEIGHT: f64 = 72.0;
pub const OPEN_HEIGHT: f64 = 400.0;
/// The bar's marker is this far above the window's bottom edge, measured to its first lime pixel.
const MARKER_FROM_BOTTOM: usize = 20;

const CSS: &str = r#"
html, body, #main { background: transparent !important; margin: 0; height: 100%; overflow: hidden; font: 13px -apple-system, sans-serif; }
.bar { position: fixed; left: 8px; right: 8px; bottom: 8px; height: 56px; border-radius: 16px; background: #1c1c1e; color: #fff; display: flex; align-items: center; gap: 8px; padding: 0 12px; box-sizing: border-box; }
.marker { position: absolute; left: 4px; bottom: 12px; width: 4px; height: 32px; background: #00ff00; }
.handle { width: 24px; height: 32px; border-radius: 6px; background: #3a3a3c; cursor: grab; margin-left: 8px; }
button { background: #2c2c2e; color: #fff; border: 0; border-radius: 8px; padding: 8px 12px; }
.pop { position: fixed; left: 170px; bottom: 72px; width: 300px; height: 300px; border-radius: 12px; background: #2c2c2e; color: #fff; padding: 12px; box-sizing: border-box; }
"#;

fn main() {
    let window = WindowBuilder::new()
        .with_title("T3 Launch Bar")
        .with_decorations(false)
        .with_transparent(true)
        .with_resizable(false)
        .with_always_on_top(true)
        .with_visible_on_all_workspaces(true)
        .with_inner_size(LogicalSize::new(BAR_WIDTH, BAR_HEIGHT));

    let config = Config::new()
        .with_window(window)
        .with_menu(None)
        .with_background_color((0, 0, 0, 0))
        .with_custom_head(format!("<style>{CSS}</style>"));

    dioxus::LaunchBuilder::desktop()
        .with_cfg(config)
        .launch(App);
}

#[component]
fn App() -> Element {
    let mut open = use_signal(|| false);
    use_hook(|| {
        macos::accept_first_mouse();
        macos::pin_webview(&dioxus::desktop::window().window, BAR_WIDTH, OPEN_HEIGHT);
    });

    use_future(move || async move {
        if std::env::var_os("T3_SELF_TEST").is_some() {
            self_test(open).await;
        }
    });

    let toggle = move |_| {
        let window = dioxus::desktop::window();
        if open() {
            macos::set_height(&window.window, BAR_HEIGHT);
            open.set(false);
        } else {
            macos::set_height(&window.window, OPEN_HEIGHT);
            open.set(true);
        }
    };

    rsx! {
        div { class: "bar", id: "bar",
            div { class: "marker" }
            div {
                class: "handle",
                id: "handle",
                onmousedown: move |_| dioxus::desktop::window().drag(),
            }
            button { id: "audio", onclick: toggle, "Audio" }
            button { id: "other", "Record" }
        }
        if open() {
            div { class: "pop", id: "pop", "Popover, 300 px wide, opening upward." }
        }
    }
}

const TRACK_CLICKS: &str = r#"
window.__clicks = [];
document.addEventListener('click', e => window.__clicks.push(e.target.id || e.target.tagName), true);
return true;
"#;

async fn sleep(ms: u64) {
    tokio::time::sleep(Duration::from_millis(ms)).await;
}

async fn eval(script: &str) -> Value {
    document::eval(script).await.unwrap_or(Value::Null)
}

async fn center_of(id: &str) -> (f64, f64) {
    let value = eval(&format!(
        "const r = document.getElementById('{id}').getBoundingClientRect(); return [r.x + r.width / 2, r.y + r.height / 2];"
    ))
    .await;
    (
        value[0].as_f64().unwrap_or(0.0),
        value[1].as_f64().unwrap_or(0.0),
    )
}

/// Finds the marker's lower end, counting up from the window's bottom edge.
fn marker_from_bottom(capture: &Capture) -> Option<usize> {
    let column = 12;
    (0..capture.height).find(|&up| {
        let [r, g, b, a] = capture.pixel(column, capture.height - 1 - up);
        a > 200 && g > 190 && r < 170 && b < 140
    })
}

struct Sampler {
    done: std::sync::Arc<AtomicBool>,
    handle: std::thread::JoinHandle<Vec<(f64, usize, Option<usize>, bool)>>,
}

/// Captures the window as fast as it can from another thread, for the whole of a transition.
fn sample(number: isize, saved_example: std::path::PathBuf) -> Sampler {
    let done = std::sync::Arc::new(AtomicBool::new(false));
    let flag = done.clone();
    let handle = std::thread::spawn(move || {
        let started = Instant::now();
        let mut samples = Vec::new();
        let mut saved = false;
        while !flag.load(Ordering::Relaxed) {
            if let Some(capture) = macos::capture_number(number) {
                let at = marker_from_bottom(&capture);
                let visible = capture.rgba.chunks_exact(4).any(|p| p[3] > 200);
                if at != Some(MARKER_FROM_BOTTOM) && !saved {
                    saved = true;
                    let _ = capture.save(&saved_example);
                }
                samples.push((started.elapsed().as_secs_f64(), capture.height, at, visible));
            }
        }
        samples
    });
    Sampler { done, handle }
}

async fn self_test(mut open: Signal<bool>) {
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_secs(120));
        eprintln!("self-test timed out");
        std::process::exit(3);
    });
    let out = std::path::PathBuf::from(std::env::var("T3_OUT").unwrap_or_else(|_| ".".into()));
    std::fs::create_dir_all(&out).expect("couldn't create the output folder");
    let desktop = dioxus::desktop::window();
    let window = desktop.window.clone();
    let mut checks: Vec<Value> = vec![];
    let mut check = |name: &str, pass: bool, detail: String| {
        println!("{} {name}: {detail}", if pass { "PASS" } else { "FAIL" });
        checks.push(json!({ "name": name, "pass": pass, "detail": detail }));
    };

    sleep(1500).await;
    eval(TRACK_CLICKS).await;

    // Idle: the window is exactly the bar, and is see-through around its rounded corners.
    let idle = macos::capture(&window).expect("no capture");
    idle.save(&out.join("1-idle.png")).ok();
    let corner = idle.pixel(1, 1);
    let inside = idle.pixel(idle.width / 2, idle.height / 2);
    check(
        "idle window is only the bar, and transparent around it",
        corner[3] < 10 && inside[3] > 240 && idle.height == BAR_HEIGHT as usize,
        format!(
            "{}x{} corner alpha {} centre alpha {}",
            idle.width, idle.height, corner[3], inside[3]
        ),
    );

    // A first click on a background window should work, unlike T1's default.
    macos::make_key(&window);
    sleep(300).await;
    macos::deactivate_app();
    sleep(500).await;
    let key_before = macos::is_key(&window);
    let (x, y) = center_of("other").await;
    let view = macos::hit_test(&window, x, y);
    macos::click(&window, x, y, sleep(40)).await;
    sleep(300).await;
    let clicks = eval("return window.__clicks;").await;
    let received = clicks.as_array().is_some_and(|list| !list.is_empty());
    check(
        "first click on a background window works",
        !key_before && received,
        format!("key before {key_before}, view {view}, DOM saw {clicks}"),
    );
    macos::make_key(&window);
    sleep(300).await;

    // Growing and shrinking, repeatedly, while capturing as fast as possible. Start at the bottom
    // of the screen, where the bar lives: AppKit pushes a window down rather than let it grow past
    // the top of the screen, so step 5.3 must handle a bar that has been dragged up there.
    if let Some(monitor) = window.current_monitor() {
        let scale = monitor.scale_factor();
        let size = monitor.size().to_logical::<f64>(scale);
        let position = monitor.position().to_logical::<f64>(scale);
        window.set_outer_position(LogicalPosition::new(
            position.x + (size.width - BAR_WIDTH) / 2.0,
            position.y + size.height - BAR_HEIGHT - 40.0,
        ));
        sleep(300).await;
    }
    let number = macos::window_number(&window);
    let frame_before = macos::window_frame(&window);
    let sampler = sample(number, out.join("2-misplaced-example.png"));
    let cycles = 12;
    let mut grow_ms = vec![];
    let (ax, ay) = center_of("audio").await;
    println!("audio button at {ax},{ay}");
    let mut lost_clicks = 0;
    for _ in 0..cycles {
        let expected = if open() { BAR_HEIGHT } else { OPEN_HEIGHT };
        let started = Instant::now();
        macos::click(&window, ax, ay, sleep(16)).await;
        let mut reached = false;
        for _ in 0..200 {
            if (macos::window_frame(&window).3 - expected).abs() < 0.5 {
                reached = true;
                break;
            }
            sleep(2).await;
        }
        if reached {
            grow_ms.push(started.elapsed().as_secs_f32() * 1000.0);
        } else {
            lost_clicks += 1;
        }
        sleep(180).await;
    }
    sampler.done.store(true, Ordering::Relaxed);
    let samples = sampler.handle.join().unwrap_or_default();
    let total = samples.len();
    let misplaced = samples
        .iter()
        .filter(|s| s.2 != Some(MARKER_FROM_BOTTOM))
        .count();
    let blank = samples.iter().filter(|s| !s.3).count();
    let positions: std::collections::BTreeMap<String, usize> =
        samples.iter().fold(Default::default(), |mut map, s| {
            *map.entry(format!("{:?}", s.2)).or_default() += 1;
            map
        });
    let frame_after = macos::window_frame(&window);
    // The window must really have changed size, or none of the rest means anything.
    let changes = samples
        .windows(2)
        .filter(|pair| pair[0].1 != pair[1].1)
        .count();
    check(
        "the window really grew and shrank during the captures",
        changes == cycles && lost_clicks == 0,
        format!("{changes} height changes seen for {cycles} clicks, {lost_clicks} clicks lost"),
    );
    check(
        "the bar doesn't jump while the window grows and shrinks",
        total > 50 && misplaced == 0 && blank == 0,
        format!(
            "{total} captures across {cycles} transitions: {misplaced} misplaced, {blank} blank, marker heights {positions:?}"
        ),
    );
    check(
        "the bar's bottom edge never moved on screen",
        (frame_before.1 - frame_after.1).abs() < 0.5
            && (frame_before.0 - frame_after.0).abs() < 0.5,
        format!("before {frame_before:?}, after {frame_after:?}"),
    );

    // Open again, to see the popover.
    macos::set_height(&window, OPEN_HEIGHT);
    open.set(true);
    sleep(300).await;
    if let Some(capture) = macos::capture(&window) {
        capture.save(&out.join("3-popover.png")).ok();
        let popover = capture.pixel(300, 150);
        let beside = capture.pixel(40, 150);
        check(
            "the popover shows, and the window stays transparent beside it",
            popover[3] > 240 && beside[3] < 10,
            format!("popover alpha {} beside it {}", popover[3], beside[3]),
        );
    }
    open.set(false);
    sleep(40).await;
    macos::set_height(&window, BAR_HEIGHT);
    sleep(300).await;

    // Moving between monitors.
    let monitors: Vec<_> = window.available_monitors().collect();
    let mut visited = vec![];
    for monitor in &monitors {
        let scale = monitor.scale_factor();
        let size = monitor.size().to_logical::<f64>(scale);
        let position = monitor.position().to_logical::<f64>(scale);
        window.set_outer_position(LogicalPosition::new(
            position.x + (size.width - BAR_WIDTH) / 2.0,
            position.y + size.height - BAR_HEIGHT - 40.0,
        ));
        sleep(400).await;
        let current = window.current_monitor().map(|m| m.position());
        let capture = macos::capture(&window);
        let drawn = capture.as_ref().and_then(marker_from_bottom);
        visited.push(json!({
            "monitor": format!("{:?}", monitor.name()),
            "scale": scale,
            "reached": current == Some(monitor.position()),
            "marker": drawn,
        }));
    }
    let moved = visited
        .iter()
        .all(|v| v["reached"] == true && !v["marker"].is_null());
    check(
        "the bar moves to every monitor and draws there",
        moved,
        format!("{} monitors: {}", monitors.len(), json!(visited)),
    );

    // Content protection. The self-test can't read the window after this.
    window.set_content_protection(true);
    sleep(400).await;
    let protected = macos::capture(&window);
    let hidden = protected
        .as_ref()
        .is_none_or(|c| !c.rgba.chunks_exact(4).any(|p| p[3] > 200));
    if let Some(capture) = &protected {
        capture.save(&out.join("4-protected.png")).ok();
    }
    check(
        "content protection hides the bar from window capture",
        hidden,
        format!(
            "capture {}",
            protected
                .as_ref()
                .map_or("was refused".to_string(), |c| format!(
                    "{}x{}",
                    c.width, c.height
                ))
        ),
    );
    window.set_content_protection(false);

    let passed = checks.iter().all(|c| c["pass"] == true);
    let report = json!({ "passed": passed, "checks": checks, "transition_ms": grow_ms, "screen_recording_permission": macos::has_screen_recording_permission() });
    std::fs::write(
        out.join("report.json"),
        serde_json::to_vec_pretty(&report).unwrap(),
    )
    .expect("couldn't write the report");
    println!("done");
    std::process::exit(if passed { 0 } else { 1 });
}
