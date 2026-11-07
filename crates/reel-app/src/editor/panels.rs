//! The settings panel beside the preview (§8–§13). Selecting something on the timeline swaps it for
//! that item's own settings (§7).

use dioxus::prelude::*;
use reel_core::edit::Edit;
use reel_core::history::Selection;
use reel_core::project::{
    Background, CaptionAnimation, ClickEffect, Padding, Project, WebcamPosition,
};
use reel_core::settings::{MotionPreset, SettingsStore, Theme};
use reel_core::timeline::{
    AnnotationContent, ArrowDirection, CLIP_SPEEDS, ItemId, Point, TextAlign, ZOOM_SCALES, ZoomMode,
};

use super::controls::{ChoiceRow, ColorPicker, Format, Section, SliderRow, ToggleRow};
use super::math::{exact_time, parse_time};
use super::state::{Ed, Overlay, Rail, use_ed};
use crate::recording;

const COLOR_SWATCHES: [&str; 15] = [
    "#FF0000", "#FFD700", "#00FF00", "#FFFFFF", "#0000FF", "#FF6B00", "#9B59B6", "#E91E63",
    "#00BCD4", "#FF5722", "#8BC34A", "#FFC107", "#2563EB", "#000000", "#607D8B",
];
const EFFECT_SWATCHES: [&str; 8] = [
    "#2563EB", "#EF4444", "#F59E0B", "#22C55E", "#A855F7", "#EC4899", "#14B8A6", "#F97316",
];
/// Reel's own gradients. Recordly's wallpapers aren't carried over, because their origin isn't known (D8).
const GRADIENTS: [&str; 12] = [
    "linear-gradient(135deg, #1e3a8a, #9333ea)",
    "linear-gradient(135deg, #0f172a, #334155)",
    "linear-gradient(135deg, #f97316, #db2777)",
    "linear-gradient(135deg, #059669, #0ea5e9)",
    "linear-gradient(135deg, #7c3aed, #ec4899)",
    "linear-gradient(135deg, #fde68a, #f472b6)",
    "linear-gradient(180deg, #0ea5e9, #1e3a8a)",
    "linear-gradient(180deg, #111827, #000000)",
    "linear-gradient(90deg, #14b8a6, #84cc16)",
    "linear-gradient(90deg, #f43f5e, #fbbf24)",
    "radial-gradient(circle, #3b82f6, #0f172a)",
    "radial-gradient(circle, #f9fafb, #9ca3af)",
];
const TEXT_SIZES: [u32; 17] = [
    12, 14, 16, 18, 20, 24, 28, 32, 36, 40, 48, 56, 64, 72, 80, 96, 128,
];
const FONTS: [(&str, &str); 8] = [
    ("Classic", "system-ui"),
    ("Editor", "Georgia"),
    ("Strong", "Impact"),
    ("Typewriter", "Courier New"),
    ("Deco", "Brush Script MT"),
    ("Simple", "Arial"),
    ("Modern", "Verdana"),
    ("Clean", "Trebuchet MS"),
];

fn options(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs
        .iter()
        .map(|(key, text)| ((*key).to_string(), (*text).to_string()))
        .collect()
}

/// The panel for what's selected, or else for the rail's choice.
#[component]
pub fn SidePanel() -> Element {
    let ed = use_ed();
    let Some(project) = ed.project() else {
        return rsx! {};
    };
    let content = match ed.selection() {
        Selection::Clip(id) => rsx! { ClipPanel { id } },
        Selection::Zoom(id) => rsx! { ZoomPanel { id } },
        Selection::AllZooms => rsx! { AllZoomsPanel {} },
        Selection::Annotation(id) => rsx! { AnnotationPanel { id } },
        Selection::Audio(id) => rsx! { AudioPanel { id } },
        Selection::Caption(id) => rsx! { CaptionPanel { id } },
        Selection::None => match (ed.rail)() {
            Rail::Scene => rsx! { ScenePanel { project: project.clone() } },
            Rail::Cursor => rsx! { CursorPanel { project: project.clone() } },
            Rail::Webcam => rsx! { WebcamPanel { project: project.clone() } },
            Rail::Captions => rsx! { CaptionsPanel { project: project.clone() } },
            Rail::Settings => rsx! { SettingsPanel { project: project.clone() } },
        },
    };
    rsx! {
        aside { class: "ed-panel", id: "side-panel", {content} }
    }
}

fn set_look(ed: Ed, gesture: &str, update: impl FnOnce(&mut Project)) {
    ed.change(gesture, update);
}

#[component]
fn ScenePanel(project: Project) -> Element {
    let ed = use_ed();
    let look = project.look.clone();
    let current_color = match &look.background {
        Background::Color(color) => color.clone(),
        _ => String::new(),
    };
    let current_gradient = match &look.background {
        Background::Gradient(gradient) => gradient.clone(),
        _ => String::new(),
    };
    let padding = look.padding;
    let crop = look.crop;
    // No background means zero padding, no shadow and the video's own shape (§8).
    let removed =
        padding == Padding::uniform(0.0) && look.aspect_ratio == "native" && look.shadow == 0.0;
    rsx! {
        h2 { "Scene" }
        Section { title: "Background",
            SliderRow {
                label: "Blur",
                value: look.background_blur,
                min: 0.0,
                max: 8.0,
                step: 0.25,
                default: 0.0,
                onchange: move |value| set_look(ed, "background-blur", |p| p.look.background_blur = value),
                onrelease: move |()| ed.end_gesture(),
            }
            ColorPicker {
                label: "Colour",
                value: current_color,
                swatches: COLOR_SWATCHES.to_vec(),
                onchange: move |color: String| ed.change("background-color", |p| p.look.background = Background::Color(color)),
            }
            div { class: "row-gradients", role: "group", "aria-label": "Gradients",
                for gradient in GRADIENTS {
                    button {
                        class: "row-gradient",
                        style: "background: {gradient}",
                        "aria-label": "Use this gradient",
                        "data-selected": "{gradient == current_gradient}",
                        onclick: move |_| {
                            ed.end_gesture();
                            ed.change("background-gradient", |p| p.look.background = Background::Gradient(gradient.to_string()));
                            ed.end_gesture();
                        },
                    }
                }
            }
            div { class: "row-buttons",
                button {
                    class: "row-button",
                    id: "background-image",
                    onclick: move |_| {
                        let picked = rfd::FileDialog::new()
                            .set_title("Choose a background image")
                            .add_filter("Images", &["jpg", "jpeg", "png", "webp"])
                            .pick_file();
                        if let Some(path) = picked {
                            ed.change("background-image", |p| p.look.background = Background::Image(reel_core::project::MediaRef::absolute(path)));
                            ed.end_gesture();
                        }
                    },
                    "Image…"
                }
                button {
                    class: "row-button",
                    id: "background-video",
                    onclick: move |_| {
                        let picked = rfd::FileDialog::new()
                            .set_title("Choose a background video")
                            .add_filter("Videos", &["mp4", "mov", "m4v"])
                            .pick_file();
                        if let Some(path) = picked {
                            ed.change("background-video", |p| p.look.background = Background::Video(reel_core::project::MediaRef::absolute(path)));
                            ed.end_gesture();
                        }
                    },
                    "Video…"
                }
            }
        }
        Section { title: "Frame",
            SliderRow {
                label: "Shadow",
                id: "slider-shadow",
                value: look.shadow,
                min: 0.0,
                max: 1.0,
                step: 0.01,
                default: 0.67,
                onchange: move |value| set_look(ed, "shadow", |p| p.look.shadow = value),
                onrelease: move |()| ed.end_gesture(),
            }
            SliderRow {
                label: "Radius",
                value: look.corner_radius,
                min: 0.0,
                max: 200.0,
                step: 0.5,
                default: 12.5,
                onchange: move |value| set_look(ed, "radius", |p| p.look.corner_radius = value),
                onrelease: move |()| ed.end_gesture(),
            }
            ToggleRow {
                label: "Link padding",
                value: padding.linked,
                onchange: move |linked| {
                    ed.change("padding-linked", |p| {
                        let old = p.look.padding;
                        p.look.padding.linked = linked;
                        // Relinking averages the four sides (§8).
                        if linked {
                            let average = (old.top + old.bottom + old.left + old.right) / 4.0;
                            p.look.padding = Padding::uniform(average.round());
                        }
                    });
                    ed.end_gesture();
                },
            }
            if padding.linked {
                SliderRow {
                    label: "Padding",
                    value: padding.top,
                    min: 0.0,
                    max: 100.0,
                    step: 1.0,
                    default: 20.0,
                    format: Format::Percent,
                    onchange: move |value| set_look(ed, "padding", |p| p.look.padding = Padding::uniform(value)),
                    onrelease: move |()| ed.end_gesture(),
                }
            } else {
                SliderRow { label: "Top", value: padding.top, min: 0.0, max: 250.0, step: 1.0, default: 20.0,
                    onchange: move |value| set_look(ed, "padding-top", |p| p.look.padding.top = value),
                    onrelease: move |()| ed.end_gesture(),
                }
                SliderRow { label: "Bottom", value: padding.bottom, min: 0.0, max: 250.0, step: 1.0, default: 20.0,
                    onchange: move |value| set_look(ed, "padding-bottom", |p| p.look.padding.bottom = value),
                    onrelease: move |()| ed.end_gesture(),
                }
                SliderRow { label: "Left", value: padding.left, min: 0.0, max: 100.0, step: 1.0, default: 20.0,
                    onchange: move |value| set_look(ed, "padding-left", |p| p.look.padding.left = value),
                    onrelease: move |()| ed.end_gesture(),
                }
                SliderRow { label: "Right", value: padding.right, min: 0.0, max: 100.0, step: 1.0, default: 20.0,
                    onchange: move |value| set_look(ed, "padding-right", |p| p.look.padding.right = value),
                    onrelease: move |()| ed.end_gesture(),
                }
            }
            ToggleRow {
                label: "Remove background",
                value: removed,
                onchange: move |remove: bool| {
                    ed.change("remove-background", |p| {
                        if remove {
                            p.look.padding = Padding::uniform(0.0);
                            p.look.aspect_ratio = "native".to_string();
                            p.look.shadow = 0.0;
                        } else {
                            let defaults = reel_core::project::Look::default();
                            p.look.padding = defaults.padding;
                            p.look.aspect_ratio = defaults.aspect_ratio;
                            p.look.shadow = defaults.shadow;
                        }
                    });
                    ed.end_gesture();
                },
            }
        }
        Section { title: "Crop",
            SliderRow { label: "Top", value: crop.y * 100.0, min: 0.0, max: 50.0, step: 1.0, default: 0.0, format: Format::Percent,
                onchange: move |value: f64| ed.change("crop", |p| set_crop_edge(p, CropEdge::Top, value)),
                onrelease: move |()| ed.end_gesture(),
            }
            SliderRow { label: "Bottom", value: (1.0 - crop.y - crop.height) * 100.0, min: 0.0, max: 50.0, step: 1.0, default: 0.0, format: Format::Percent,
                onchange: move |value: f64| ed.change("crop", |p| set_crop_edge(p, CropEdge::Bottom, value)),
                onrelease: move |()| ed.end_gesture(),
            }
            SliderRow { label: "Left", value: crop.x * 100.0, min: 0.0, max: 50.0, step: 1.0, default: 0.0, format: Format::Percent,
                onchange: move |value: f64| ed.change("crop", |p| set_crop_edge(p, CropEdge::Left, value)),
                onrelease: move |()| ed.end_gesture(),
            }
            SliderRow { label: "Right", value: (1.0 - crop.x - crop.width) * 100.0, min: 0.0, max: 50.0, step: 1.0, default: 0.0, format: Format::Percent,
                onchange: move |value: f64| ed.change("crop", |p| set_crop_edge(p, CropEdge::Right, value)),
                onrelease: move |()| ed.end_gesture(),
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CropEdge {
    Top,
    Bottom,
    Left,
    Right,
}

/// The least of the video a crop leaves, as a fraction (§8).
const SMALLEST_CROP: f64 = 0.05;

/// Sets one side's inset in percent, keeping at least a little of the video.
pub fn set_crop_edge(project: &mut Project, edge: CropEdge, percent: f64) {
    let crop = &mut project.look.crop;
    let inset = (percent / 100.0).clamp(0.0, 0.5);
    match edge {
        CropEdge::Left => {
            let right = crop.x + crop.width;
            crop.x = inset.min(right - SMALLEST_CROP);
            crop.width = right - crop.x;
        }
        CropEdge::Right => {
            let right = (1.0 - inset).max(crop.x + SMALLEST_CROP);
            crop.width = right - crop.x;
        }
        CropEdge::Top => {
            let bottom = crop.y + crop.height;
            crop.y = inset.min(bottom - SMALLEST_CROP);
            crop.height = bottom - crop.y;
        }
        CropEdge::Bottom => {
            let bottom = (1.0 - inset).max(crop.y + SMALLEST_CROP);
            crop.height = bottom - crop.y;
        }
    }
}

#[component]
fn CursorPanel(project: Project) -> Element {
    let ed = use_ed();
    let cursor = project.cursor.clone();
    let effect = match cursor.click_effect {
        ClickEffect::None => "none",
        ClickEffect::Spotlight => "spotlight",
        ClickEffect::Ripple => "ripple",
        ClickEffect::Echo => "echo",
    };
    let has_data = project.media.cursor_data.is_some();
    rsx! {
        h2 { "Cursor" }
        if !has_data {
            p { class: "panel-note", "This video has no cursor data, so these settings have nothing to show." }
        }
        ToggleRow {
            label: "Show cursor",
            value: cursor.show,
            onchange: move |show| { ed.change("cursor-show", |p| p.cursor.show = show); ed.end_gesture(); },
        }
        ToggleRow {
            label: "Loop",
            value: cursor.looped,
            onchange: move |looped| { ed.change("cursor-loop", |p| p.cursor.looped = looped); ed.end_gesture(); },
        }
        ChoiceRow {
            label: "Style",
            options: options(&[("tahoe", "Pointer"), ("tahoe-inverted", "Inverted"), ("mono", "Mono"), ("dot", "Dot"), ("minimal", "Minimal")]),
            value: cursor.style.clone(),
            onchange: move |style: String| { ed.change("cursor-style", |p| p.cursor.style = style); ed.end_gesture(); },
        }
        SliderRow { label: "Size", value: cursor.size, min: 0.5, max: 10.0, step: 0.05, default: 2.5,
            onchange: move |value| ed.change("cursor-size", |p| p.cursor.size = value),
            onrelease: move |()| ed.end_gesture(),
        }
        SliderRow { label: "Click bounce", value: cursor.click_bounce, min: 0.0, max: 5.0, step: 0.05, default: 3.5,
            onchange: move |value| ed.change("cursor-bounce", |p| p.cursor.click_bounce = value),
            onrelease: move |()| ed.end_gesture(),
        }
        SliderRow { label: "Bounce speed", value: cursor.click_bounce_ms, min: 60.0, max: 500.0, step: 5.0, default: 350.0, format: Format::Milliseconds,
            onchange: move |value| ed.change("cursor-bounce-speed", |p| p.cursor.click_bounce_ms = value),
            onrelease: move |()| ed.end_gesture(),
        }
        SliderRow { label: "Sway", value: cursor.sway / 2.0, min: 0.0, max: 1.0, step: 0.025, default: 0.2,
            onchange: move |value: f64| ed.change("cursor-sway", |p| p.cursor.sway = value * 2.0),
            onrelease: move |()| ed.end_gesture(),
        }
        SliderRow { label: "Motion blur", value: cursor.motion_blur, min: 0.0, max: 2.0, step: 0.05, default: 0.6,
            onchange: move |value| ed.change("cursor-blur", |p| p.cursor.motion_blur = value),
            onrelease: move |()| ed.end_gesture(),
        }
        Section { title: "Click effect",
            ChoiceRow {
                label: "Effect",
                options: options(&[("none", "Off"), ("spotlight", "Spotlight"), ("ripple", "Ripple"), ("echo", "Echo")]),
                value: effect.to_string(),
                onchange: move |choice: String| {
                    ed.change("cursor-effect", |p| {
                        p.cursor.click_effect = match choice.as_str() {
                            "spotlight" => ClickEffect::Spotlight,
                            "ripple" => ClickEffect::Ripple,
                            "echo" => ClickEffect::Echo,
                            _ => ClickEffect::None,
                        };
                    });
                    ed.end_gesture();
                },
            }
            if cursor.click_effect != ClickEffect::None {
                ColorPicker {
                    label: "Colour",
                    value: cursor.click_effect_color.clone(),
                    swatches: EFFECT_SWATCHES.to_vec(),
                    onchange: move |color: String| ed.change("cursor-effect-color", |p| p.cursor.click_effect_color = color),
                }
                SliderRow { label: "Size", value: cursor.click_effect_scale, min: 0.5, max: 2.0, step: 0.05, default: 1.0,
                    onchange: move |value| ed.change("cursor-effect-size", |p| p.cursor.click_effect_scale = value),
                    onrelease: move |()| ed.end_gesture(),
                }
                SliderRow { label: "Opacity", value: cursor.click_effect_opacity, min: 0.0, max: 1.0, step: 0.01, default: 1.0,
                    onchange: move |value| ed.change("cursor-effect-opacity", |p| p.cursor.click_effect_opacity = value),
                    onrelease: move |()| ed.end_gesture(),
                }
                SliderRow { label: "Duration", value: cursor.click_effect_ms, min: 120.0, max: 1200.0, step: 10.0, default: 600.0, format: Format::Milliseconds,
                    onchange: move |value| ed.change("cursor-effect-duration", |p| p.cursor.click_effect_ms = value),
                    onrelease: move |()| ed.end_gesture(),
                }
            }
        }
    }
}

#[component]
fn WebcamPanel(project: Project) -> Element {
    let ed = use_ed();
    let webcam = project.webcam.clone();
    let positions = [
        (WebcamPosition::TopLeft, "Top left"),
        (WebcamPosition::TopCenter, "Top"),
        (WebcamPosition::TopRight, "Top right"),
        (WebcamPosition::CenterLeft, "Left"),
        (WebcamPosition::Center, "Centre"),
        (WebcamPosition::CenterRight, "Right"),
        (WebcamPosition::BottomLeft, "Bottom left"),
        (WebcamPosition::BottomCenter, "Bottom"),
        (WebcamPosition::BottomRight, "Bottom right"),
    ];
    let custom = webcam.position == WebcamPosition::Custom;
    rsx! {
        h2 { "Webcam" }
        ToggleRow {
            label: "Show webcam",
            value: webcam.enabled,
            onchange: move |enabled| { ed.change("webcam-show", |p| p.webcam.enabled = enabled); ed.end_gesture(); },
        }
        div { class: "row-buttons",
            button {
                class: "row-button",
                id: "webcam-footage",
                onclick: move |_| {
                    let picked = rfd::FileDialog::new()
                        .set_title("Choose webcam footage")
                        .add_filter("Videos", &["mp4", "mov", "m4v"])
                        .pick_file();
                    if let Some(path) = picked {
                        ed.change("webcam-file", |p| {
                            p.webcam.source = Some(reel_core::project::MediaRef::absolute(path));
                            p.webcam.enabled = true;
                        });
                        ed.end_gesture();
                    }
                },
                if webcam.source.is_some() { "Replace footage…" } else { "Add footage…" }
            }
            if webcam.source.is_some() {
                button {
                    class: "row-button",
                    onclick: move |_| {
                        ed.change("webcam-file", |p| {
                            p.webcam.source = None;
                            p.webcam.enabled = false;
                        });
                        ed.end_gesture();
                    },
                    "Remove"
                }
            }
        }
        ToggleRow {
            label: "Shrink when zoomed in",
            value: webcam.react_to_zoom,
            onchange: move |react| { ed.change("webcam-react", |p| p.webcam.react_to_zoom = react); ed.end_gesture(); },
        }
        ToggleRow {
            label: "Mirror",
            value: webcam.mirror,
            onchange: move |mirror| { ed.change("webcam-mirror", |p| p.webcam.mirror = mirror); ed.end_gesture(); },
        }
        SliderRow { label: "Width", value: webcam.width, min: 10.0, max: 100.0, step: 1.0, default: 40.0, format: Format::Percent,
            onchange: move |value| ed.change("webcam-width", |p| p.webcam.width = value),
            onrelease: move |()| ed.end_gesture(),
        }
        SliderRow { label: "Height", value: webcam.height, min: 10.0, max: 100.0, step: 1.0, default: 40.0, format: Format::Percent,
            onchange: move |value| ed.change("webcam-height", |p| p.webcam.height = value),
            onrelease: move |()| ed.end_gesture(),
        }
        Section { title: "Position",
            div { class: "row-grid", role: "group", "aria-label": "Position",
                for (position, name) in positions {
                    button {
                        "aria-label": "{name}",
                        title: "{name}",
                        "data-selected": "{webcam.position == position}",
                        onclick: move |_| { ed.change("webcam-position", |p| p.webcam.position = position); ed.end_gesture(); },
                    }
                }
            }
            ToggleRow {
                label: "Custom position",
                value: custom,
                onchange: move |on: bool| {
                    ed.change("webcam-position", |p| {
                        p.webcam.position = if on { WebcamPosition::Custom } else { WebcamPosition::BottomRight };
                    });
                    ed.end_gesture();
                },
            }
            if custom {
                SliderRow { label: "Across", value: webcam.custom_x * 100.0, min: 0.0, max: 100.0, step: 1.0, default: 100.0, format: Format::Percent,
                    onchange: move |value: f64| ed.change("webcam-x", |p| p.webcam.custom_x = value / 100.0),
                    onrelease: move |()| ed.end_gesture(),
                }
                SliderRow { label: "Down", value: webcam.custom_y * 100.0, min: 0.0, max: 100.0, step: 1.0, default: 100.0, format: Format::Percent,
                    onchange: move |value: f64| ed.change("webcam-y", |p| p.webcam.custom_y = value / 100.0),
                    onrelease: move |()| ed.end_gesture(),
                }
            }
        }
        Section { title: "Look",
            SliderRow { label: "Margin", value: webcam.margin, min: 0.0, max: 96.0, step: 1.0, default: 24.0, format: Format::Pixels,
                onchange: move |value| ed.change("webcam-margin", |p| p.webcam.margin = value),
                onrelease: move |()| ed.end_gesture(),
            }
            SliderRow { label: "Roundness", value: webcam.corner_radius, min: 0.0, max: 160.0, step: 1.0, default: 90.0, format: Format::Pixels,
                onchange: move |value| ed.change("webcam-roundness", |p| p.webcam.corner_radius = value),
                onrelease: move |()| ed.end_gesture(),
            }
            SliderRow { label: "Shadow", value: webcam.shadow, min: 0.0, max: 1.0, step: 0.01, default: 0.67,
                onchange: move |value| ed.change("webcam-shadow", |p| p.webcam.shadow = value),
                onrelease: move |()| ed.end_gesture(),
            }
            // Recordly has no control for this (§11), though a late camera often needs it.
            SliderRow { label: "Sync offset", value: webcam.offset.ms(), min: -2000.0, max: 2000.0, step: 10.0, default: webcam.offset.ms(), format: Format::Milliseconds,
                onchange: move |value: f64| ed.change("webcam-offset", |p| p.webcam.offset = reel_core::time::Micros::from_ms(value)),
                onrelease: move |()| ed.end_gesture(),
            }
        }
    }
}

#[component]
fn CaptionsPanel(project: Project) -> Element {
    let ed = use_ed();
    let style = project.caption_style.clone();
    let animation = match style.animation {
        CaptionAnimation::None => "none",
        CaptionAnimation::Fade => "fade",
        CaptionAnimation::Rise => "rise",
        CaptionAnimation::Pop => "pop",
    };
    rsx! {
        h2 { "Captions" }
        super::captions::GeneratePanel { language: style.language.clone(), has_captions: !project.timeline.captions.is_empty() }
        Section { title: "Style",
            ToggleRow {
                label: "Show captions",
                value: style.show,
                onchange: move |show| { ed.change("captions-show", |p| p.caption_style.show = show); ed.end_gesture(); },
            }
            ToggleRow {
                label: "Hover to add on the timeline",
                value: style.hover_to_add,
                onchange: move |hover| { ed.change("captions-hover", |p| p.caption_style.hover_to_add = hover); ed.end_gesture(); },
            }
            ChoiceRow {
                label: "Animation",
                options: options(&[("none", "Off"), ("fade", "Fade"), ("rise", "Rise"), ("pop", "Pop")]),
                value: animation.to_string(),
                onchange: move |choice: String| {
                    ed.change("captions-animation", |p| {
                        p.caption_style.animation = match choice.as_str() {
                            "none" => CaptionAnimation::None,
                            "rise" => CaptionAnimation::Rise,
                            "pop" => CaptionAnimation::Pop,
                            _ => CaptionAnimation::Fade,
                        };
                    });
                    ed.end_gesture();
                },
            }
            ColorPicker {
                label: "Text colour",
                value: style.text_color.clone(),
                swatches: vec!["#FFFFFF", "#000000", "#FFD700", "#00BCD4"],
                onchange: move |color: String| ed.change("captions-color", |p| p.caption_style.text_color = color),
            }
            SliderRow { label: "Font size", value: style.font_size, min: 16.0, max: 72.0, step: 1.0, default: 30.0,
                onchange: move |value| ed.change("captions-size", |p| p.caption_style.font_size = value),
                onrelease: move |()| ed.end_gesture(),
            }
            SliderRow { label: "Rows", value: f64::from(style.max_rows), min: 1.0, max: 4.0, step: 1.0, default: 1.0,
                onchange: move |value: f64| ed.change("captions-rows", |p| p.caption_style.max_rows = value as u32),
                onrelease: move |()| ed.end_gesture(),
            }
            SliderRow { label: "Bottom offset", value: style.bottom_offset, min: 0.0, max: 30.0, step: 1.0, default: 3.0, format: Format::Percent,
                onchange: move |value| ed.change("captions-offset", |p| p.caption_style.bottom_offset = value),
                onrelease: move |()| ed.end_gesture(),
            }
            SliderRow { label: "Max width", value: style.max_width, min: 40.0, max: 95.0, step: 1.0, default: 62.0, format: Format::Percent,
                onchange: move |value| ed.change("captions-width", |p| p.caption_style.max_width = value),
                onrelease: move |()| ed.end_gesture(),
            }
            SliderRow { label: "Box radius", value: style.box_radius, min: 0.0, max: 40.0, step: 0.5, default: 17.5,
                onchange: move |value| ed.change("captions-radius", |p| p.caption_style.box_radius = value),
                onrelease: move |()| ed.end_gesture(),
            }
            SliderRow { label: "Background", value: style.background_opacity, min: 0.0, max: 1.0, step: 0.01, default: 0.9,
                onchange: move |value| ed.change("captions-background", |p| p.caption_style.background_opacity = value),
                onrelease: move |()| ed.end_gesture(),
            }
        }
    }
}

fn settings_path() -> std::path::PathBuf {
    recording::support_folder().join("settings.json")
}

/// Puts a motion preset's values on a project (§19).
pub fn apply_motion_preset(project: &mut Project, preset: MotionPreset) {
    let (zoom_in, zoom_out, stiffness, damping) = match preset {
        MotionPreset::Focused => (200.0, 200.0, 1.35, 0.79),
        MotionPreset::Smooth => (1522.575, 1015.05, 0.92, 1.36),
    };
    project.zoom_motion.zoom_in_ms = zoom_in;
    project.zoom_motion.zoom_out_ms = zoom_out;
    project.cursor.spring.stiffness = stiffness;
    project.cursor.spring.damping = damping;
    project.cursor.spring.mass = 1.29;
    project.cursor.size = 2.5;
    project.cursor.smoothing = 0.67;
    project.cursor.click_bounce = 3.5;
    project.cursor.click_bounce_ms = 350.0;
}

#[component]
fn SettingsPanel(project: Project) -> Element {
    let ed = use_ed();
    let mut stored = use_signal(|| SettingsStore::open(&settings_path()).get().clone());
    let mut overlay = ed.overlay;
    let mut dark = ed.dark;
    let theme = match stored().theme {
        Theme::Light => "light",
        Theme::Dark => "dark",
        Theme::System => "system",
    };
    let motion = match stored().motion_preset {
        MotionPreset::Focused => "focused",
        MotionPreset::Smooth => "smooth",
    };
    let mut save = move |update: Box<dyn FnOnce(&mut reel_core::settings::Settings)>| {
        let mut store = SettingsStore::open(&settings_path());
        let _ = store.update(update);
        stored.set(store.get().clone());
    };
    rsx! {
        h2 { "Settings" }
        ChoiceRow {
            label: "Appearance",
            options: options(&[("light", "Light"), ("dark", "Dark"), ("system", "System")]),
            value: theme.to_string(),
            onchange: move |choice: String| {
                let value = match choice.as_str() {
                    "light" => Theme::Light,
                    "dark" => Theme::Dark,
                    _ => Theme::System,
                };
                save(Box::new(move |settings| settings.theme = value));
                dark.set(crate::theme_is_dark(value));
            },
        }
        ChoiceRow {
            label: "Motion",
            options: options(&[("focused", "Focused"), ("smooth", "Smooth")]),
            value: motion.to_string(),
            onchange: move |choice: String| {
                let value = if choice == "smooth" { MotionPreset::Smooth } else { MotionPreset::Focused };
                save(Box::new(move |settings| settings.motion_preset = value));
                ed.change("motion-preset", |p| apply_motion_preset(p, value));
                ed.end_gesture();
            },
        }
        ToggleRow {
            label: "Add zooms to new recordings",
            value: stored().auto_suggest_zooms,
            onchange: move |on: bool| save(Box::new(move |settings| settings.auto_suggest_zooms = on)),
        }
        ToggleRow {
            label: "Connect neighbouring zooms",
            value: project.zoom_motion.connect_zooms,
            onchange: move |on| { ed.change("connect-zooms", |p| p.zoom_motion.connect_zooms = on); ed.end_gesture(); },
        }
        ToggleRow {
            label: "Classic zoom animation",
            value: project.zoom_motion.classic,
            onchange: move |on| { ed.change("classic-zoom", |p| p.zoom_motion.classic = on); ed.end_gesture(); },
        }
        SliderRow { label: "Zoom motion blur", value: project.zoom_motion.motion_blur, min: 0.0, max: 2.0, step: 0.05, default: 0.35,
            onchange: move |value| ed.change("zoom-blur", |p| p.zoom_motion.motion_blur = value),
            onrelease: move |()| ed.end_gesture(),
        }
        ToggleRow {
            label: "Keep old recordings for 14 days only",
            value: stored().retention_enabled,
            onchange: move |on: bool| save(Box::new(move |settings| settings.retention_enabled = on)),
        }
        ToggleRow {
            label: "Experimental updates",
            value: stored().experimental_updates,
            onchange: move |on: bool| save(Box::new(move |settings| settings.experimental_updates = on)),
        }
        div { class: "row-buttons",
            button {
                class: "row-button",
                id: "open-shortcuts",
                onclick: move |_| overlay.set(Overlay::Shortcuts),
                "Keyboard shortcuts…"
            }
        }
    }
}

fn delete_button(label: &'static str, ed: Ed, edit: Edit) -> Element {
    rsx! {
        div { class: "row-buttons",
            button {
                class: "row-button row-danger",
                id: "delete-selected",
                onclick: move |_| {
                    if ed.apply(&edit).is_some() {
                        ed.select(Selection::None);
                    }
                },
                "{label}"
            }
        }
    }
}

#[component]
fn ClipPanel(id: ItemId) -> Element {
    let ed = use_ed();
    let Some(project) = ed.project() else {
        return rsx! {};
    };
    let Some(clip) = project
        .timeline
        .clips
        .iter()
        .find(|clip| clip.id == id)
        .cloned()
    else {
        return rsx! {};
    };
    let tracks = project.media.source_tracks.clone();
    let id_mute = id.clone();
    let id_audio = id.clone();
    rsx! {
        h2 { "Clip" }
        div { class: "row-choice",
            span { class: "row-label", "Speed" }
            div { class: "row-speeds", role: "group", "aria-label": "Speed",
                for speed in CLIP_SPEEDS {
                    {
                        let id = id.clone();
                        rsx! {
                            button {
                                "data-selected": "{(clip.speed - speed).abs() < 1e-9}",
                                onclick: move |_| { ed.apply(&Edit::SetClipSpeed { id: id.clone(), speed }); },
                                "{speed}×"
                            }
                        }
                    }
                }
            }
        }
        ToggleRow {
            label: "Mute this clip",
            value: clip.muted,
            onchange: move |muted| { ed.apply(&Edit::SetClipMuted { id: id_mute.clone(), muted }); },
        }
        ToggleRow {
            label: "Show its audio",
            value: clip.show_source_audio,
            onchange: move |show| { ed.apply(&Edit::SetClipSourceAudio { id: id_audio.clone(), show }); },
        }
        if !tracks.is_empty() {
            Section { title: "Source tracks",
                for (index, track) in tracks.iter().enumerate() {
                    {
                        let name = match track.kind {
                            reel_core::project::SourceTrackKind::System => "System audio",
                            reel_core::project::SourceTrackKind::Mic => "Microphone",
                            reel_core::project::SourceTrackKind::Mixed => "Source",
                        };
                        let boost = track.boost;
                        rsx! {
                            SliderRow {
                                label: "{name}",
                                value: track.volume,
                                min: 0.0,
                                max: 1.0,
                                step: 0.01,
                                default: 1.0,
                                onchange: move |value| ed.change("track-volume", |p| {
                                    if let Some(item) = p.media.source_tracks.get_mut(index) {
                                        item.volume = value;
                                    }
                                }),
                                onrelease: move |()| ed.end_gesture(),
                            }
                            // It's a fixed ×1.35 gain, so it says so rather than "Normalize" (BUILD_PLAN 7.6).
                            ToggleRow {
                                label: "Boost {name} by ×1.35",
                                value: boost,
                                onchange: move |on| {
                                    ed.change("track-boost", |p| {
                                        if let Some(item) = p.media.source_tracks.get_mut(index) {
                                            item.boost = on;
                                        }
                                    });
                                    ed.end_gesture();
                                },
                            }
                        }
                    }
                }
            }
        }
        {delete_button("Delete clip", ed, Edit::DeleteClip { id: clip.id.clone() })}
    }
}

#[component]
fn ZoomPanel(id: ItemId) -> Element {
    let ed = use_ed();
    let Some(project) = ed.project() else {
        return rsx! {};
    };
    let Some(zoom) = project
        .timeline
        .zooms
        .iter()
        .find(|zoom| zoom.id == id)
        .cloned()
    else {
        return rsx! {};
    };
    let id_mode = id.clone();
    let mode = if zoom.mode == ZoomMode::Manual {
        "manual"
    } else {
        "auto"
    };
    rsx! {
        h2 { "Zoom" }
        div { class: "row-choice",
            span { class: "row-label", "Depth" }
            div { class: "row-choice-options", role: "group", "aria-label": "Depth",
                for (depth, scale) in ZOOM_SCALES.iter().enumerate() {
                    {
                        let id = id.clone();
                        rsx! {
                            button {
                                id: "zoom-depth-{depth}",
                                "data-selected": "{usize::from(zoom.depth) == depth}",
                                onclick: move |_| { ed.apply(&Edit::UpdateZoom { id: id.clone(), depth: Some(depth as u8), focus: None, mode: None }); },
                                "{scale}×"
                            }
                        }
                    }
                }
            }
        }
        ChoiceRow {
            label: "Follow",
            options: options(&[("auto", "Cursor"), ("manual", "Fixed spot")]),
            value: mode.to_string(),
            onchange: move |choice: String| {
                let mode = if choice == "manual" { ZoomMode::Manual } else { ZoomMode::Auto };
                ed.apply(&Edit::UpdateZoom { id: id_mode.clone(), depth: None, focus: None, mode: Some(mode) });
            },
        }
        if zoom.mode == ZoomMode::Manual {
            p { class: "panel-note", "Pause, then click or drag on the preview to choose where it looks." }
        }
        {delete_button("Delete zoom", ed, Edit::DeleteZoom { id: zoom.id.clone() })}
    }
}

#[component]
fn AllZoomsPanel() -> Element {
    let ed = use_ed();
    let count = ed
        .project()
        .map_or(0, |project| project.timeline.zooms.len());
    rsx! {
        h2 { "{count} zooms" }
        {delete_button("Delete all zooms", ed, Edit::DeleteAllZooms)}
    }
}

#[component]
fn AudioPanel(id: ItemId) -> Element {
    let ed = use_ed();
    let Some(project) = ed.project() else {
        return rsx! {};
    };
    let Some(item) = project
        .timeline
        .audio
        .iter()
        .find(|item| item.id == id)
        .cloned()
    else {
        return rsx! {};
    };
    let id_volume = id.clone();
    let id_boost = id.clone();
    rsx! {
        h2 { "Audio" }
        SliderRow { label: "Volume", value: item.volume, min: 0.0, max: 1.0, step: 0.01, default: 1.0,
            onchange: move |volume| {
                let edit = Edit::SetAudioVolume { id: id_volume.clone(), volume };
                ed.change("audio-volume", |p| { let _ = p.timeline.apply(&edit); });
            },
            onrelease: move |()| ed.end_gesture(),
        }
        ToggleRow {
            label: "Boost by ×1.35",
            value: item.boost,
            onchange: move |boost| { ed.apply(&Edit::SetAudioBoost { id: id_boost.clone(), boost }); },
        }
        {delete_button("Delete audio", ed, Edit::DeleteAudio { id: item.id.clone() })}
    }
}

fn caption_time_input(
    label: &'static str,
    value: String,
    on_commit: EventHandler<reel_core::time::TimelineTime>,
) -> Element {
    rsx! {
        label { class: "row-time",
            span { class: "row-label", "{label}" }
            input {
                r#type: "text",
                value: "{value}",
                spellcheck: "false",
                onkeydown: move |event| event.stop_propagation(),
                onchange: move |event| {
                    if let Some(time) = parse_time(&event.value()) {
                        on_commit.call(time);
                    }
                },
            }
        }
    }
}

#[component]
fn CaptionPanel(id: ItemId) -> Element {
    let ed = use_ed();
    let Some(project) = ed.project() else {
        return rsx! {};
    };
    let Some(caption) = project
        .timeline
        .captions
        .iter()
        .find(|item| item.id == id)
        .cloned()
    else {
        return rsx! {};
    };
    let id_text = id.clone();
    let (id_start, id_end, id_split, id_merge) = (id.clone(), id.clone(), id.clone(), id.clone());
    let next = project
        .timeline
        .captions
        .iter()
        .find(|item| item.start >= caption.end)
        .map(|item| item.id.clone());
    rsx! {
        h2 { "Caption" }
        label { class: "row-text",
            span { class: "row-label", "Text" }
            textarea {
                id: "caption-text",
                rows: "3",
                value: "{caption.text}",
                onkeydown: move |event| event.stop_propagation(),
                oninput: move |event| {
                    let edit = Edit::SetCaptionText { id: id_text.clone(), text: event.value() };
                    ed.change("caption-text", |p| { let _ = p.timeline.apply(&edit); });
                },
                onblur: move |_| ed.end_gesture(),
            }
        }
        {caption_time_input("Start", exact_time(caption.start), EventHandler::new(move |to| {
            ed.apply(&Edit::ResizeCaption { id: id_start.clone(), edge: reel_core::edit::Edge::Start, to });
        }))}
        {caption_time_input("End", exact_time(caption.end), EventHandler::new(move |to| {
            ed.apply(&Edit::ResizeCaption { id: id_end.clone(), edge: reel_core::edit::Edge::End, to });
        }))}
        div { class: "row-buttons",
            button {
                class: "row-button",
                id: "caption-split",
                onclick: move |_| super::captions::split_caption(ed, &id_split),
                "Split at playhead"
            }
            button {
                class: "row-button",
                id: "caption-merge",
                disabled: next.is_none(),
                onclick: move |_| super::captions::merge_caption(ed, &id_merge),
                "Merge with next"
            }
        }
        {delete_button("Delete caption", ed, Edit::DeleteCaption { id: caption.id.clone() })}
    }
}

#[component]
fn AnnotationPanel(id: ItemId) -> Element {
    let ed = use_ed();
    let Some(project) = ed.project() else {
        return rsx! {};
    };
    let Some(item) = project
        .timeline
        .annotations
        .iter()
        .find(|item| item.id == id)
        .cloned()
    else {
        return rsx! {};
    };
    let content = item.content.clone();
    let update = move |id: ItemId, content: AnnotationContent, gesture: &str| {
        let edit = Edit::UpdateAnnotation { id, content };
        ed.change(gesture, |p| {
            let _ = p.timeline.apply(&edit);
        });
    };
    let body = match content {
        AnnotationContent::Text { text, style } => {
            let id_text = id.clone();
            let (style_text, style_font, style_size, style_color, style_background) = (
                style.clone(),
                style.clone(),
                style.clone(),
                style.clone(),
                style.clone(),
            );
            let (style_bold, style_italic, style_underline, style_align) =
                (style.clone(), style.clone(), style.clone(), style.clone());
            let (text_font, text_size, text_color, text_background) =
                (text.clone(), text.clone(), text.clone(), text.clone());
            let (text_bold, text_italic, text_underline, text_align) =
                (text.clone(), text.clone(), text.clone(), text.clone());
            let (id_font, id_size, id_color, id_background) =
                (id.clone(), id.clone(), id.clone(), id.clone());
            let (id_bold, id_italic, id_underline, id_align) =
                (id.clone(), id.clone(), id.clone(), id.clone());
            let align = match style.align {
                TextAlign::Left => "left",
                TextAlign::Center => "center",
                TextAlign::Right => "right",
            };
            rsx! {
                label { class: "row-text",
                    span { class: "row-label", "Text" }
                    textarea {
                        id: "annotation-text",
                        rows: "3",
                        value: "{text}",
                        onkeydown: move |event| event.stop_propagation(),
                        oninput: move |event| update(id_text.clone(), AnnotationContent::Text { text: event.value(), style: style_text.clone() }, "annotation-text"),
                        onblur: move |_| ed.end_gesture(),
                    }
                }
                div { class: "row-choice",
                    span { class: "row-label", "Font" }
                    select {
                        "aria-label": "Font",
                        value: "{style.font_family}",
                        onchange: move |event| {
                            let mut next = style_font.clone();
                            next.font_family = event.value();
                            update(id_font.clone(), AnnotationContent::Text { text: text_font.clone(), style: next }, "annotation-font");
                            ed.end_gesture();
                        },
                        for (name, family) in FONTS {
                            option { value: "{family}", selected: family == style.font_family, "{name}" }
                        }
                    }
                }
                div { class: "row-choice",
                    span { class: "row-label", "Size" }
                    select {
                        "aria-label": "Size",
                        value: "{style.font_size}",
                        onchange: move |event| {
                            if let Ok(size) = event.value().parse::<f64>() {
                                let mut next = style_size.clone();
                                next.font_size = size;
                                update(id_size.clone(), AnnotationContent::Text { text: text_size.clone(), style: next }, "annotation-size");
                                ed.end_gesture();
                            }
                        },
                        for size in TEXT_SIZES {
                            option { value: "{size}", selected: (f64::from(size) - style.font_size).abs() < 0.5, "{size}" }
                        }
                    }
                }
                ToggleRow {
                    label: "Bold",
                    value: style.bold,
                    onchange: move |bold| {
                        let mut next = style_bold.clone();
                        next.bold = bold;
                        update(id_bold.clone(), AnnotationContent::Text { text: text_bold.clone(), style: next }, "annotation-bold");
                        ed.end_gesture();
                    },
                }
                ToggleRow {
                    label: "Italic",
                    value: style.italic,
                    onchange: move |italic| {
                        let mut next = style_italic.clone();
                        next.italic = italic;
                        update(id_italic.clone(), AnnotationContent::Text { text: text_italic.clone(), style: next }, "annotation-italic");
                        ed.end_gesture();
                    },
                }
                ToggleRow {
                    label: "Underline",
                    value: style.underline,
                    onchange: move |underline| {
                        let mut next = style_underline.clone();
                        next.underline = underline;
                        update(id_underline.clone(), AnnotationContent::Text { text: text_underline.clone(), style: next }, "annotation-underline");
                        ed.end_gesture();
                    },
                }
                ChoiceRow {
                    label: "Align",
                    options: options(&[("left", "Left"), ("center", "Centre"), ("right", "Right")]),
                    value: align.to_string(),
                    onchange: move |choice: String| {
                        let mut next = style_align.clone();
                        next.align = match choice.as_str() {
                            "left" => TextAlign::Left,
                            "right" => TextAlign::Right,
                            _ => TextAlign::Center,
                        };
                        update(id_align.clone(), AnnotationContent::Text { text: text_align.clone(), style: next }, "annotation-align");
                        ed.end_gesture();
                    },
                }
                ColorPicker {
                    label: "Text colour",
                    value: style.color.clone(),
                    swatches: COLOR_SWATCHES.to_vec(),
                    onchange: move |color: String| {
                        let mut next = style_color.clone();
                        next.color = color;
                        update(id_color.clone(), AnnotationContent::Text { text: text_color.clone(), style: next }, "annotation-color");
                    },
                }
                ColorPicker {
                    label: "Background",
                    value: style.background.clone(),
                    swatches: COLOR_SWATCHES.to_vec(),
                    onchange: move |color: String| {
                        let mut next = style_background.clone();
                        next.background = color;
                        update(id_background.clone(), AnnotationContent::Text { text: text_background.clone(), style: next }, "annotation-background");
                    },
                }
            }
        }
        AnnotationContent::Arrow {
            direction,
            color,
            stroke_width,
        } => {
            let directions = [
                (ArrowDirection::UpLeft, "↖"),
                (ArrowDirection::Up, "↑"),
                (ArrowDirection::UpRight, "↗"),
                (ArrowDirection::Left, "←"),
                (ArrowDirection::Right, "→"),
                (ArrowDirection::DownLeft, "↙"),
                (ArrowDirection::Down, "↓"),
                (ArrowDirection::DownRight, "↘"),
            ];
            let (color_direction, color_stroke, id_stroke, id_color) =
                (color.clone(), color.clone(), id.clone(), id.clone());
            rsx! {
                div { class: "row-arrows", role: "group", "aria-label": "Direction",
                    for (choice, symbol) in directions {
                        {
                            let id = id.clone();
                            let color = color_direction.clone();
                            rsx! {
                                button {
                                    "data-selected": "{direction == choice}",
                                    onclick: move |_| {
                                        update(id.clone(), AnnotationContent::Arrow { direction: choice, color: color.clone(), stroke_width }, "annotation-direction");
                                        ed.end_gesture();
                                    },
                                    "{symbol}"
                                }
                            }
                        }
                    }
                }
                SliderRow { label: "Stroke", value: stroke_width, min: 1.0, max: 6.0, step: 0.5, default: 4.0,
                    onchange: move |width| update(id_stroke.clone(), AnnotationContent::Arrow { direction, color: color_stroke.clone(), stroke_width: width }, "annotation-stroke"),
                    onrelease: move |()| ed.end_gesture(),
                }
                ColorPicker {
                    label: "Colour",
                    value: color.clone(),
                    swatches: COLOR_SWATCHES.to_vec(),
                    onchange: move |color: String| update(id_color.clone(), AnnotationContent::Arrow { direction, color, stroke_width }, "annotation-color"),
                }
            }
        }
        AnnotationContent::Blur { strength, fill } => {
            let (fill_strength, id_strength, id_fill) = (fill.clone(), id.clone(), id.clone());
            let choice = match fill.as_deref() {
                None => "none",
                Some("#000000") => "black",
                Some("#FFFFFF") => "white",
                Some(_) => "custom",
            };
            rsx! {
                SliderRow { label: "Strength", value: strength, min: 1.0, max: 100.0, step: 1.0, default: 20.0,
                    onchange: move |value| update(id_strength.clone(), AnnotationContent::Blur { strength: value, fill: fill_strength.clone() }, "annotation-strength"),
                    onrelease: move |()| ed.end_gesture(),
                }
                ChoiceRow {
                    label: "Fill",
                    options: options(&[("none", "None"), ("black", "Black"), ("white", "White")]),
                    value: choice.to_string(),
                    onchange: move |picked: String| {
                        let fill = match picked.as_str() {
                            "black" => Some("#000000".to_string()),
                            "white" => Some("#FFFFFF".to_string()),
                            _ => None,
                        };
                        update(id_fill.clone(), AnnotationContent::Blur { strength, fill }, "annotation-fill");
                        ed.end_gesture();
                    },
                }
            }
        }
        AnnotationContent::Image { .. } => {
            rsx! { p { class: "panel-note", "Drag it on the preview to move it, and by a corner to resize it." } }
        }
    };
    rsx! {
        h2 { "Annotation" }
        {body}
        {delete_button("Delete annotation", ed, Edit::DeleteAnnotation { id: item.id.clone() })}
    }
}

/// A new annotation (§13): a second long, centred, at 30×20% of the frame.
#[must_use]
pub fn new_annotation(
    id: ItemId,
    start: reel_core::time::TimelineTime,
    content: AnnotationContent,
) -> reel_core::timeline::Annotation {
    reel_core::timeline::Annotation {
        id,
        start,
        end: start + reel_core::time::Micros::from_secs(1),
        content,
        position: Point { x: 35.0, y: 40.0 },
        width: 30.0,
        height: 20.0,
        track: 0,
        layer: 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reel_core::project::Crop;

    fn project() -> Project {
        Project::default()
    }

    fn near(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn each_crop_side_moves_only_that_side() {
        let mut p = project();
        set_crop_edge(&mut p, CropEdge::Left, 10.0);
        assert!(near(p.look.crop.x, 0.1) && near(p.look.crop.width, 0.9));
        set_crop_edge(&mut p, CropEdge::Right, 20.0);
        assert!(near(p.look.crop.x, 0.1) && near(p.look.crop.width, 0.7));
        set_crop_edge(&mut p, CropEdge::Top, 5.0);
        assert!(near(p.look.crop.y, 0.05) && near(p.look.crop.height, 0.95));
        set_crop_edge(&mut p, CropEdge::Bottom, 15.0);
        assert!(near(p.look.crop.y, 0.05) && near(p.look.crop.height, 0.8));
    }

    #[test]
    fn a_crop_always_leaves_some_of_the_video() {
        let mut p = project();
        p.look.crop = Crop {
            x: 0.5,
            y: 0.0,
            width: 0.5,
            height: 1.0,
        };
        set_crop_edge(&mut p, CropEdge::Right, 50.0);
        assert!(p.look.crop.width >= SMALLEST_CROP - 1e-9);
        assert!(p.look.crop.x + p.look.crop.width <= 1.0 + 1e-9);
    }

    #[test]
    fn motion_presets_set_the_documented_values() {
        let mut p = project();
        apply_motion_preset(&mut p, MotionPreset::Smooth);
        assert!(near(p.zoom_motion.zoom_in_ms, 1522.575));
        assert!(near(p.cursor.spring.stiffness, 0.92));
        apply_motion_preset(&mut p, MotionPreset::Focused);
        assert!(near(p.zoom_motion.zoom_out_ms, 200.0));
        assert!(near(p.cursor.spring.damping, 0.79));
    }

    #[test]
    fn a_new_annotation_is_centred_and_a_second_long() {
        let item = new_annotation(
            "a".into(),
            reel_core::time::TimelineTime::ZERO,
            AnnotationContent::Blur {
                strength: 20.0,
                fill: None,
            },
        );
        assert!(near(item.position.x + item.width / 2.0, 50.0));
        assert!(near(item.position.y + item.height / 2.0, 50.0));
        assert_eq!(item.end - item.start, reel_core::time::Micros::from_secs(1));
    }
}
