//! Reel's desktop app. So far: the preview and transport from Milestone 3, in the shell from Milestone 0.

use dioxus::desktop::tao::dpi::LogicalSize;
use dioxus::desktop::tao::window::Theme;
use dioxus::desktop::{Config, WindowBuilder};
use dioxus::prelude::*;

// Vendored from Rust/UI and kept byte-for-byte, so don't lint or reformat them. See VENDORED.md.
#[rustfmt::skip]
#[allow(dead_code, unused_imports, clippy::all)]
mod hooks;
#[rustfmt::skip]
#[allow(dead_code, unused_imports, clippy::all)]
mod ui;

#[cfg(target_os = "macos")]
mod editor;
#[cfg(target_os = "macos")]
mod preview;
#[cfg(all(target_os = "macos", feature = "self-test"))]
mod self_test;

use ui::button::{Button, ButtonVariant};
use ui::toast::ToastToaster;

// Embedded rather than loaded through asset!(), so the app runs the same however it's launched.
const TAILWIND_CSS: &str = include_str!("../assets/tailwind.css");
const TOAST_CSS: &str = include_str!("../assets/rust-ui/toast.css");
const TOAST_JS: &str = include_str!("../assets/rust-ui/toast.js");
const SHELL_CSS: &str = include_str!("../assets/shell.css");

const LOGO_SVG: &str = include_str!("../../../assets/logo.svg");

fn main() {
    let window = WindowBuilder::new()
        .with_title("Reel")
        .with_inner_size(LogicalSize::new(1280.0, 800.0));
    let head = format!("<style>{TAILWIND_CSS}{TOAST_CSS}{SHELL_CSS}</style>");

    #[cfg(target_os = "macos")]
    let window = window.with_transparent(true);

    let config = Config::new().with_window(window).with_custom_head(head);

    // The preview's surface has to exist before the webview does, so its layer ends up underneath.
    #[cfg(target_os = "macos")]
    let config = config
        .with_as_child_window()
        .with_background_color((0, 0, 0, 0))
        .with_on_window(|window, _dom| preview::start(window));

    dioxus::LaunchBuilder::desktop()
        .with_cfg(config)
        .launch(App);
}

#[component]
fn App() -> Element {
    let system_is_dark = use_hook(|| dioxus::desktop::window().theme() == Theme::Dark);
    let mut dark = use_signal(|| system_is_dark);

    // Rust/UI's dark tokens and `dark:` variants key off a `dark` class on <html>.
    use_effect(move || {
        document::eval(&format!(
            "document.documentElement.classList.toggle('dark', {});",
            dark()
        ));
    });

    let toggle_theme = move |()| dark.toggle();

    rsx! {
        script { dangerous_inner_html: "{TOAST_JS}" }
        Content { dark: dark(), toggle_theme }
        ToastToaster {}
    }
}

#[cfg(target_os = "macos")]
#[component]
fn Content(dark: bool, toggle_theme: EventHandler<()>) -> Element {
    rsx! { editor::Editor { dark, toggle_theme } }
}

#[cfg(not(target_os = "macos"))]
#[component]
fn Content(dark: bool, toggle_theme: EventHandler<()>) -> Element {
    rsx! { Home { dark, toggle_theme } }
}

/// What shows until a project is open.
#[component]
pub fn Home(dark: bool, toggle_theme: EventHandler<()>, children: Element) -> Element {
    let logo = use_hook(themed_logo);
    let toggle_label = if dark {
        "Switch to light theme"
    } else {
        "Switch to dark theme"
    };

    rsx! {
        main { class: "home",
            div {
                class: "home-logo",
                role: "img",
                "aria-label": "Reel logo",
                dangerous_inner_html: "{logo}",
            }
            h1 { class: "home-title", "Reel" }
            {children}
            Button {
                variant: ButtonVariant::Outline,
                onclick: move |_| toggle_theme.call(()),
                "{toggle_label}"
            }
        }
    }
}

/// The logo file is black on transparent. Drawing it in the current text colour keeps it visible
/// in both themes without a second copy of the artwork.
pub fn themed_logo() -> String {
    LOGO_SVG.replace(r#"fill="black""#, r#"fill="currentColor""#)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[component]
    fn HomeForTest(dark: bool) -> Element {
        rsx! { Home { dark, toggle_theme: |()| {} } }
    }

    fn render_home(dark: bool) -> String {
        let mut dom = VirtualDom::new_with_props(HomeForTest, HomeForTestProps { dark });
        dom.rebuild_in_place();
        dioxus_ssr::render(&dom)
    }

    #[test]
    fn logo_follows_the_text_colour() {
        let logo = themed_logo();
        assert!(logo.contains(r#"fill="currentColor""#));
        assert!(!logo.contains(r#"fill="black""#));
    }

    #[test]
    fn home_shows_the_logo_and_a_button_offering_the_other_theme() {
        let light = render_home(false);
        assert!(light.contains("currentColor"));
        assert!(light.contains("Switch to dark theme"));

        let dark = render_home(true);
        assert!(dark.contains("Switch to light theme"));
    }
}
