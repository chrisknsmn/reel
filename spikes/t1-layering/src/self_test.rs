//! The scripted version of T1: clicks controls through AppKit, measures frame pacing while the DOM
//! does different things, and checks alignment on captured pixels. It writes `report.json` and PNGs
//! to T1_OUT, then exits.

use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use dioxus::desktop::tao::dpi::LogicalSize;
use dioxus::desktop::tao::window::Window;
use dioxus::prelude::*;
use serde_json::{Value, json};

use crate::macos::{self, Capture};
use crate::render::{PhaseStats, SHARED};

const TRACK_CLICKS: &str = r#"
window.__clicks = [];
window.__mouse = { pointerdown: 0, mousedown: 0, mouseup: 0, click: 0 };
for (const kind of Object.keys(window.__mouse)) {
    document.addEventListener(kind, () => window.__mouse[kind]++, true);
}
document.addEventListener('click', e => {
    const named = e.target.closest('[data-name]');
    window.__clicks.push({ x: e.clientX, y: e.clientY, id: e.target.id, name: named ? named.dataset.name : '' });
}, true);
return true;
"#;

const UNMEASURED: &str = "between";
const POPOVER: &str = "[data-name='PopoverContent']";
const POPOVER_TRIGGER: &str = "[data-name='PopoverTrigger']";
const DIALOG: &str = "[data-name='DialogContent']";
const DIALOG_TRIGGER: &str = "[data-dialog-trigger]";
const DIALOG_CLOSE: &str = "[data-name='DialogContent'] footer [data-dialog-close]";

/// Re-styles 600 elements every frame for five seconds, then tells us it has finished.
const BUSY_DOM: &str = r#"
const box = document.querySelector('.timeline');
const chips = [];
for (let i = 0; i < 600; i++) {
    const chip = document.createElement('i');
    chip.className = 'chip';
    box.appendChild(chip);
    chips.push(chip);
}
await new Promise(done => {
    const start = performance.now();
    const tick = now => {
        if (now - start > 5000) { done(); return; }
        chips.forEach((chip, i) => {
            chip.style.left = ((now / 5 + i * 7) % 1000) + 'px';
            chip.style.top = (i % 60) * 2 + 'px';
        });
        requestAnimationFrame(tick);
    };
    requestAnimationFrame(tick);
});
chips.forEach(chip => chip.remove());
return true;
"#;

struct Css {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

impl Css {
    fn center(&self) -> (f64, f64) {
        (self.x + self.w / 2.0, self.y + self.h / 2.0)
    }
}

struct Test {
    window: std::sync::Arc<Window>,
    out: PathBuf,
    checks: Vec<Value>,
    hit_tests: Vec<Value>,
    /// Clicks sent while the window wasn't key, which AppKit absorbs. Failures that follow one say
    /// nothing about layering, so the report flags them rather than blaming the layers.
    clicks_without_focus: u32,
}

impl Test {
    fn check(&mut self, name: &str, pass: bool, detail: impl Into<String>) {
        let detail = detail.into();
        println!("{} {name}: {detail}", if pass { "PASS" } else { "FAIL" });
        self.checks
            .push(json!({ "name": name, "pass": pass, "detail": detail }));
    }

    async fn eval(&self, script: &str) -> Value {
        document::eval(script).await.unwrap_or(Value::Null)
    }

    async fn rect(&self, selector: &str) -> Option<Css> {
        let script = format!(
            "const e = document.querySelector({selector:?}); if (!e) return null;
             const r = e.getBoundingClientRect(); return {{ x: r.x, y: r.y, w: r.width, h: r.height }};"
        );
        let value = self.eval(&script).await;
        Some(Css {
            x: value.get("x")?.as_f64()?,
            y: value.get("y")?.as_f64()?,
            w: value.get("w")?.as_f64()?,
            h: value.get("h")?.as_f64()?,
        })
    }

    async fn state(&self, selector: &str) -> String {
        let script = format!(
            "return document.querySelector({selector:?})?.getAttribute('data-state') ?? 'missing';"
        );
        self.eval(&script)
            .await
            .as_str()
            .unwrap_or("missing")
            .to_string()
    }

    async fn last_click(&self) -> Value {
        self.eval("return window.__clicks[window.__clicks.length - 1] ?? null;")
            .await
    }

    /// Clicks at a CSS position through AppKit, noting which view AppKit hit-tested to.
    async fn click(&mut self, label: &str, x: f64, y: f64) -> String {
        let key_on_arrival = macos::is_visible_and_key(&self.window).1;
        if !key_on_arrival {
            macos::make_key(&self.window);
            sleep(150).await;
        }
        let key_at_click = macos::is_visible_and_key(&self.window).1;
        self.clicks_without_focus += u32::from(!key_at_click);
        let view = macos::hit_test(&self.window, x, y);
        macos::send_mouse(&self.window, x, y, true);
        sleep(40).await;
        macos::send_mouse(&self.window, x, y, false);
        sleep(300).await;
        let mouse = self.eval("return window.__mouse;").await;
        println!(
            "click {label} at {x:.0},{y:.0}: key on arrival {key_on_arrival}, key at click {key_at_click}, view {view}, DOM events so far {mouse}"
        );
        self.hit_tests.push(json!({ "target": label, "x": x, "y": y, "view": view, "key_on_arrival": key_on_arrival, "key_at_click": key_at_click, "dom_events": mouse }));
        view
    }

    async fn click_element(&mut self, label: &str, selector: &str) -> String {
        match self.rect(selector).await {
            Some(rect) => {
                let (x, y) = rect.center();
                self.click(label, x, y).await
            }
            None => {
                self.check(label, false, format!("{selector} isn't in the DOM"));
                String::new()
            }
        }
    }

    fn capture(&self, name: &str) -> Option<Capture> {
        let capture = macos::capture(&self.window);
        match &capture {
            Some(capture) if !name.is_empty() => {
                if let Err(error) = capture.save(&self.out.join(format!("{name}.png"))) {
                    eprintln!("couldn't save {name}.png: {error}");
                }
            }
            _ => {}
        }
        capture
    }

    /// How far the capture's top edge sits above the content view, in pixels.
    fn title_bar_height(&self, capture: &Capture) -> usize {
        capture
            .height
            .saturating_sub(self.window.inner_size().height as usize)
    }

    /// Records whether the window can be seen, since hidden windows make frame timing meaningless.
    fn note_visibility(&mut self, phase: &str) {
        let (visible, key) = macos::is_visible_and_key(&self.window);
        println!("{phase}: visible {visible}, key {key}");
    }

    async fn phase(&mut self, name: &str, seconds: u64) {
        self.note_visibility(name);
        SHARED.begin_phase(name);
        sleep(seconds * 1000).await;
        // Captures and clicks stall the render thread a little, so keep them out of measured phases.
        SHARED.begin_phase(UNMEASURED);
    }

    async fn set_size(&self, width: u32, height: u32) -> Instant {
        let at = Instant::now();
        self.window
            .set_inner_size(LogicalSize::new(f64::from(width), f64::from(height)));
        at
    }
}

async fn sleep(ms: u64) {
    tokio::time::sleep(Duration::from_millis(ms)).await;
}

pub async fn run() {
    // Keeps a hung run from sitting on screen forever.
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_secs(240));
        eprintln!("self-test timed out");
        std::process::exit(3);
    });

    let out = PathBuf::from(std::env::var("T1_OUT").unwrap_or_else(|_| ".".to_string()));
    std::fs::create_dir_all(&out).expect("couldn't create the output folder");
    let window = dioxus::desktop::window().window.clone();
    window.set_always_on_top(true);

    let mut test = Test {
        window,
        out: out.clone(),
        checks: vec![],
        hit_tests: vec![],
        clicks_without_focus: 0,
    };

    SHARED.begin_phase("startup");
    wait_for_startup().await;
    test.eval(TRACK_CLICKS).await;

    if std::env::var_os("T1_ONLY_RESIZE").is_none() {
        run_clicks_and_overlays(&mut test).await;
        probe_first_click(&mut test).await;
        run_load_phases(&mut test).await;
    }
    let alignment = run_resize_checks(&mut test).await;
    let live_drag = run_live_drag(&mut test).await;

    let phases = {
        let mut phases = SHARED.phases.lock().unwrap();
        if let Some(last) = phases.last_mut() {
            last.ended = Some(Instant::now());
        }
        phases
            .iter()
            .filter(|phase| phase.name != UNMEASURED)
            .map(summarize)
            .collect::<Vec<_>>()
    };
    let passed = test.checks.iter().all(|check| check["pass"] == true);
    let report = json!({
        "passed": passed,
        "inconclusive_clicks_without_focus": test.clicks_without_focus,
        "adapter": *SHARED.adapter.lock().unwrap(),
        "scale_factor": test.window.scale_factor(),
        "gravity": std::env::var("T1_GRAVITY").unwrap_or_else(|_| "topLeft".to_string()),
        "frame_latency": crate::render::frame_latency(),
        "has_screen_recording_permission": macos::has_screen_recording_permission(),
        "checks": test.checks,
        "hit_tests": test.hit_tests,
        "phases": phases,
        "alignment": alignment,
        "live_drag": live_drag,
    });
    let path = out.join("report.json");
    std::fs::write(&path, serde_json::to_vec_pretty(&report).unwrap())
        .expect("couldn't write the report");
    if test.clicks_without_focus > 0 {
        println!(
            "INCONCLUSIVE: {} clicks were sent without focus, so ignore click failures from this run",
            test.clicks_without_focus
        );
    }
    println!("wrote {}", path.display());
    std::process::exit(if passed { 0 } else { 1 });
}

async fn wait_for_startup() {
    let started = Instant::now();
    loop {
        let shown = SHARED
            .phases
            .lock()
            .unwrap()
            .last()
            .map_or(0, |phase| phase.shown);
        if shown > 120 && SHARED.hole.lock().unwrap().is_some() {
            return;
        }
        if started.elapsed() > Duration::from_secs(30) {
            eprintln!("no video or DOM rectangle after 30 seconds, shown {shown}");
            std::process::exit(2);
        }
        sleep(50).await;
    }
}

/// T1's core question: controls are drawn over the video, and clicks reach them.
async fn run_clicks_and_overlays(test: &mut Test) {
    test.phase("baseline", 10).await;

    let before = test.capture("1-baseline");
    let Some(before) = before else {
        test.check("capture", false, "CGWindowListCreateImage returned nothing");
        return;
    };
    test.check(
        "capture",
        true,
        format!("{}x{} from the window server", before.width, before.height),
    );
    let title = test.title_bar_height(&before);
    let hole = test
        .rect("#hole")
        .await
        .expect("the hole is always in the DOM");

    sleep(200).await;
    if let Some(later) = test.capture("") {
        let moved = mean_difference(&before, &later, &inset_points(&hole, title, 12));
        test.check(
            "video visible through the clear area",
            moved > 1.0,
            format!("two captures 200 ms apart differ by {moved:.1}/255 inside the hole"),
        );
    }

    // A click on bare video has to reach the DOM too, for the drag handles in M7.
    let (x, y) = (hole.x + hole.w * 0.25, hole.y + hole.h * 0.4);
    let view = test.click("clear area", x, y).await;
    let click = test.last_click().await;
    let landed = click["id"] == "hole" && (click["x"].as_f64().unwrap_or(-1.0) - x).abs() <= 1.0;
    test.check(
        "click on the clear area reaches the DOM",
        landed,
        format!("AppKit view {view}, DOM saw {click}"),
    );

    // Popover over the video.
    let view = test.click_element("popover trigger", POPOVER_TRIGGER).await;
    let state = test.state(POPOVER).await;
    test.check(
        "popover opens from a click over the video",
        state == "open",
        format!("AppKit view {view}, data-state {state}"),
    );

    if let Some(rect) = test.rect(POPOVER).await {
        let overlaps = rect.x >= hole.x
            && rect.y >= hole.y
            && rect.x + rect.w <= hole.x + hole.w
            && rect.y + rect.h <= hole.y + hole.h;
        test.check(
            "popover sits over the video",
            overlaps,
            format!(
                "popover {:.0},{:.0} {:.0}x{:.0}",
                rect.x, rect.y, rect.w, rect.h
            ),
        );
        let open = test.capture("2-popover");
        sleep(200).await;
        let later = test.capture("");
        if let (Some(open), Some(later)) = (open, later) {
            let points = corner_points(&rect, title, 12);
            let covered = mean_difference(&before, &open, &points);
            let still = mean_difference(&open, &later, &points);
            let video = mean_difference(&open, &later, &inset_points(&hole, title, 12));
            test.check(
                "popover is drawn over the live video",
                covered > 5.0 && still < 1.0 && video > 1.0,
                format!("changed {covered:.1}/255 when it opened, {still:.1} while open, video moved {video:.1}"),
            );
        }
    }
    test.phase("popover-open", 5).await;

    let view = test
        .click_element(
            "button inside popover",
            "[data-name='PopoverContent'] button",
        )
        .await;
    let click = test.last_click().await;
    let state = test.state(POPOVER).await;
    test.check(
        "button inside the popover takes the click",
        click["name"] == "PopoverContent" && state == "open",
        format!("AppKit view {view}, DOM saw {click}, data-state {state}"),
    );

    test.click("clear area, to close", x, y).await;
    let state = test.state(POPOVER).await;
    test.check(
        "clicking the video closes the popover",
        state == "closed",
        format!("data-state {state}"),
    );

    // Dialog with its translucent backdrop.
    let view = test.click_element("dialog trigger", DIALOG_TRIGGER).await;
    let state = test.state(DIALOG).await;
    test.check(
        "dialog opens from a click over the video",
        state == "open",
        format!("AppKit view {view}, data-state {state}"),
    );
    if let Some(rect) = test.rect(DIALOG).await {
        let open = test.capture("3-dialog");
        sleep(200).await;
        let later = test.capture("");
        if let (Some(open), Some(later)) = (open, later) {
            let points = corner_points(&rect, title, 14);
            let still = mean_difference(&open, &later, &points);
            let backdrop = inset_points(&hole, title, 12)
                .into_iter()
                .filter(|&(px, py)| !inside(&rect, px, py, title))
                .collect::<Vec<_>>();
            let moved = mean_difference(&open, &later, &backdrop);
            let dim =
                mean_brightness(&open, &backdrop) / mean_brightness(&before, &backdrop).max(1.0);
            test.check(
                "dialog is drawn over the video, which stays live behind the backdrop",
                still < 1.0 && moved > 0.5 && dim < 0.8,
                format!("panel changed {still:.1}/255, backdrop area moved {moved:.1}, brightness x{dim:.2}"),
            );
        }
    }
    test.phase("dialog-open", 5).await;

    let view = test
        .click_element("dialog close button", DIALOG_CLOSE)
        .await;
    let state = test.state(DIALOG).await;
    test.check(
        "dialog closes from its own button",
        state == "closed",
        format!("AppKit view {view}, data-state {state}"),
    );
}

/// Not a pass or fail: wry defaults to not accepting the click that activates a window, and Dioxus
/// doesn't expose the option. This records what that does, which matters for the launch bar.
///
/// It runs after every other click because macOS may not give focus back to an app that has given
/// it up. A real click would take focus, but a synthetic one can't.
async fn probe_first_click(test: &mut Test) {
    let hole = test
        .rect("#hole")
        .await
        .expect("the hole is always in the DOM");
    let (x, y) = (hole.x + hole.w * 0.25, hole.y + hole.h * 0.4);
    macos::deactivate_app();
    sleep(500).await;
    let key_before = macos::is_visible_and_key(&test.window).1;
    let clicks_before = test
        .eval("return window.__clicks.length;")
        .await
        .as_u64()
        .unwrap_or(0);
    macos::send_mouse(&test.window, x, y, true);
    sleep(40).await;
    macos::send_mouse(&test.window, x, y, false);
    sleep(300).await;
    let clicks_after = test
        .eval("return window.__clicks.length;")
        .await
        .as_u64()
        .unwrap_or(0);
    println!(
        "first click on a background window: key before {key_before}, DOM clicks {clicks_before} -> {clicks_after}"
    );
    test.hit_tests.push(json!({ "target": "first click on a background window", "key_before": key_before, "dom_clicks_added": clicks_after - clicks_before }));
    macos::make_key(&test.window);
    sleep(300).await;
}

/// Is the video independent of what the UI is doing?
async fn run_load_phases(test: &mut Test) {
    SHARED.playhead_on.store(true, Ordering::Relaxed);
    test.phase("playhead-60hz", 5).await;
    SHARED.playhead_on.store(false, Ordering::Relaxed);

    test.note_visibility("busy-dom");
    SHARED.begin_phase("busy-dom");
    test.eval(BUSY_DOM).await;
    SHARED.begin_phase(UNMEASURED);

    // A stuck UI must not stall the picture, so block the thread that owns the window.
    test.note_visibility("main-thread-stalls");
    SHARED.begin_phase("main-thread-stalls");
    for _ in 0..10 {
        std::thread::sleep(Duration::from_millis(100));
        sleep(400).await;
    }
    SHARED.begin_phase(UNMEASURED);
}

async fn run_resize_checks(test: &mut Test) -> Value {
    let mut steps = vec![];
    for (i, &(w, h)) in [(900, 600), (1500, 900), (1023, 677), (1280, 800)]
        .iter()
        .enumerate()
    {
        let requested = test.set_size(w, h).await;
        let lag = wait_for_dom_size(requested, w, h).await;
        sleep(250).await;
        let drawn = *SHARED.drawn.lock().unwrap();
        let dom = *SHARED.hole.lock().unwrap();
        let gaps = test
            .capture(&format!("4-resize-{w}x{h}"))
            .map(|capture| edge_gaps(&capture));
        let aligned = gaps.as_ref().is_some_and(EdgeGaps::aligned) && dom == Some(drawn.0);
        test.check(
            &format!("aligned after settling at {w}x{h}"),
            aligned,
            format!("edge gaps {gaps:?}, DOM {dom:?}, drawn {drawn:?}"),
        );
        steps.push(json!({ "step": i, "size": [w, h], "dom_lag_ms": lag.map(|ms| ms.as_secs_f64() * 1000.0), "gaps": format!("{gaps:?}") }));
    }

    // Continuous resizing with no captures, for clean frame timing.
    test.note_visibility("resize-sweep");
    SHARED.begin_phase("resize-sweep");
    let started = Instant::now();
    let mut lags = vec![];
    while started.elapsed() < Duration::from_secs(6) {
        let (w, h) = sweep_size(started.elapsed().as_secs_f64());
        let requested = test.set_size(w, h).await;
        sleep(16).await;
        if let Some(lag) = dom_report_since(requested, w, h) {
            lags.push(lag.as_secs_f32() * 1000.0);
        }
    }

    // The same again, capturing as it goes, to see the two layers mid-resize.
    test.note_visibility("resize-sweep-captured");
    SHARED.begin_phase(UNMEASURED);
    let started = Instant::now();
    let mut tally = EdgeTally::default();
    while started.elapsed() < Duration::from_secs(6) {
        let (w, h) = sweep_size(started.elapsed().as_secs_f64());
        test.set_size(w, h).await;
        sleep(16).await;
        let Some(capture) = test.capture("") else {
            continue;
        };
        tally.add(&capture, &test.out);
    }

    // How long the layers take to agree once resizing stops.
    let requested = test.set_size(1280, 800).await;
    let mut settled_ms = None;
    while requested.elapsed() < Duration::from_secs(1) {
        if test
            .capture("")
            .is_some_and(|capture| edge_gaps(&capture).aligned())
        {
            settled_ms = Some(requested.elapsed().as_secs_f64() * 1000.0);
            break;
        }
        sleep(4).await;
    }
    test.check(
        "aligned again after continuous resizing",
        settled_ms.is_some(),
        format!("settled in {settled_ms:?} ms"),
    );

    json!({
        "settled_steps": steps,
        "sweep_dom_lag_ms": percentiles(&lags),
        "sweep_captures": tally.summary(),
        "settle_after_sweep_ms": settled_ms,
    })
}

/// Drives a real live resize: posted mouse events put AppKit in the modal loop it runs when someone
/// drags a window edge, which `set_inner_size` never enters. Everything measured here happens on
/// another thread, so it still works if that loop stops the UI from running.
async fn run_live_drag(test: &mut Test) -> Value {
    let (w, h) = (1100.0, 700.0);
    test.set_size(w as u32, h as u32).await;
    macos::make_key(&test.window);
    sleep(500).await;

    let number = macos::window_number(&test.window);
    let address = macos::window_address(&test.window);
    let reports_before = SHARED.hole_reports.lock().unwrap().len();
    let out = test.out.clone();
    SHARED.begin_phase("live-drag");
    let drag = std::thread::spawn(move || drag_right_edge(number, address, w, h, &out));
    while !drag.is_finished() {
        sleep(50).await;
    }
    SHARED.begin_phase(UNMEASURED);
    let result = drag.join().unwrap_or(Value::Null);
    let reports = SHARED.hole_reports.lock().unwrap().len() - reports_before;

    let live = result["samples_in_live_resize"].as_u64().unwrap_or(0);
    test.check(
        "drove a real live resize",
        live > 0,
        format!("{live} captures with inLiveResize set, {reports} DOM reports during the drag"),
    );

    // Does it recover once the drag ends? The DOM should report the final size and agree with wgpu.
    sleep(400).await;
    let settled = test
        .capture("6-after-live-drag")
        .map(|capture| edge_gaps(&capture));
    let aligned = settled.as_ref().is_some_and(EdgeGaps::aligned);
    test.check(
        "aligned after a live resize",
        aligned,
        format!("edge gaps {settled:?}"),
    );

    json!({ "result": result, "dom_reports_during_drag": reports })
}

fn drag_right_edge(number: isize, address: usize, w: f64, h: f64, out: &std::path::Path) -> Value {
    use objc2_app_kit::NSEventType;

    // AppKit's resize zone straddles the window edge, so try on it and then just outside it.
    let y = h / 2.0;
    let mut grabbed_at = None;
    for x in [w - 1.0, w, w + 1.0, w + 2.0] {
        macos::post_mouse(number, NSEventType::LeftMouseDown, x, y);
        std::thread::sleep(Duration::from_millis(200));
        macos::post_mouse(number, NSEventType::LeftMouseDragged, x - 20.0, y);
        std::thread::sleep(Duration::from_millis(200));
        if macos::in_live_resize(address) {
            grabbed_at = Some(x);
            break;
        }
        macos::post_mouse(number, NSEventType::LeftMouseUp, x, y);
        std::thread::sleep(Duration::from_millis(200));
    }
    let Some(x) = grabbed_at else {
        return json!({ "driven": false, "samples_in_live_resize": 0 });
    };

    let mut tally = EdgeTally::default();
    let mut in_live_resize = 0;
    let mut widths = (usize::MAX, 0);
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(6) {
        let phase = (started.elapsed().as_secs_f64() * std::f64::consts::TAU / 2.0).sin();
        macos::post_mouse(
            number,
            NSEventType::LeftMouseDragged,
            x - 20.0 + 250.0 * phase,
            y,
        );
        std::thread::sleep(Duration::from_millis(16));
        if macos::in_live_resize(address) {
            in_live_resize += 1;
        }
        if let Some(capture) = macos::capture_number(number) {
            widths = (widths.0.min(capture.width), widths.1.max(capture.width));
            tally.add(&capture, out);
        }
    }
    macos::post_mouse(number, NSEventType::LeftMouseUp, x, y);
    std::thread::sleep(Duration::from_millis(100));

    json!({
        "driven": true,
        "grabbed_at_x": x,
        "samples_in_live_resize": in_live_resize,
        "capture_width_range": [widths.0, widths.1],
        "edges": tally.summary(),
    })
}

/// A size that varies smoothly, like a window edge being dragged back and forth.
fn sweep_size(t: f64) -> (u32, u32) {
    let phase = (t * std::f64::consts::TAU / 2.0).sin();
    (
        (1100.0 + 300.0 * phase) as u32,
        (720.0 + 140.0 * phase) as u32,
    )
}

async fn wait_for_dom_size(since: Instant, width: u32, height: u32) -> Option<Duration> {
    while since.elapsed() < Duration::from_secs(2) {
        if let Some(lag) = dom_report_since(since, width, height) {
            return Some(lag);
        }
        sleep(1).await;
    }
    None
}

fn dom_report_since(since: Instant, width: u32, height: u32) -> Option<Duration> {
    let reports = SHARED.hole_reports.lock().unwrap();
    reports
        .iter()
        .rev()
        .take_while(|report| report.at >= since)
        .filter(|report| report.inner_w == width && report.inner_h == height)
        .last()
        .map(|report| report.at.duration_since(since))
}

fn summarize(phase: &PhaseStats) -> Value {
    let seconds = match (phase.started, phase.ended) {
        (Some(start), Some(end)) => end.duration_since(start).as_secs_f64(),
        _ => 0.0,
    };
    let median = {
        let mut sorted = phase.present_ms.clone();
        sorted.sort_by(f32::total_cmp);
        sorted.get(sorted.len() / 2).copied().unwrap_or(0.0)
    };
    // A present that took more than one and a half vsync intervals missed a refresh.
    let long = phase
        .present_ms
        .iter()
        .filter(|&&ms| ms > median * 1.5)
        .count();
    json!({
        "name": phase.name,
        "seconds": seconds,
        "presents": phase.present_ms.len(),
        "present_fps": phase.present_ms.len() as f64 / seconds.max(0.001),
        "present_ms": percentiles(&phase.present_ms),
        "long_presents": long,
        "video_frames_shown": phase.shown,
        "video_fps": phase.shown as f64 / seconds.max(0.001),
        "video_frames_dropped": phase.dropped,
        "underruns": phase.underruns,
        "occluded": phase.occluded,
        "reconfigures": phase.reconfigures,
        "upload_ms": percentiles(&phase.upload_ms),
    })
}

fn percentiles(values: &[f32]) -> Value {
    if values.is_empty() {
        return Value::Null;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(f32::total_cmp);
    let at = |fraction: f64| sorted[((sorted.len() - 1) as f64 * fraction) as usize];
    json!({ "p50": at(0.5), "p95": at(0.95), "p99": at(0.99), "max": sorted[sorted.len() - 1] })
}

// ---- Reading captured pixels ----

/// Points in capture pixels across the middle of a rectangle given in CSS pixels. These assume a
/// scale factor of 1, and the self-test reports the scale it ran at.
fn inset_points(rect: &Css, title: usize, step_count: usize) -> Vec<(usize, usize)> {
    let mut points = vec![];
    for i in 1..step_count {
        for j in 1..step_count {
            let x = rect.x + rect.w * i as f64 / step_count as f64;
            let y = rect.y + rect.h * j as f64 / step_count as f64;
            points.push((x as usize, y as usize + title));
        }
    }
    points
}

/// Points just inside each corner, which are empty panel background on the popover and dialog.
fn corner_points(rect: &Css, title: usize, inset: usize) -> Vec<(usize, usize)> {
    let (left, top) = (rect.x as usize + inset, rect.y as usize + inset + title);
    let (right, bottom) = (
        (rect.x + rect.w) as usize - inset,
        (rect.y + rect.h) as usize - inset + title,
    );
    vec![
        (left, bottom),
        (right, bottom),
        (left, top + 2),
        (left + 4, bottom),
    ]
    .into_iter()
    .chain([(right, bottom - 2), (right - 4, bottom)])
    .collect()
}

fn inside(rect: &Css, x: usize, y: usize, title: usize) -> bool {
    let (x, y) = (x as f64, y as f64 - title as f64);
    x >= rect.x && x <= rect.x + rect.w && y >= rect.y && y <= rect.y + rect.h
}

fn mean_difference(a: &Capture, b: &Capture, points: &[(usize, usize)]) -> f64 {
    let mut total = 0.0;
    for &(x, y) in points {
        let (pa, pb) = (a.pixel(x, y), b.pixel(x, y));
        for channel in 0..3 {
            total += (f64::from(pa[channel]) - f64::from(pb[channel])).abs();
        }
    }
    total / (points.len().max(1) * 3) as f64
}

fn mean_brightness(capture: &Capture, points: &[(usize, usize)]) -> f64 {
    let total: f64 = points
        .iter()
        .map(|&(x, y)| {
            capture
                .pixel(x, y)
                .iter()
                .map(|&c| f64::from(c))
                .sum::<f64>()
                / 3.0
        })
        .sum();
    total / points.len().max(1) as f64
}

/// Thresholds rather than exact colours, because the capture is in the display's colour space.
fn is_lime(p: [u8; 3]) -> bool {
    p[1] > 190 && p[0] < 170 && p[2] < 140
}

fn is_magenta(p: [u8; 3]) -> bool {
    p[0] > 170 && p[2] > 170 && p[1] < 130
}

/// For each edge, the pixels between the DOM's lime outline and wgpu's magenta ring. Zero means the
/// layers agree exactly. `None` means no ring was found near the outline, usually because it's
/// hidden under a panel or smeared by a surface being stretched mid-resize.
#[derive(Debug)]
struct EdgeGaps {
    left: Option<i32>,
    right: Option<i32>,
    top: Option<i32>,
    bottom: Option<i32>,
}

impl EdgeGaps {
    fn aligned(&self) -> bool {
        [self.left, self.right, self.top, self.bottom] == [Some(0); 4]
    }
}

/// Sorts mid-resize captures by what a person would actually see at each edge.
#[derive(Default)]
struct EdgeTally {
    samples: u32,
    all_exact: u32,
    saved_example: bool,
    edges: [EdgeCounts; 4],
}

#[derive(Default)]
struct EdgeCounts {
    exact: u32,
    /// The ring isn't next to the outline, and neither is any background. Either wgpu's rectangle
    /// runs on under an opaque panel, which looks fine, or the ring is smeared by stretching.
    ring_not_visible: u32,
    /// A band of background this wide shows between the video and the panel.
    gap_px: Vec<f32>,
}

impl EdgeTally {
    fn add(&mut self, capture: &Capture, out: &std::path::Path) {
        let gaps = edge_gaps(capture);
        self.samples += 1;
        self.all_exact += u32::from(gaps.aligned());
        let values = [gaps.left, gaps.right, gaps.top, gaps.bottom];
        for (counts, value) in self.edges.iter_mut().zip(values) {
            match value {
                Some(0) => counts.exact += 1,
                Some(gap) => counts.gap_px.push(gap as f32),
                None => counts.ring_not_visible += 1,
            }
        }
        if !self.saved_example && !gaps.aligned() && self.samples > 5 {
            self.saved_example = true;
            let _ = capture.save(&out.join("5-mid-resize.png"));
        }
    }

    fn summary(&self) -> Value {
        let edge = |counts: &EdgeCounts| {
            json!({
                "exact": counts.exact,
                "ring_not_visible": counts.ring_not_visible,
                "gaps": counts.gap_px.len(),
                "gap_px": percentiles(&counts.gap_px),
            })
        };
        json!({
            "samples": self.samples,
            "all_four_exact": self.all_exact,
            "left": edge(&self.edges[0]),
            "right": edge(&self.edges[1]),
            "top": edge(&self.edges[2]),
            "bottom": edge(&self.edges[3]),
        })
    }
}

fn edge_gaps(capture: &Capture) -> EdgeGaps {
    let row = capture.height * 55 / 100;
    let column = capture.width * 60 / 100;
    let across = |x: usize| capture.pixel(x, row);
    let down = |y: usize| capture.pixel(column, y);
    EdgeGaps {
        left: scan((0..capture.width).map(across)),
        right: scan((0..capture.width).rev().map(across)),
        top: scan((0..capture.height).map(down)),
        bottom: scan((0..capture.height).rev().map(down)),
    }
}

/// Walks inward from outside the hole. A real gap can only be the grey background, so anything else
/// after the outline means the ring isn't next to it, rather than being video that looks like a ring.
fn scan(pixels: impl Iterator<Item = [u8; 3]>) -> Option<i32> {
    let mut seen_lime = false;
    let mut gap = 0;
    for pixel in pixels {
        if !seen_lime {
            seen_lime = is_lime(pixel);
        } else if is_lime(pixel) {
            continue;
        } else if is_magenta(pixel) {
            return Some(gap);
        } else if is_background(pixel) {
            gap += 1;
        } else {
            return None;
        }
    }
    None
}

fn is_background(p: [u8; 3]) -> bool {
    let (max, min) = (p.iter().max().copied(), p.iter().min().copied());
    max.zip(min)
        .is_some_and(|(max, min)| max < 100 && max - min < 24)
}
