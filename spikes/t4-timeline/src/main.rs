//! T4 from BUILD_PLAN.md: a timeline with 500 items and a moving playhead, with all state in Rust,
//! comparing DOM items with a canvas. Measures latency across the webview boundary while it plays,
//! while an item is dragged, and while a zoom repositions everything.

use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use dioxus::desktop::tao::dpi::LogicalSize;
use dioxus::desktop::{Config, WindowBuilder};
use dioxus::prelude::*;
use serde_json::{Value, json};

mod macos;

const ROWS: usize = 6;
const ROW_HEIGHT: f64 = 40.0;
const ITEM_WIDTH: f64 = 60.0;

const CSS: &str = r#"
html, body, #main { margin: 0; height: 100%; overflow: hidden; background: #111; font: 11px -apple-system, sans-serif; }
#tl { position: relative; width: 100%; height: 100vh; overflow: hidden; user-select: none; }
.item { position: absolute; height: 32px; background: #2563eb; color: #fff; border-radius: 4px; overflow: hidden; white-space: nowrap; box-sizing: border-box; padding: 2px 4px; }
.item.drag { background: #f43f5e; z-index: 2; }
.playhead { position: absolute; top: 0; bottom: 0; width: 2px; background: #3d8bff; z-index: 3; }
canvas { display: block; }
"#;

/// Records how smoothly the page paints, and when each mouse move left JavaScript.
const INSTRUMENT: &str = r#"
window.__m = { raf: [], moves: new Map(), drag: [], play: [], mouse: 0, items: 0 };
let last = performance.now();
const frame = now => { __m.raf.push(now - last); last = now; requestAnimationFrame(frame); };
requestAnimationFrame(frame);
document.addEventListener('mousemove', e => { __m.mouse++; __m.moves.set(Math.round(e.clientX), performance.now()); }, true);
window.__sawDrag = x => {
    const t = __m.moves.get(Math.round(x));
    if (t !== undefined) { __m.drag.push(performance.now() - t); __m.moves.delete(Math.round(x)); }
};
window.__sawPlay = stamp => { __m.play.push(Date.now() - stamp); };
window.__take = () => {
    const out = { raf: __m.raf, drag: __m.drag, play: __m.play, mouse: __m.mouse, items: __m.items };
    __m.raf = []; __m.drag = []; __m.play = []; __m.mouse = 0; __m.items = 0;
    return out;
};
return true;
"#;

/// In DOM mode, watches what Rust's changes do to the page.
const WATCH_DOM: &str = r#"
const start = () => {
    const tl = document.getElementById('tl');
    if (!tl) { setTimeout(start, 20); return; }
    new MutationObserver(records => {
        for (const r of records) {
            const el = r.target;
            if (el.classList.contains('item')) __m.items++;
            if (el.classList.contains('playhead')) __sawPlay(Number(el.dataset.stamp));
            else if (el.classList.contains('drag')) __sawDrag(parseFloat(el.style.left));
        }
    }).observe(tl, { subtree: true, attributes: true, attributeFilter: ['style', 'data-stamp'] });
};
start();
return true;
"#;

/// In canvas mode, JavaScript only draws what Rust sends it. Mouse moves still go through Rust.
const CANVAS: &str = r#"
let canvas;
while (!(canvas = document.getElementById('c'))) await new Promise(done => setTimeout(done, 10));
const ctx = canvas.getContext('2d', { alpha: false });
const items = await dioxus.recv();
let zoom = 1, playhead = 0, dragIndex = -1, dirty = true;
const resize = () => { canvas.width = innerWidth * devicePixelRatio; canvas.height = innerHeight * devicePixelRatio; dirty = true; };
addEventListener('resize', resize); resize();
const draw = () => {
    if (dirty) {
        dirty = false;
        ctx.setTransform(devicePixelRatio, 0, 0, devicePixelRatio, 0, 0);
        ctx.fillStyle = '#111';
        ctx.fillRect(0, 0, innerWidth, innerHeight);
        ctx.font = '11px -apple-system, sans-serif';
        ctx.textBaseline = 'top';
        for (let i = 0; i < items.length; i++) {
            const it = items[i];
            const x = i === dragIndex ? it.x : it.x * zoom;
            const w = it.w * zoom;
            ctx.fillStyle = i === dragIndex ? '#f43f5e' : '#2563eb';
            ctx.beginPath();
            ctx.roundRect(x, it.y, w, 32, 4);
            ctx.fill();
            ctx.fillStyle = '#fff';
            ctx.fillText('Clip ' + i, x + 4, it.y + 4);
        }
        ctx.fillStyle = '#3d8bff';
        ctx.fillRect(playhead * zoom, 0, 2, innerHeight);
    }
    requestAnimationFrame(draw);
};
requestAnimationFrame(draw);
canvas.addEventListener('mousemove', e => dioxus.send({ kind: 'move', x: e.clientX }));
canvas.addEventListener('mousedown', e => dioxus.send({ kind: 'down', x: e.clientX }));
canvas.addEventListener('mouseup', e => dioxus.send({ kind: 'up', x: e.clientX }));
while (true) {
    const m = await dioxus.recv();
    if (m.kind === 'play') { playhead = m.x; __sawPlay(m.stamp); }
    else if (m.kind === 'zoom') { zoom = m.z; }
    else if (m.kind === 'drag') { dragIndex = m.i; items[m.i].x = m.x; __sawDrag(m.x); }
    dirty = true;
}
"#;

#[derive(Clone, Copy, PartialEq)]
struct Item {
    x: f64,
    y: f64,
}

fn item_count() -> usize {
    std::env::var("T4_ITEMS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(500)
}

fn initial_item(i: usize) -> Item {
    Item {
        x: (i / ROWS) as f64 * (ITEM_WIDTH + 4.0) + 20.0,
        y: (i % ROWS) as f64 * ROW_HEIGHT + 8.0,
    }
}

static PLAYING: AtomicBool = AtomicBool::new(false);
static ZOOMING: AtomicBool = AtomicBool::new(false);
static TICKS: AtomicI64 = AtomicI64::new(0);

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |t| t.as_millis() as i64)
}

fn main() {
    let window = WindowBuilder::new()
        .with_title("T4 Timeline")
        .with_always_on_top(true)
        .with_inner_size(LogicalSize::new(1280.0, 280.0));
    let config = Config::new()
        .with_window(window)
        .with_custom_head(format!("<style>{CSS}</style>"));
    dioxus::LaunchBuilder::desktop()
        .with_cfg(config)
        .launch(App);
}

#[component]
fn App() -> Element {
    let canvas = std::env::var("T4_MODE").as_deref() == Ok("canvas");
    use_hook(|| document::eval(INSTRUMENT));
    if canvas {
        rsx! { CanvasTimeline {} }
    } else {
        rsx! { DomTimeline {} }
    }
}

#[component]
fn DomTimeline() -> Element {
    let items = use_hook(|| {
        (0..item_count())
            .map(|i| Signal::new(initial_item(i)))
            .collect::<Vec<_>>()
    });
    let mut zoom = use_signal(|| 1.0_f64);
    let mut playhead = use_signal(|| (0.0_f64, 0_i64));
    let mut dragging = use_signal(|| None::<usize>);

    use_future(move || async move {
        document::eval(WATCH_DOM).await.ok();
        let mut x = 0.0;
        let mut phase = 0.0_f64;
        let mut clock = tokio::time::interval(Duration::from_secs_f64(1.0 / 60.0));
        loop {
            clock.tick().await;
            if PLAYING.load(Ordering::Relaxed) {
                x = (x + 2.0) % 1200.0;
                playhead.set((x, now_ms()));
                TICKS.fetch_add(1, Ordering::Relaxed);
            }
            if ZOOMING.load(Ordering::Relaxed) {
                phase += 0.05;
                zoom.set(1.0 + 0.3 * phase.sin());
            }
        }
    });
    use_future(|| async { self_test().await });

    let drag_items = items.clone();
    rsx! {
        div {
            id: "tl",
            onmousedown: move |_| dragging.set(Some(0)),
            onmouseup: move |_| dragging.set(None),
            onmousemove: move |event| {
                if let Some(index) = dragging() {
                    let mut item = drag_items[index];
                    let y = item.read().y;
                    item.set(Item { x: event.client_coordinates().x, y });
                }
            },
            for (index, item) in items.iter().enumerate() {
                DomItem { key: "{index}", index, item: *item, zoom, dragging }
            }
            DomPlayhead { playhead, zoom }
        }
    }
}

#[component]
fn DomItem(
    index: usize,
    item: Signal<Item>,
    zoom: Signal<f64>,
    dragging: Signal<Option<usize>>,
) -> Element {
    let it = item();
    let z = zoom();
    let is_drag = dragging() == Some(index);
    // A dragged item stays at the pointer, so its left value identifies the mouse move behind it.
    let x = if is_drag { it.x } else { it.x * z };
    let w = ITEM_WIDTH * z;
    let class = if is_drag { "item drag" } else { "item" };
    rsx! {
        div { class, style: "left:{x}px;top:{it.y}px;width:{w}px", "Clip {index}" }
    }
}

#[component]
fn DomPlayhead(playhead: Signal<(f64, i64)>, zoom: Signal<f64>) -> Element {
    let (x, stamp) = playhead();
    let left = x * zoom();
    rsx! { div { class: "playhead", "data-stamp": "{stamp}", style: "left:{left}px" } }
}

#[component]
fn CanvasTimeline() -> Element {
    use_future(|| async {
        let mut channel = document::eval(CANVAS);
        let items: Vec<Value> = (0..item_count())
            .map(|i| {
                let it = initial_item(i);
                json!({ "x": it.x, "y": it.y, "w": ITEM_WIDTH })
            })
            .collect();
        channel.send(json!(items)).ok();
        let sender = channel;
        spawn(async move {
            let mut x = 0.0;
            let mut phase = 0.0_f64;
            let mut clock = tokio::time::interval(Duration::from_secs_f64(1.0 / 60.0));
            loop {
                clock.tick().await;
                if PLAYING.load(Ordering::Relaxed) {
                    x = (x + 2.0) % 1200.0;
                    sender
                        .send(json!({ "kind": "play", "x": x, "stamp": now_ms() }))
                        .ok();
                    TICKS.fetch_add(1, Ordering::Relaxed);
                }
                if ZOOMING.load(Ordering::Relaxed) {
                    phase += 0.05;
                    sender
                        .send(json!({ "kind": "zoom", "z": 1.0 + 0.3 * phase.sin() }))
                        .ok();
                }
            }
        });
        let mut dragging = false;
        while let Ok(message) = channel.recv::<Value>().await {
            match message["kind"].as_str() {
                Some("down") => dragging = true,
                Some("up") => dragging = false,
                Some("move") if dragging => {
                    channel
                        .send(json!({ "kind": "drag", "i": 0, "x": message["x"] }))
                        .ok();
                }
                _ => {}
            }
        }
    });
    use_future(|| async { self_test().await });
    rsx! {
        canvas { id: "c" }
    }
}

async fn sleep(ms: u64) {
    tokio::time::sleep(Duration::from_millis(ms)).await;
}

fn summary(values: &[f64]) -> Value {
    if values.is_empty() {
        return Value::Null;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let at = |fraction: f64| sorted[((sorted.len() - 1) as f64 * fraction) as usize];
    json!({ "count": sorted.len(), "p50": at(0.5), "p95": at(0.95), "p99": at(0.99), "max": sorted[sorted.len() - 1] })
}

async fn measure(name: &str, seconds: u64) -> Value {
    document::eval("__take(); return true;").await.ok();
    TICKS.store(0, Ordering::Relaxed);
    sleep(seconds * 1000).await;
    let taken = document::eval("return __take();")
        .await
        .unwrap_or(Value::Null);
    let list = |key: &str| -> Vec<f64> {
        taken[key].as_array().map_or(vec![], |values| {
            values.iter().filter_map(Value::as_f64).collect()
        })
    };
    let raf = list("raf");
    let median = {
        let mut sorted = raf.clone();
        sorted.sort_by(f64::total_cmp);
        sorted.get(sorted.len() / 2).copied().unwrap_or(0.0)
    };
    let long = raf.iter().filter(|&&ms| ms > median * 1.5).count();
    let fps = raf.len() as f64 / seconds as f64;
    let result = json!({
        "name": name,
        "paint_fps": fps,
        "missed_refreshes": long,
        "paint_ms": summary(&raf),
        "drag_latency_ms": summary(&list("drag")),
        "playhead_lag_ms": summary(&list("play")),
        "mouse_moves": taken["mouse"],
        "item_changes_per_second": taken["items"].as_f64().unwrap_or(0.0) / seconds as f64,
        "rust_ticks_per_second": TICKS.load(Ordering::Relaxed) as f64 / seconds as f64,
    });
    println!("{result}");
    result
}

async fn self_test() {
    if std::env::var_os("T4_SELF_TEST").is_none() {
        return;
    }
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_secs(90));
        eprintln!("self-test timed out");
        std::process::exit(3);
    });
    let out = std::path::PathBuf::from(std::env::var("T4_OUT").unwrap_or_else(|_| ".".into()));
    std::fs::create_dir_all(&out).expect("couldn't create the output folder");
    let mode = std::env::var("T4_MODE").unwrap_or_else(|_| "dom".into());
    sleep(1500).await;
    let window = dioxus::desktop::window().window.clone();
    macos::make_key(&window);
    sleep(300).await;

    let mut phases = vec![];
    phases.push(measure("idle", 3).await);

    PLAYING.store(true, Ordering::Relaxed);
    phases.push(measure("playing", 5).await);

    // Dragging while playing: mouse events at 125 Hz through AppKit, each with a distinct position.
    let number = macos::window_number(&window);
    let height = 280.0;
    let row_y = height - 20.0;
    macos::post_mouse(number, macos::Kind::Down, 200.0, row_y);
    sleep(100).await;
    let mover = std::thread::spawn(move || {
        let started = std::time::Instant::now();
        let mut x = 200.0;
        while started.elapsed() < Duration::from_secs(6) {
            macos::post_mouse(number, macos::Kind::Drag, x, row_y);
            x = if x >= 1100.0 { 200.0 } else { x + 1.0 };
            std::thread::sleep(Duration::from_millis(8));
        }
    });
    phases.push(measure("dragging-while-playing", 5).await);
    while !mover.is_finished() {
        sleep(50).await;
    }
    macos::post_mouse(number, macos::Kind::Up, 200.0, row_y);
    sleep(200).await;

    ZOOMING.store(true, Ordering::Relaxed);
    phases.push(measure("zooming-all-500-while-playing", 5).await);
    ZOOMING.store(false, Ordering::Relaxed);
    PLAYING.store(false, Ordering::Relaxed);

    let report = json!({ "mode": mode, "items": item_count(), "phases": phases });
    std::fs::write(
        out.join(format!("report-{mode}-{}.json", item_count())),
        serde_json::to_vec_pretty(&report).unwrap(),
    )
    .expect("couldn't write the report");
    std::process::exit(0);
}
