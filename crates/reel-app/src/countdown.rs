//! The small windows around starting a recording: the countdown, and the flash around a chosen source.

use std::time::Duration;

use dioxus::desktop::tao::dpi::{LogicalPosition, LogicalSize};
use dioxus::desktop::{Config, WindowBuilder};
use dioxus::prelude::*;

use crate::links::{self, ToBar};
use crate::native;

const COUNTDOWN_SIZE: f64 = 200.0;
const HIGHLIGHT_MS: u64 = 1700;
const HIGHLIGHT_MARGIN: f64 = 6.0;

fn floating_window(title: &str) -> WindowBuilder {
    WindowBuilder::new()
        .with_title(title)
        .with_decorations(false)
        .with_transparent(true)
        .with_resizable(false)
        .with_always_on_top(true)
        .with_visible_on_all_workspaces(true)
}

fn config_for(window: WindowBuilder) -> Config {
    Config::new()
        .with_window(window)
        .with_menu(None)
        .with_background_color((0, 0, 0, 0))
        .with_custom_head(crate::head())
}

/// Shows the countdown. It closes itself, and a click or Esc cancels the recording.
pub fn open_window(seconds: u32) {
    let window = floating_window("Countdown")
        .with_inner_size(LogicalSize::new(COUNTDOWN_SIZE, COUNTDOWN_SIZE));
    let dom = VirtualDom::new_with_props(Countdown, CountdownProps { seconds });
    let _pending = dioxus::desktop::window().new_window(dom, config_for(window));
}

#[component]
fn Countdown(seconds: u32) -> Element {
    let mut left = use_signal(move || seconds);
    use_hook(|| {
        let window = dioxus::desktop::window();
        native::center(&window.window);
        native::float_over_everything(&window.window);
        native::set_capture_protected(&window.window, true);
        native::bring_forward(&window.window);
    });
    use_future(move || async move {
        while left() > 0 {
            tokio::time::sleep(Duration::from_secs(1)).await;
            left -= 1;
        }
        dioxus::desktop::window().close();
    });
    let cancel = move || {
        links::tell_bar(ToBar::CancelCountdown);
        dioxus::desktop::window().close();
    };
    rsx! {
        div {
            class: "countdown-root",
            id: "countdown",
            tabindex: "0",
            autofocus: true,
            onclick: move |_| cancel(),
            onkeydown: move |event| {
                if event.key() == Key::Escape {
                    cancel();
                }
            },
            div { class: "countdown-card", "{left()}" }
        }
    }
}

/// Flashes a border around a source when it's chosen. `area` is in points from the top left.
pub fn open_highlight(area: (f64, f64, f64, f64)) {
    let (x, y, width, height) = area;
    let window = floating_window("Highlight")
        .with_position(LogicalPosition::new(
            x - HIGHLIGHT_MARGIN,
            y - HIGHLIGHT_MARGIN,
        ))
        .with_inner_size(LogicalSize::new(
            width + HIGHLIGHT_MARGIN * 2.0,
            height + HIGHLIGHT_MARGIN * 2.0,
        ))
        .with_focused(false);
    let dom = VirtualDom::new(Highlight);
    let _pending = dioxus::desktop::window().new_window(dom, config_for(window));
}

#[component]
fn Highlight() -> Element {
    use_hook(|| {
        let window = dioxus::desktop::window();
        let _ = window.set_ignore_cursor_events(true);
        native::set_capture_protected(&window.window, true);
    });
    use_future(move || async move {
        tokio::time::sleep(Duration::from_millis(HIGHLIGHT_MS)).await;
        dioxus::desktop::window().close();
    });
    rsx! { div { class: "highlight-root", id: "highlight" } }
}
