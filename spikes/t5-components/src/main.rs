//! T5 from BUILD_PLAN.md: every vendored Rust/UI component on a real webview, driven with real input
//! through AppKit, to list what needs fixing. Failures here are findings, not test failures.

use std::time::Duration;

use dioxus::desktop::tao::dpi::LogicalSize;
use dioxus::desktop::{Config, WindowBuilder};
use dioxus::prelude::*;
use objc2_app_kit::NSEventType;
use serde_json::{Value, json};

mod macos;

#[rustfmt::skip]
#[allow(dead_code, unused_imports, clippy::all)]
#[path = "../../../crates/reel-app/src/hooks/mod.rs"]
mod hooks;
#[rustfmt::skip]
#[allow(dead_code, unused_imports, clippy::all)]
#[path = "../../../crates/reel-app/src/ui/mod.rs"]
mod ui;

use ui::accordion::*;
use ui::alert_dialog::*;
use ui::button::*;
use ui::card::*;
use ui::context_menu::*;
use ui::dialog::*;
use ui::dropdown_menu::*;
use ui::input::*;
use ui::kbd::*;
use ui::label::*;
use ui::popover::*;
use ui::progress::*;
use ui::scroll_area::*;
use ui::select::*;
use ui::separator::*;
use ui::skeleton::*;
use ui::slider::*;
use ui::spinner::*;
use ui::switch::*;
use ui::tabs::*;
use ui::toast::*;
use ui::toggle_group::*;
use ui::tooltip::*;

const TAILWIND_CSS: &str = include_str!("../../../crates/reel-app/assets/tailwind.css");
const TOAST_CSS: &str = include_str!("../../../crates/reel-app/assets/rust-ui/toast.css");
const TOAST_JS: &str = include_str!("../../../crates/reel-app/assets/rust-ui/toast.js");

const PAGE_CSS: &str = r#"
.page { display: grid; grid-template-columns: repeat(4, 1fr); gap: 16px; padding: 16px; align-items: start; }
.cell { display: flex; flex-direction: column; gap: 8px; align-items: flex-start; }
.cell > h4 { font-size: 11px; opacity: .6; margin: 0; }
.w-48 { width: 12rem; } .h-24 { height: 6rem; } .w-32 { width: 8rem; } .h-4 { height: 1rem; } .w-40 { width: 10rem; } .py-3 { padding-block: .75rem; }
"#;

/// Counts `document` listeners and collects page errors, before any component's script runs.
const HEAD_SCRIPT: &str = r#"
window.__errors = [];
addEventListener('error', e => __errors.push(String(e.message)));
addEventListener('unhandledrejection', e => __errors.push(String(e.reason)));
window.__listeners = 0;
const add = document.addEventListener.bind(document);
const remove = document.removeEventListener.bind(document);
document.addEventListener = (...args) => { __listeners++; return add(...args); };
document.removeEventListener = (...args) => { __listeners--; return remove(...args); };
"#;

fn main() {
    let window = WindowBuilder::new()
        .with_title("T5 Components")
        .with_always_on_top(true)
        .with_inner_size(LogicalSize::new(1300.0, 940.0));
    let config = Config::new().with_window(window).with_custom_head(format!(
        "<style>{TAILWIND_CSS}{TOAST_CSS}{PAGE_CSS}</style><script>{HEAD_SCRIPT}</script>"
    ));
    dioxus::LaunchBuilder::desktop()
        .with_cfg(config)
        .launch(App);
}

#[component]
fn App() -> Element {
    let mut clicks = use_signal(|| 0);
    let mut slider_value = use_signal(|| 25.0);
    let mut input_value = use_signal(String::new);
    let mut select_value = use_signal(|| None::<Option<String>>);
    let mut toggled = use_signal(|| false);
    let mut dropdown_clicks = use_signal(|| 0);
    let mut context_clicks = use_signal(|| 0);
    let mut show_remounted = use_signal(|| true);

    use_future(move || async move {
        if std::env::var_os("T5_SELF_TEST").is_some() {
            self_test(show_remounted).await;
        }
    });

    rsx! {
        script { dangerous_inner_html: "{TOAST_JS}" }
        div { class: "page",
            div { class: "cell",
                h4 { "Button" }
                Button { id: "button", onclick: move |_| clicks += 1, "Clicks: {clicks}" }
                div { id: "clicks", "{clicks}" }
                Button { variant: ButtonVariant::Outline, "Outline" }
                Button { variant: ButtonVariant::Destructive, "Destructive" }
                Button { disabled: true, "Disabled" }
            }
            div { class: "cell",
                h4 { "Card" }
                Card {
                    CardHeader { CardTitle { "Title" } CardDescription { "Description" } }
                    CardContent { "Body" }
                    CardFooter { "Footer" }
                }
            }
            div { class: "cell",
                h4 { "Input and Label" }
                Label { "Name" }
                Input { id: "input", placeholder: "Type here", oninput: move |e: FormEvent| input_value.set(e.value()) }
                div { id: "input-value", "{input_value}" }
                h4 { "Kbd" }
                KbdGroup { Kbd { "Cmd" } Kbd { "K" } }
            }
            div { class: "cell",
                h4 { "Separator, Skeleton, Spinner, Progress" }
                Separator {}
                Skeleton { class: "w-32 h-4" }
                Spinner {}
                div { class: "w-40", Progress { value: 60.0 } }
            }
            div { class: "cell",
                h4 { "Slider" }
                div { class: "w-48",
                    Slider { value: slider_value(), oninput: move |e: FormEvent| slider_value.set(e.value().parse().unwrap_or(0.0)) }
                }
                div { id: "slider-value", "{slider_value}" }
                h4 { "Switch" }
                Switch { id: "switch" }
            }
            div { class: "cell",
                h4 { "Tabs" }
                Tabs { default_value: "one",
                    TabsList {
                        TabsTrigger { value: "one", "One" }
                        TabsTrigger { value: "two", "Two" }
                    }
                    TabsContent { value: "one", div { id: "tab-one", "First" } }
                    TabsContent { value: "two", div { id: "tab-two", "Second" } }
                }
            }
            div { class: "cell",
                h4 { "Toggle Group" }
                ToggleGroup {
                    ToggleGroupItem { title: "Bold", pressed: toggled(), onclick: move |_| toggled.toggle(), "B" }
                    ToggleGroupItem { title: "Italic", "I" }
                }
                div { id: "toggled", "{toggled}" }
                h4 { "Tooltip" }
                Tooltip {
                    Button { id: "tooltip-trigger", variant: ButtonVariant::Outline, "Hover me" }
                    TooltipContent { "Tooltip text" }
                }
            }
            div { class: "cell",
                h4 { "Accordion" }
                Accordion {
                    AccordionItem {
                        AccordionTrigger { "Section" }
                        AccordionContent { div { id: "accordion-body", "Hidden until opened" } }
                    }
                }
            }
            div { class: "cell",
                h4 { "Scroll Area" }
                ScrollArea { class: "w-48 h-24 border",
                    ScrollAreaViewport {
                        div { id: "scroll-body", style: "height: 400px", "Tall content" }
                    }
                    ScrollBar {}
                }
            }
            div { class: "cell",
                h4 { "Select" }
                Select { on_change: move |value| select_value.set(Some(value)),
                    SelectTrigger { SelectValue { placeholder: "Pick one" } }
                    SelectContent {
                        SelectGroup {
                            SelectOption { value: "alpha", "Alpha" }
                            SelectOption { value: "beta", "Beta" }
                        }
                    }
                }
                div { id: "select-value", "{select_value:?}" }
            }
            div { class: "cell",
                h4 { "Dropdown Menu" }
                DropdownMenu {
                    DropdownMenuTrigger { "Menu" }
                    DropdownMenuContent {
                        DropdownMenuLabel { "Actions" }
                        DropdownMenuGroup {
                            DropdownMenuItem { onclick: move |_| dropdown_clicks += 1, DropdownMenuAction { "First" } }
                            DropdownMenuItem { DropdownMenuAction { "Second" } }
                        }
                    }
                }
                div { id: "dropdown-clicks", "{dropdown_clicks}" }
            }
            div { class: "cell",
                h4 { "Context Menu" }
                ContextMenu {
                    ContextMenuTrigger { div { id: "context-trigger", class: "p-3 border", "Right-click" } }
                    ContextMenuContent {
                        ContextMenuGroup {
                            ContextMenuItem { ContextMenuAction { onclick: move |_| context_clicks += 1, "First" } }
                            ContextMenuItem { ContextMenuAction { "Second" } }
                        }
                    }
                }
                div { id: "context-clicks", "{context_clicks}" }
            }
            div { class: "cell",
                h4 { "Popover" }
                Popover {
                    PopoverTrigger { "Popover" }
                    PopoverContent { PopoverTitle { "Title" } PopoverDescription { "Text" } }
                }
            }
            div { class: "cell",
                h4 { "Dialog" }
                Dialog {
                    DialogTrigger { "Dialog" }
                    DialogContent {
                        DialogHeader { DialogTitle { "Title" } DialogDescription { "Text" } }
                        div { class: "py-3", Input { id: "dialog-input", placeholder: "Inside" } }
                        DialogFooter { DialogClose { "Close" } DialogAction { "OK" } }
                    }
                }
            }
            div { class: "cell",
                h4 { "Alert Dialog" }
                AlertDialog {
                    AlertDialogTrigger { "Alert" }
                    AlertDialogContent {
                        AlertDialogHeader { AlertDialogTitle { "Are you sure?" } AlertDialogDescription { "Text" } }
                        AlertDialogFooter { AlertDialogClose { "Cancel" } AlertDialogAction { "Delete" } }
                    }
                }
            }
            div { class: "cell",
                h4 { "Toast" }
                Button { id: "toast", onclick: move |_| show_toast(ToastType::Default, "Saved"), "Show toast" }
            }
            div { class: "cell",
                h4 { "Remounted popover and dialog" }
                if show_remounted() {
                    div { id: "remounted",
                        Popover {
                            PopoverTrigger { class: "remounted-popover", "Again" }
                            PopoverContent { class: "remounted-popover-content", "Text" }
                        }
                    }
                }
            }
        }
        ToastToaster {}
    }
}

struct Test {
    window: std::sync::Arc<dioxus::desktop::tao::window::Window>,
    results: Vec<Value>,
    /// Clicks sent while the window wasn't key, which AppKit absorbs. A run with any of these says
    /// nothing about the components.
    without_focus: u32,
}

impl Test {
    fn record(&mut self, component: &str, what: &str, pass: bool, detail: impl Into<String>) {
        let detail = detail.into();
        println!(
            "{} {component}: {what}: {detail}",
            if pass { "ok  " } else { "FAIL" }
        );
        self.results
            .push(json!({ "component": component, "what": what, "pass": pass, "detail": detail }));
    }

    async fn center(&self, selector: &str) -> Option<(f64, f64)> {
        let value = eval(&format!(
            "const e = document.querySelector({selector:?}); if (!e) return null; const r = e.getBoundingClientRect(); return [r.x + r.width / 2, r.y + r.height / 2];"
        ))
        .await;
        Some((value.get(0)?.as_f64()?, value.get(1)?.as_f64()?))
    }

    async fn click_at(&mut self, x: f64, y: f64) {
        if !macos::is_key(&self.window) {
            macos::make_key(&self.window);
            sleep(200).await;
        }
        self.without_focus += u32::from(!macos::is_key(&self.window));
        macos::send_mouse(&self.window, NSEventType::MouseMoved, x, y);
        sleep(20).await;
        macos::send_mouse(&self.window, NSEventType::LeftMouseDown, x, y);
        sleep(30).await;
        macos::send_mouse(&self.window, NSEventType::LeftMouseUp, x, y);
        sleep(250).await;
    }

    async fn click(&mut self, selector: &str) -> bool {
        match self.center(selector).await {
            Some((x, y)) => {
                self.click_at(x, y).await;
                true
            }
            None => false,
        }
    }

    async fn press_escape(&self) {
        macos::send_key(&self.window, "\u{1b}", 53);
        sleep(250).await;
    }

    async fn state(&self, selector: &str) -> String {
        eval(&format!(
            "return document.querySelector({selector:?})?.getAttribute('data-state') ?? 'missing';"
        ))
        .await
        .as_str()
        .unwrap_or("missing")
        .to_string()
    }

    async fn text(&self, selector: &str) -> String {
        eval(&format!(
            "return document.querySelector({selector:?})?.textContent ?? 'missing';"
        ))
        .await
        .as_str()
        .unwrap_or("missing")
        .to_string()
    }
}

async fn sleep(ms: u64) {
    tokio::time::sleep(Duration::from_millis(ms)).await;
}

async fn eval(script: &str) -> Value {
    document::eval(script).await.unwrap_or(Value::Null)
}

async fn self_test(mut show_remounted: Signal<bool>) {
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_secs(150));
        eprintln!("self-test timed out");
        std::process::exit(3);
    });
    let out = std::path::PathBuf::from(std::env::var("T5_OUT").unwrap_or_else(|_| ".".into()));
    std::fs::create_dir_all(&out).expect("couldn't create the output folder");
    let window = dioxus::desktop::window().window.clone();
    sleep(1500).await;
    macos::make_key(&window);
    sleep(400).await;
    let mut test = Test {
        window: window.clone(),
        results: vec![],
        without_focus: 0,
    };
    println!("key: {}", macos::is_key(&window));
    macos::save_capture(&window, &out.join("1-light.png"));

    // Simple controls.
    test.click("#button").await;
    let clicks = test.text("#clicks").await;
    test.record(
        "Button",
        "click reaches the Rust handler",
        clicks == "1",
        format!("count {clicks}"),
    );

    test.click("#input").await;
    macos::send_key(&window, "a", 0);
    sleep(150).await;
    let typed = test.text("#input-value").await;
    test.record(
        "Input",
        "typing reaches the Rust handler",
        typed == "a",
        format!("value {typed:?}"),
    );

    if let Some(rect) = rect_of("[data-name='Slider']").await {
        test.click_at(rect.0 + rect.2 * 0.75, rect.1 + rect.3 / 2.0)
            .await;
        let value = test.text("#slider-value").await;
        let number: f64 = value.parse().unwrap_or(-1.0);
        test.record(
            "Slider",
            "click sets the value through its handler",
            (70.0..=80.0).contains(&number),
            format!("value {value} after clicking at 75%"),
        );
    } else {
        test.record("Slider", "is on the page", false, "not found");
    }

    test.click("#switch").await;
    let checked =
        eval("return document.getElementById('switch').getAttribute('aria-checked');").await;
    test.record(
        "Switch",
        "click toggles it",
        checked == "true",
        format!("aria-checked {checked}"),
    );
    test.record(
        "Switch",
        "reports changes to Rust",
        false,
        "has no change handler, so Rust can't read or control its value",
    );

    // Tabs.
    let tab_two_visible =
        "const e = document.getElementById('tab-two'); return !!e && e.offsetParent !== null;";
    let before = eval(tab_two_visible).await;
    let clicked = eval("const b = [...document.querySelectorAll('[data-name=\"TabsTrigger\"]')].find(e => e.textContent === 'Two'); if (!b) return null; const r = b.getBoundingClientRect(); return [r.x + r.width / 2, r.y + r.height / 2];").await;
    if let (Some(x), Some(y)) = (
        clicked.get(0).and_then(Value::as_f64),
        clicked.get(1).and_then(Value::as_f64),
    ) {
        test.click_at(x, y).await;
    }
    let after = eval(tab_two_visible).await;
    test.record(
        "Tabs",
        "click switches panels",
        before == false && after == true,
        format!("second panel visible: {before} then {after}"),
    );

    // Toggle group.
    test.click("[title='Bold']").await;
    let toggled = test.text("#toggled").await;
    let pressed = eval("return document.querySelector(\"[title='Bold']\").getAttribute('aria-pressed') ?? document.querySelector(\"[title='Bold']\").getAttribute('data-state');").await;
    test.record(
        "Toggle Group",
        "click reaches the Rust handler and shows as pressed",
        toggled == "true",
        format!("signal {toggled}, attribute {pressed}"),
    );

    // Tooltip, which is pure CSS hover.
    if let Some((x, y)) = test.center("#tooltip-trigger").await {
        let hidden = eval("return getComputedStyle(document.querySelector('[data-name=\"TooltipContent\"]')).opacity;").await;
        macos::hover(&window, x, y);
        sleep(500).await;
        let shown = eval("return getComputedStyle(document.querySelector('[data-name=\"TooltipContent\"]')).opacity;").await;
        test.record(
            "Tooltip",
            "shows on hover",
            shown == "1",
            format!("opacity {hidden} then {shown}"),
        );
    }

    // Accordion, which is a hidden checkbox.
    let height = "const e = document.querySelector('[data-name=\"AccordionItem\"] article'); return e ? e.getBoundingClientRect().height : -1;";
    let closed = eval(height).await;
    test.click("[data-name='AccordionItem'] label").await;
    sleep(400).await;
    let opened = eval(height).await;
    test.record(
        "Accordion",
        "click opens it",
        opened.as_f64().unwrap_or(0.0) > closed.as_f64().unwrap_or(0.0),
        format!("content height {closed} then {opened}"),
    );

    // Scroll area.
    let scrolled = eval("const v = document.querySelector('[data-name=\"ScrollAreaViewport\"]'); v.scrollTop = 80; return [v.scrollHeight > v.clientHeight, v.scrollTop, getComputedStyle(v).overflowY];").await;
    test.record(
        "Scroll Area",
        "content scrolls",
        scrolled[1].as_f64().unwrap_or(0.0) > 0.0,
        format!("{scrolled}"),
    );

    // Select.
    test.click("[data-name='SelectTrigger']").await;
    let open_state = test.state("[data-name='SelectContent']").await;
    test.record(
        "Select",
        "click opens it",
        open_state == "open",
        format!("data-state {open_state}"),
    );
    macos::save_capture(&window, &out.join("2-select.png"));
    let option_near = eval("const o = document.querySelector('[data-name=\"SelectOption\"]'); if (!o) return null; const r = o.getBoundingClientRect(); return [r.x + r.width / 2, r.y + r.height / 2];").await;
    if let (Some(x), Some(y)) = (
        option_near.get(0).and_then(Value::as_f64),
        option_near.get(1).and_then(Value::as_f64),
    ) {
        test.click_at(x, y).await;
    }
    let chosen = test.text("#select-value").await;
    let closed_state = test.state("[data-name='SelectContent']").await;
    test.record(
        "Select",
        "choosing reaches Rust and closes it",
        chosen.contains("alpha") && closed_state != "open",
        format!("value {chosen}, data-state {closed_state}"),
    );

    // Keyboard navigation in Select, listed in step 6.4.
    test.click("[data-name='SelectTrigger']").await;
    macos::send_key(&window, "\u{f701}", 125);
    sleep(150).await;
    let focused = eval("const a = document.activeElement; return a ? (a.getAttribute('data-name') || a.tagName) : null;").await;
    test.record(
        "Select",
        "arrow keys move through options",
        focused == "SelectOption",
        format!("focus is on {focused}"),
    );
    test.press_escape().await;
    let after_escape = test.state("[data-name='SelectContent']").await;
    test.record(
        "Select",
        "Escape closes it",
        after_escape != "open",
        format!("data-state {after_escape}"),
    );

    // Dropdown menu.
    test.click("[data-name='DropdownMenuTrigger']").await;
    let open_state = test.state("[data-name='DropdownMenuContent']").await;
    test.record(
        "Dropdown Menu",
        "click opens it",
        open_state == "open",
        format!("data-state {open_state}"),
    );
    macos::save_capture(&window, &out.join("3-dropdown.png"));
    macos::send_key(&window, "\u{f701}", 125);
    sleep(150).await;
    let focused = eval("const a = document.activeElement; return a ? (a.getAttribute('data-name') || a.tagName) : null;").await;
    test.record(
        "Dropdown Menu",
        "arrow keys move through items",
        focused == "DropdownMenuAction" || focused == "DropdownMenuItem",
        format!("focus is on {focused}"),
    );
    test.click("[data-name='DropdownMenuItem']").await;
    let clicked = test.text("#dropdown-clicks").await;
    let closed_state = test.state("[data-name='DropdownMenuContent']").await;
    test.record(
        "Dropdown Menu",
        "item click reaches Rust and closes it",
        clicked == "1" && closed_state != "open",
        format!("count {clicked}, data-state {closed_state}"),
    );

    // Context menu.
    if let Some((x, y)) = test.center("#context-trigger").await {
        macos::send_mouse(&window, NSEventType::RightMouseDown, x, y);
        sleep(40).await;
        macos::send_mouse(&window, NSEventType::RightMouseUp, x, y);
        sleep(300).await;
    }
    let content = "[data-target='target__context']";
    let open_state = test.state(content).await;
    test.record(
        "Context Menu",
        "right-click opens it",
        open_state == "open",
        format!("data-state {open_state}"),
    );
    macos::save_capture(&window, &out.join("4-context.png"));
    close_context_menu();
    sleep(250).await;
    let still_open = test.state(content).await;
    test.record(
        "Context Menu",
        "close_context_menu() closes it from Rust",
        still_open != "open",
        format!("data-state {still_open}. It's wasm32-only, so it does nothing here"),
    );
    test.click("[data-name='ContextMenuAction']").await;
    let clicked = test.text("#context-clicks").await;
    let closed_state = test.state(content).await;
    test.record(
        "Context Menu",
        "action click reaches Rust and closes it",
        clicked == "1" && closed_state != "open",
        format!("count {clicked}, data-state {closed_state}"),
    );

    // Popover.
    test.click("[data-name='PopoverTrigger']").await;
    let open_state = test.state("[data-name='PopoverContent']").await;
    test.record(
        "Popover",
        "click opens it",
        open_state == "open",
        format!("data-state {open_state}"),
    );
    macos::save_capture(&window, &out.join("5-popover.png"));
    test.press_escape().await;
    let after_escape = test.state("[data-name='PopoverContent']").await;
    test.record(
        "Popover",
        "Escape closes it",
        after_escape == "closed",
        format!("data-state {after_escape}"),
    );

    // Dialog, including the focus trap listed in step 6.4.
    let dialog = "[data-name='__Dialog'] [data-name='DialogContent']";
    let trigger = "[data-dialog-trigger]";
    test.click(trigger).await;
    let open_state = test.state(dialog).await;
    test.record(
        "Dialog",
        "click opens it",
        open_state == "open",
        format!("data-state {open_state}"),
    );
    macos::save_capture(&window, &out.join("6-dialog.png"));
    let mut escaped = false;
    for _ in 0..6 {
        macos::send_key(&window, "\t", 48);
        sleep(80).await;
        let inside = eval(&format!(
            "return !!document.activeElement?.closest({dialog:?});"
        ))
        .await;
        if inside == false {
            escaped = true;
        }
    }
    test.record(
        "Dialog",
        "Tab stays inside it",
        !escaped,
        if escaped {
            "focus left the dialog"
        } else {
            "focus stayed inside"
        },
    );
    test.press_escape().await;
    let after_escape = test.state(dialog).await;
    test.record(
        "Dialog",
        "Escape closes it",
        after_escape == "closed",
        format!("data-state {after_escape}"),
    );

    // Alert dialog, which reuses Dialog.
    let triggers = eval("return [...document.querySelectorAll('[data-dialog-trigger]')].map(e => { const r = e.getBoundingClientRect(); return [r.x + r.width / 2, r.y + r.height / 2]; });").await;
    if let Some(second) = triggers.get(1) {
        let (x, y) = (
            second[0].as_f64().unwrap_or(0.0),
            second[1].as_f64().unwrap_or(0.0),
        );
        test.click_at(x, y).await;
        let open = eval("return [...document.querySelectorAll('[data-name=\"DialogContent\"]')].filter(e => e.getAttribute('data-state') === 'open').length;").await;
        test.record(
            "Alert Dialog",
            "click opens it",
            open == 1,
            format!("{open} dialogs open"),
        );
        macos::save_capture(&window, &out.join("7-alert.png"));
        test.click_at(30.0, 30.0).await;
        let stays = eval("return [...document.querySelectorAll('[data-name=\"DialogContent\"]')].filter(e => e.getAttribute('data-state') === 'open').length;").await;
        test.record(
            "Alert Dialog",
            "a click on the backdrop leaves it open",
            stays == 1,
            format!("{stays} dialogs open"),
        );
        test.press_escape().await;
        let open = eval("return [...document.querySelectorAll('[data-name=\"DialogContent\"]')].filter(e => e.getAttribute('data-state') === 'open').length;").await;
        test.record(
            "Alert Dialog",
            "Escape closes it",
            open == 0,
            format!("{open} dialogs open"),
        );
    }

    // Toast.
    println!("window before toast: {}", macos::describe(&window));
    test.click("#toast").await;
    sleep(400).await;
    let toast = eval("return !!document.querySelector('[data-name=\"Toast\"], [data-toast], [data-sonner-toast], .toast');").await;
    let body = eval("return document.body.innerText.includes('Saved');").await;
    test.record(
        "Toast",
        "show_toast displays it",
        body == true,
        format!("found element {toast}, text on page {body}"),
    );
    println!("window after toast: {}", macos::describe(&window));
    macos::save_capture(&window, &out.join("8-toast.png"));

    // What Dioxus remounting does, which a real app does constantly.
    let trigger = ".remounted-popover";
    let content = ".remounted-popover-content";
    test.click(trigger).await;
    let first = test.state(content).await;
    test.record(
        "Popover",
        "works on its first mount",
        first == "open",
        format!("data-state {first}"),
    );
    test.press_escape().await;
    let listeners_before = eval("return window.__listeners;")
        .await
        .as_i64()
        .unwrap_or(0);
    for _ in 0..5 {
        show_remounted.set(false);
        sleep(80).await;
        show_remounted.set(true);
        sleep(120).await;
    }
    let listeners_after = eval("return window.__listeners;")
        .await
        .as_i64()
        .unwrap_or(0);
    test.record(
        "Popover",
        "removes its document listeners when unmounted",
        listeners_after == listeners_before,
        format!("{listeners_before} listeners, then {listeners_after} after 5 remounts"),
    );
    test.click(trigger).await;
    let again = test.state(content).await;
    test.record(
        "Popover",
        "works again after a remount",
        again == "open",
        format!("data-state {again}"),
    );
    test.press_escape().await;

    println!("window at the end: {}", macos::describe(&window));

    // Page errors.
    let errors = eval("return window.__errors;").await;
    test.record(
        "All",
        "no JavaScript errors",
        errors.as_array().is_some_and(Vec::is_empty),
        format!("{errors}"),
    );

    // Which CSS the macOS 14 minimum might not have. This run's WebKit supports what's listed true.
    let supports = eval("return { anchor: CSS.supports('anchor-name', '--x'), position_area: CSS.supports('position-area', 'block-start'), oklch: CSS.supports('color', 'oklch(50% 0.1 10)'), color_mix: CSS.supports('color', 'color-mix(in oklab, red, blue)'), has: CSS.supports('selector(:has(a))'), container: CSS.supports('container-type', 'inline-size'), layer: true };").await;

    // The same page in dark mode, to look at by eye.
    eval("document.documentElement.classList.add('dark'); return true;").await;
    sleep(300).await;
    macos::save_capture(&window, &out.join("9-dark.png"));

    let failures = test
        .results
        .iter()
        .filter(|result| result["pass"] == false)
        .count();
    let report = json!({ "total": test.results.len(), "failures": failures, "webkit_supports": supports, "results": test.results });
    std::fs::write(
        out.join("report.json"),
        serde_json::to_vec_pretty(&report).unwrap(),
    )
    .expect("couldn't write the report");
    if test.without_focus > 0 {
        println!("INCONCLUSIVE: {} clicks had no focus", test.without_focus);
    }
    println!("{failures} findings of {}", test.results.len());
    std::process::exit(0);
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
