//! A scripted run through the launch bar, using a test pattern so it needs no permissions.
//!
//! It clicks the real controls, from opening popovers to recording, pausing, stopping and having the
//! editor open with the result.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use dioxus::desktop::tao::window::Window;
use dioxus::prelude::*;
use serde_json::{Value, json};

use crate::bar::{BAR_HEIGHT, OPEN_HEIGHT};
use crate::native::{self, ns_window};
use crate::preview;
use crate::recording;
use crate::self_test::{capture, send_mouse_at};

async fn sleep(ms: u64) {
    tokio::time::sleep(Duration::from_millis(ms)).await;
}

async fn eval(script: &str) -> Value {
    document::eval(script).await.unwrap_or(Value::Null)
}

async fn center_of(id: &str) -> Option<(f64, f64)> {
    let value = eval(&format!(
        "const e = document.getElementById({id:?}); if (!e) return null; const r = e.getBoundingClientRect(); return [r.x + r.width / 2, r.y + r.height / 2];"
    ))
    .await;
    Some((value.get(0)?.as_f64()?, value.get(1)?.as_f64()?))
}

async fn click(window: &Window, id: &str) -> bool {
    let Some((x, y)) = center_of(id).await else {
        return false;
    };
    // The page is pinned at its largest size, so measure from there rather than from the window.
    let from_bottom = OPEN_HEIGHT - y;
    send_mouse_at(window, x, from_bottom, true);
    sleep(30).await;
    send_mouse_at(window, x, from_bottom, false);
    sleep(250).await;
    true
}

async fn text_of(id: &str) -> String {
    eval(&format!(
        "const e = document.getElementById({id:?}); return e ? e.textContent : '';"
    ))
    .await
    .as_str()
    .unwrap_or_default()
    .to_string()
}

async fn exists(id: &str) -> bool {
    center_of(id).await.is_some()
}

async fn wait_for(id: &str, seconds: u64) -> bool {
    let deadline = Instant::now() + Duration::from_secs(seconds);
    while Instant::now() < deadline {
        if exists(id).await {
            return true;
        }
        sleep(50).await;
    }
    false
}

fn opaque_pixels(image: &crate::self_test::Capture) -> f64 {
    let mut solid = 0_u64;
    let mut total = 0_u64;
    for y in (0..image.height).step_by(4) {
        for x in (0..image.width).step_by(4) {
            total += 1;
            if image.pixel(x, y)[3] > 200 {
                solid += 1;
            }
        }
    }
    solid as f64 / total.max(1) as f64
}

pub async fn run() {
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_secs(120));
        eprintln!("self-test timed out");
        std::process::exit(3);
    });
    let out = PathBuf::from(std::env::var("REEL_OUT").unwrap_or_else(|_| ".".into()));
    std::fs::create_dir_all(&out).expect("couldn't create the output folder");
    let context = dioxus::desktop::window();
    let window = context.window.clone();
    let mut checks: Vec<Value> = vec![];
    let mut check = |name: &str, pass: bool, detail: String| {
        println!("{} {name}: {detail}", if pass { "PASS" } else { "FAIL" });
        checks.push(json!({ "name": name, "pass": pass, "detail": detail }));
    };
    native::bring_forward(&window);
    sleep(1000).await;

    // Capture protection blanks the window in captures too, including this test's own, so check that
    // it's on and then turn it off to take pictures.
    check(
        "the bar is kept out of captures",
        ns_window(&window).sharingType() == objc2_app_kit::NSWindowSharingType::None,
        String::new(),
    );
    native::set_capture_protected(&window, false);
    sleep(100).await;

    // The bar on its own.
    check(
        "the bar starts small",
        (native::height(&window) - BAR_HEIGHT).abs() < 1.0,
        format!("{} tall", native::height(&window)),
    );
    let (x_before, y_before) = native::origin(&window);
    if let Some(image) = capture(&window) {
        image.save(&out.join("bar-1-idle.png"));
        // The corners are see-through, and the bar itself isn't.
        let corner = image.pixel(1, image.height - 2);
        check(
            "the window around the bar is see-through",
            corner[3] < 40,
            format!("corner alpha {}", corner[3]),
        );
        let middle = image.pixel(image.width / 2, image.height / 2);
        check(
            "the bar itself is solid",
            middle[3] > 200,
            format!("alpha {}", middle[3]),
        );
        check(
            "the bar is drawn in the window",
            opaque_pixels(&image) > 0.5,
            format!("{:.0}% solid", opaque_pixels(&image) * 100.0),
        );
    }

    // Popovers grow the window upward, one at a time, and shrink it again.
    click(&window, "pop-countdown").await;
    let (x_open, y_open) = native::origin(&window);
    check(
        "a popover grows the window",
        (native::height(&window) - OPEN_HEIGHT).abs() < 1.0,
        format!("{} tall", native::height(&window)),
    );
    check(
        "and the bar doesn't move",
        (x_open - x_before).abs() < 0.5 && (y_open - y_before).abs() < 0.5,
        format!("origin {x_open},{y_open}, was {x_before},{y_before}"),
    );
    check(
        "the countdown choices are showing",
        exists("countdown-5").await,
        String::new(),
    );
    if let Some(image) = capture(&window) {
        image.save(&out.join("bar-2-popover.png"));
    }
    click(&window, "pop-audio").await;
    check(
        "only one popover is open at a time",
        exists("toggle-system-audio").await && !exists("countdown-5").await,
        String::new(),
    );
    sleep(300).await;
    if let Some(image) = capture(&window) {
        image.save(&out.join("bar-3-audio.png"));
    }
    click(&window, "pop-audio").await;
    check(
        "closing it shrinks the window again",
        (native::height(&window) - BAR_HEIGHT).abs() < 1.0 && !exists("bar-pop").await,
        format!("{} tall", native::height(&window)),
    );
    let (_, y_closed) = native::origin(&window);
    check(
        "still without moving the bar",
        (y_closed - y_before).abs() < 0.5,
        format!("{y_closed}, was {y_before}"),
    );

    // Choosing a one-second-free run: no delay, so the test doesn't wait on the countdown.
    click(&window, "pop-countdown").await;
    click(&window, "countdown-0").await;
    click(&window, "pop-countdown").await;

    // A whole recording, on the test pattern.
    let had_record = click(&window, "record").await;
    check("record can be clicked", had_record, String::new());
    let recording = wait_for("bar-pause", 10).await;
    check("recording starts", recording, text_of("bar-status").await);
    if let Some(image) = capture(&window) {
        image.save(&out.join("bar-4-recording.png"));
    }
    sleep(2300).await;
    let timer = text_of("bar-timer").await;
    check("the timer runs", timer.contains("00:02"), timer.clone());

    click(&window, "bar-pause").await;
    check(
        "pausing shows PAUSED",
        text_of("bar-status").await.contains("PAUSED"),
        text_of("bar-status").await,
    );
    let paused_at = text_of("bar-timer").await;
    sleep(1500).await;
    let still = text_of("bar-timer").await;
    check(
        "the timer stops during a pause",
        paused_at == still,
        format!("{paused_at} then {still}"),
    );
    if let Some(image) = capture(&window) {
        image.save(&out.join("bar-5-paused.png"));
    }
    click(&window, "bar-resume").await;
    check(
        "resuming goes back to REC",
        text_of("bar-status").await.contains("REC"),
        text_of("bar-status").await,
    );

    // Cancel asks first, which Recordly doesn't.
    click(&window, "bar-cancel").await;
    let asks = exists("cancel-keep").await;
    check("cancelling asks first", asks, String::new());
    if let Some(image) = capture(&window) {
        image.save(&out.join("bar-6-confirm.png"));
    }
    click(&window, "cancel-keep").await;
    check(
        "and keep carries on recording",
        exists("bar-stop").await && !exists("cancel-keep").await,
        String::new(),
    );
    sleep(1000).await;

    // Straight after the click, without the usual wait: a short recording finishes quickly.
    let (x, y) = center_of("bar-stop").await.unwrap_or_default();
    send_mouse_at(&window, x, OPEN_HEIGHT - y, true);
    send_mouse_at(&window, x, OPEN_HEIGHT - y, false);
    let mut finishing = false;
    for _ in 0..40 {
        if exists("bar-finalizing").await {
            finishing = true;
            break;
        }
        sleep(1).await;
    }
    check("stopping shows it's preparing", finishing, String::new());
    let deadline = Instant::now() + Duration::from_secs(20);
    while !crate::editor_is_showing() && Instant::now() < deadline {
        sleep(50).await;
    }
    check(
        "the editor opens with the recording",
        crate::editor_is_showing(),
        String::new(),
    );
    let deadline = Instant::now() + Duration::from_secs(10);
    while preview::duration().secs() <= 0.0 && Instant::now() < deadline {
        sleep(50).await;
    }
    let length = preview::duration().secs();
    // About 2.3 s, then a little over 1 s, plus how long the clicks and captures took.
    check(
        "the recording has the right length, without the pause",
        (3.0..6.5).contains(&length),
        format!("{length:.2} s"),
    );
    check(
        "the bar has hidden",
        !ns_window(&window).isVisible(),
        String::new(),
    );

    // The saved project has its tracks linked up.
    let saved = recording::support_folder().join("recordings");
    let project = std::fs::read_dir(&saved).ok().and_then(|entries| {
        entries.flatten().map(|entry| entry.path()).find(|path| {
            path.extension()
                .is_some_and(|extension| extension == reel_core::format::EXTENSION)
        })
    });
    check(
        "a project is saved beside the recording",
        project.is_some(),
        format!("{project:?}"),
    );

    // Closing the editor, the way its close button does, brings the bar back (§48).
    if let Some(main_thread) = objc2::MainThreadMarker::new() {
        let windows = objc2_app_kit::NSApplication::sharedApplication(main_thread).windows();
        let bar_number = ns_window(&window).windowNumber();
        if let Some(editor) = windows
            .iter()
            .find(|other| other.isVisible() && other.windowNumber() != bar_number)
        {
            editor.performClose(None);
        }
    }
    sleep(500).await;
    check(
        "closing the editor brings the bar back",
        ns_window(&window).isVisible(),
        String::new(),
    );
    check(
        "and the editor is hidden",
        !crate::editor_is_showing(),
        String::new(),
    );
    check(
        "the bar is small again",
        (native::height(&window) - BAR_HEIGHT).abs() < 1.0,
        format!("{}", native::height(&window)),
    );

    // It stays where it was put, and a place that's no longer on any screen isn't used.
    let (x, y) = native::origin(&window);
    check(
        "a place on screen can be used",
        native::place_at(&window, x + 40.0, y + 40.0),
        String::new(),
    );
    sleep(300).await;
    let store = reel_core::settings::SettingsStore::open(
        &recording::support_folder().join("settings.json"),
    );
    check(
        "and it's remembered",
        store
            .get()
            .bar_position
            .is_some_and(|(saved_x, _)| (saved_x - (x + 40.0)).abs() < 1.0),
        format!("{:?}", store.get().bar_position),
    );
    check(
        "a place off every screen isn't",
        !native::place_at(&window, -90_000.0, -90_000.0),
        String::new(),
    );

    let report =
        json!({ "checks": checks, "passed": checks.iter().all(|check| check["pass"] == true) });
    let passed = report["passed"] == true;
    let _ = std::fs::write(out.join("report.json"), report.to_string());
    println!(
        "{}",
        if passed {
            "self-test passed"
        } else {
            "self-test failed"
        }
    );
    sleep(200).await;
    std::process::exit(i32::from(!passed));
}
