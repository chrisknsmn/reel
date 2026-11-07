//! The controls every panel is built from (BUILD_PLAN 6.3). Rust/UI has a slider, but none with a label
//! and reset, and has no colour picker.

use dioxus::prelude::*;
use icons::RotateCcw;

use crate::ui::slider::Slider;

/// How to show a slider's value.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Format {
    #[default]
    Plain,
    Percent,
    Pixels,
    Milliseconds,
    Times,
}

/// A value as a label shows it.
#[must_use]
pub fn format_value(value: f64, step: f64, format: Format) -> String {
    let digits = if step >= 1.0 {
        0
    } else if step >= 0.1 {
        1
    } else {
        2
    };
    let number = format!("{value:.digits$}");
    match format {
        Format::Plain => number,
        Format::Percent => format!("{number}%"),
        Format::Pixels => format!("{number} px"),
        Format::Milliseconds => format!("{number} ms"),
        Format::Times => format!("{number}×"),
    }
}

/// A labelled slider with its value and a reset. Recordly's sliders have no typed entry or reset to
/// defaults (§7), so this adds both a reset button and double-clicking the label.
#[component]
pub fn SliderRow(
    label: String,
    value: f64,
    min: f64,
    max: f64,
    step: f64,
    default: f64,
    #[props(default)] format: Format,
    onchange: EventHandler<f64>,
    #[props(optional)] onrelease: Option<EventHandler<()>>,
    #[props(optional)] id: Option<String>,
) -> Element {
    let changed = (value - default).abs() > step / 2.0;
    let release = move |()| {
        if let Some(handler) = &onrelease {
            handler.call(());
        }
    };
    rsx! {
        div { class: "row-slider",
            div { class: "row-slider-top",
                label {
                    class: "row-label",
                    title: "Double-click to reset",
                    ondoubleclick: move |_| {
                        onchange.call(default);
                        release(());
                    },
                    "{label}"
                }
                span { class: "row-value", "{format_value(value, step, format)}" }
                if changed {
                    button {
                        class: "row-reset",
                        "aria-label": "Reset {label}",
                        onclick: move |_| {
                            onchange.call(default);
                            release(());
                        },
                        RotateCcw {}
                    }
                }
            }
            // The label around the slider gives it a name for screen readers, which the vendored slider
            // has no way to set directly.
            label {
                class: "row-slider-track",
                id,
                onmouseup: move |_| release(()),
                onkeyup: move |_| release(()),
                span { class: "sr-only", "{label}" }
                Slider {
                    min,
                    max,
                    step,
                    value,
                    oninput: move |event: FormEvent| {
                        if let Ok(number) = event.value().parse::<f64>() {
                            onchange.call(number);
                        }
                    },
                }
            }
        }
    }
}

/// A labelled on-or-off control.
#[component]
pub fn ToggleRow(
    label: String,
    value: bool,
    onchange: EventHandler<bool>,
    #[props(optional)] id: Option<String>,
) -> Element {
    rsx! {
        button {
            class: "row-toggle",
            id,
            role: "switch",
            "aria-checked": "{value}",
            "data-on": "{value}",
            onclick: move |_| onchange.call(!value),
            span { "{label}" }
            span { class: "row-switch", span {} }
        }
    }
}

/// A choice from a few options, shown as a row of buttons.
#[component]
pub fn ChoiceRow(
    label: String,
    options: Vec<(String, String)>,
    value: String,
    onchange: EventHandler<String>,
) -> Element {
    rsx! {
        div { class: "row-choice",
            span { class: "row-label", "{label}" }
            div { class: "row-choice-options", role: "group", "aria-label": "{label}",
                for (key, text) in options {
                    {
                        let selected = key == value;
                        let chosen = key.clone();
                        rsx! {
                            button {
                                "data-selected": "{selected}",
                                "aria-pressed": "{selected}",
                                onclick: move |_| onchange.call(chosen.clone()),
                                "{text}"
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Whether text is a colour this can show: `#RGB`, `#RRGGBB` or `#RRGGBBAA`.
#[must_use]
pub fn is_hex_color(text: &str) -> bool {
    text.strip_prefix('#').is_some_and(|digits| {
        matches!(digits.len(), 3 | 6 | 8) && digits.chars().all(|digit| digit.is_ascii_hexdigit())
    })
}

/// A colour picker: swatches, the system's picker, and typed hex.
#[component]
pub fn ColorPicker(
    label: String,
    value: String,
    swatches: Vec<&'static str>,
    onchange: EventHandler<String>,
) -> Element {
    let mut typed = use_signal(|| value.clone());
    let current = value.clone();
    use_effect(use_reactive!(|current| typed.set(current)));
    // The system's picker only understands six digits.
    let for_input = if value.len() == 7 && is_hex_color(&value) {
        value.clone()
    } else {
        "#000000".to_string()
    };
    rsx! {
        div { class: "row-color",
            span { class: "row-label", "{label}" }
            div { class: "row-swatches",
                for swatch in swatches {
                    button {
                        class: "row-swatch",
                        style: "background: {swatch}",
                        "aria-label": "{swatch}",
                        "data-selected": "{swatch.eq_ignore_ascii_case(&value)}",
                        onclick: move |_| onchange.call(swatch.to_string()),
                    }
                }
            }
            div { class: "row-color-entry",
                input {
                    r#type: "color",
                    value: "{for_input}",
                    "aria-label": "Pick {label}",
                    oninput: move |event| onchange.call(event.value()),
                }
                input {
                    class: "row-hex",
                    r#type: "text",
                    value: "{typed()}",
                    spellcheck: "false",
                    "aria-label": "{label} as hex",
                    oninput: move |event| {
                        let text = event.value();
                        if is_hex_color(&text) {
                            onchange.call(text.clone());
                        }
                        typed.set(text);
                    },
                }
            }
        }
    }
}

/// A heading that starts a group of controls.
#[component]
pub fn Section(title: String, children: Element) -> Element {
    rsx! {
        section { class: "panel-section",
            h3 { "{title}" }
            {children}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_show_with_the_right_detail_and_unit() {
        assert_eq!(format_value(0.674, 0.01, Format::Plain), "0.67");
        assert_eq!(format_value(12.5, 0.5, Format::Plain), "12.5");
        assert_eq!(format_value(20.0, 1.0, Format::Percent), "20%");
        assert_eq!(format_value(600.0, 10.0, Format::Milliseconds), "600 ms");
        assert_eq!(format_value(24.0, 1.0, Format::Pixels), "24 px");
        assert_eq!(format_value(2.25, 0.05, Format::Times), "2.25×");
    }

    #[test]
    fn only_real_hex_colours_count() {
        for good in ["#fff", "#2563EB", "#2563EB80"] {
            assert!(is_hex_color(good), "{good}");
        }
        for bad in ["", "2563EB", "#12", "#GGGGGG", "#12345"] {
            assert!(!is_hex_color(bad), "{bad}");
        }
    }
}
