//! T1 from BUILD_PLAN.md: video in a wgpu surface under a transparent Dioxus webview, with real
//! Rust/UI controls drawn over it. See README.md for how to run it.

use std::time::Duration;

use dioxus::desktop::tao::dpi::LogicalSize;
use dioxus::desktop::tao::event::{Event, WindowEvent};
use dioxus::desktop::{Config, WindowBuilder, use_wry_event_handler};
use dioxus::prelude::*;

mod render;
mod video;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
mod self_test;

// The same vendored files Reel uses, so this tests the components that will really ship.
#[rustfmt::skip]
#[allow(dead_code, unused_imports, clippy::all)]
#[path = "../../../crates/reel-app/src/hooks/mod.rs"]
mod hooks;
#[rustfmt::skip]
#[allow(dead_code, unused_imports, clippy::all)]
#[path = "../../../crates/reel-app/src/ui/mod.rs"]
mod ui;

use render::{Rect, SHARED};
use ui::button::{Button, ButtonVariant};
use ui::dialog::{
    Dialog, DialogAction, DialogClose, DialogContent, DialogDescription, DialogFooter,
    DialogHeader, DialogTitle, DialogTrigger,
};
use ui::popover::{Popover, PopoverContent, PopoverDescription, PopoverTitle, PopoverTrigger};

const TAILWIND_CSS: &str = include_str!("../../../crates/reel-app/assets/tailwind.css");

const SHELL_CSS: &str = r#"
html, body, #main { background: transparent !important; height: 100%; margin: 0; overflow: hidden; }
.shell { display: grid; height: 100vh; grid-template: 44px minmax(0, 1fr) 120px / 240px minmax(0, 1fr) 24px; }
.panel { background: var(--background); color: var(--foreground); }
.head { grid-column: 1 / 4; display: flex; align-items: center; justify-content: space-between; padding: 0 12px; border-bottom: 1px solid var(--border); font-size: 13px; }
.hud { font: 11px ui-monospace, monospace; color: var(--muted-foreground); }
.side { padding: 12px; font-size: 13px; }
.timeline { grid-column: 1 / 4; position: relative; overflow: hidden; }
.playhead { position: absolute; top: 0; bottom: 0; width: 2px; background: var(--primary); }
.chip { position: absolute; display: block; width: 18px; height: 6px; background: var(--muted-foreground); }
#hole { position: relative; z-index: 1; box-shadow: 0 0 0 1px #00ff00; }
.controls { position: absolute; left: 0; right: 0; bottom: 16px; display: flex; justify-content: center; gap: 8px; pointer-events: none; }
.controls > * { pointer-events: auto; }
"#;

/// Reports where `#hole` is whenever it moves or changes size.
const OBSERVE_HOLE: &str = r#"
const start = () => {
    const hole = document.getElementById('hole');
    if (!hole) { setTimeout(start, 10); return; }
    const send = () => {
        const r = hole.getBoundingClientRect();
        dioxus.send({ x: r.x, y: r.y, w: r.width, h: r.height, dpr: devicePixelRatio, iw: innerWidth, ih: innerHeight });
    };
    new ResizeObserver(send).observe(hole);
    addEventListener('resize', send);
    send();
};
start();
await new Promise(() => {});
"#;

fn main() {
    let ffmpeg = std::env::var("T1_FFMPEG").unwrap_or_else(|_| "ffmpeg".to_string());
    let video = std::env::var("T1_VIDEO").expect("set T1_VIDEO to a 1080p60 video file");

    let window = WindowBuilder::new()
        .with_title("T1 Layering")
        .with_transparent(true)
        .with_inner_size(LogicalSize::new(1280.0, 800.0));

    let config = Config::new()
        .with_window(window)
        .with_as_child_window()
        .with_background_color((0, 0, 0, 0))
        .with_custom_head(format!("<style>{TAILWIND_CSS}{SHELL_CSS}</style>"))
        .with_on_window(move |window, _dom| {
            render::start(window, ffmpeg.clone(), video.clone());
        });

    dioxus::LaunchBuilder::desktop()
        .with_cfg(config)
        .launch(App);
}

#[component]
fn App() -> Element {
    let mut hud = use_signal(render::Hud::default);
    let mut playhead = use_signal(|| 0.0_f64);

    use_wry_event_handler(|event, _| {
        if let Event::WindowEvent {
            event: WindowEvent::Resized(size),
            ..
        } = event
        {
            SHARED.set_surface_size(size.width, size.height);
        }
    });

    use_future(|| async {
        let mut reports = document::eval(OBSERVE_HOLE);
        while let Ok(value) = reports.recv::<serde_json::Value>().await {
            record_hole(&value);
        }
    });

    use_future(move || async move {
        loop {
            tokio::time::sleep(Duration::from_millis(250)).await;
            hud.set(*SHARED.hud.lock().unwrap());
        }
    });

    // Moves one element at about 60 Hz, as the real playhead will during playback.
    use_future(move || async move {
        loop {
            tokio::time::sleep(Duration::from_millis(16)).await;
            if SHARED
                .playhead_on
                .load(std::sync::atomic::Ordering::Relaxed)
            {
                playhead.set((playhead() + 2.0) % 1000.0);
            }
        }
    });

    #[cfg(target_os = "macos")]
    use_future(|| async {
        if std::env::var_os("T1_SELF_TEST").is_some() {
            self_test::run().await;
        }
    });

    let readout = {
        let h = hud();
        format!(
            "{:.0} fps, worst {:.1} ms, shown {}, dropped {}, surface {}x{}, hole {},{} {}x{}",
            h.present_fps,
            h.worst_ms,
            h.shown,
            h.dropped,
            h.surface.0,
            h.surface.1,
            h.hole.x,
            h.hole.y,
            h.hole.w,
            h.hole.h,
        )
    };

    rsx! {
        div { class: "shell",
            header { class: "panel head",
                span { "T1 layering" }
                span { class: "hud", "{readout}" }
            }
            aside { class: "panel side",
                p { "DOM panel. The video is in a wgpu surface underneath this webview." }
            }
            div { id: "hole",
                div { class: "controls",
                    Popover {
                        PopoverTrigger { "Open popover" }
                        PopoverContent {
                            PopoverTitle { "Rust/UI popover" }
                            PopoverDescription { "Drawn by the webview, over the wgpu video." }
                            Button { variant: ButtonVariant::Outline, class: "mt-3", "Inside" }
                        }
                    }
                    Dialog {
                        DialogTrigger { "Open dialog" }
                        DialogContent {
                            DialogHeader {
                                DialogTitle { "Rust/UI dialog" }
                                DialogDescription { "The video keeps playing behind the backdrop." }
                            }
                            DialogFooter { class: "mt-4",
                                DialogClose { "Close" }
                                DialogAction { "OK" }
                            }
                        }
                    }
                }
            }
            div { class: "panel" }
            footer { class: "panel timeline",
                div { class: "playhead", style: "left: {playhead}px" }
            }
        }
    }
}

fn record_hole(value: &serde_json::Value) {
    let number = |key: &str| value[key].as_f64().unwrap_or(0.0);
    let dpr = number("dpr");
    let physical = |css: f64| (css * dpr).round().max(0.0) as u32;
    let rect = Rect {
        x: physical(number("x")),
        y: physical(number("y")),
        w: physical(number("w")),
        h: physical(number("h")),
    };
    *SHARED.hole.lock().unwrap() = Some(rect);
    SHARED
        .hole_reports
        .lock()
        .unwrap()
        .push(render::HoleReport {
            at: std::time::Instant::now(),
            inner_w: number("iw") as u32,
            inner_h: number("ih") as u32,
        });
}
