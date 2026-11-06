//! Reel's desktop app. For now this is the Milestone 0 shell: a window, the logo and a theme toggle.

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

use ui::button::{Button, ButtonVariant};
use ui::toast::ToastToaster;

const TAILWIND_CSS: Asset = asset!("/assets/tailwind.css");
const TOAST_CSS: Asset = asset!("/assets/rust-ui/toast.css");
const TOAST_JS: Asset = asset!("/assets/rust-ui/toast.js");

const LOGO_SVG: &str = include_str!("../../../assets/logo.svg");

fn main() {
    let window = WindowBuilder::new()
        .with_title("Reel")
        .with_inner_size(LogicalSize::new(960.0, 640.0));

    dioxus::LaunchBuilder::desktop()
        .with_cfg(Config::new().with_window(window))
        .launch(App);
}

#[component]
fn App() -> Element {
    let system_is_dark = use_hook(|| dioxus::desktop::window().theme() == Theme::Dark);

    rsx! {
        document::Stylesheet { href: TAILWIND_CSS }
        document::Stylesheet { href: TOAST_CSS }
        Home { initial_dark: system_is_dark }
        ToastToaster {}
        script { src: TOAST_JS }
    }
}

#[component]
fn Home(initial_dark: bool) -> Element {
    let mut dark = use_signal(|| initial_dark);
    let logo = use_hook(themed_logo);

    // Rust/UI's dark tokens and `dark:` variants key off a `dark` class on <html>.
    use_effect(move || {
        document::eval(&format!(
            "document.documentElement.classList.toggle('dark', {});",
            dark()
        ));
    });

    let toggle_label = if dark() {
        "Switch to light theme"
    } else {
        "Switch to dark theme"
    };

    rsx! {
        main { class: "flex flex-col gap-6 justify-center items-center h-full",
            div {
                class: "size-24 text-foreground [&>svg]:size-full",
                role: "img",
                "aria-label": "Reel logo",
                dangerous_inner_html: "{logo}",
            }
            h1 { class: "text-2xl font-semibold tracking-tight", "Reel" }
            Button {
                variant: ButtonVariant::Outline,
                onclick: move |_| dark.toggle(),
                "{toggle_label}"
            }
        }
    }
}

/// The logo file is black on transparent. Drawing it in the current text colour keeps it visible
/// in both themes without a second copy of the artwork.
fn themed_logo() -> String {
    LOGO_SVG.replace(r#"fill="black""#, r#"fill="currentColor""#)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render_home(initial_dark: bool) -> String {
        let mut dom = VirtualDom::new_with_props(Home, HomeProps { initial_dark });
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
