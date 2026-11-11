//! What can be recorded, and how each source is filtered and named.

use serde::{Deserialize, Serialize};

const SMALLEST_WINDOW: f64 = 50.0;

/// System items that show up as windows but aren't anything a person would record.
const SYSTEM_APPS: &[&str] = &[
    "com.apple.dock",
    "com.apple.controlcenter",
    "com.apple.notificationcenterui",
    "com.apple.WindowManager",
    "com.apple.Spotlight",
    "com.apple.systemuiserver",
    "com.apple.WindowServer",
];

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DisplaySource {
    pub id: u32,
    pub name: String,
    pub primary: bool,
    /// In points.
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    /// Pixels per point.
    pub scale: f64,
    #[serde(default)]
    pub thumbnail: Option<std::path::PathBuf>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WindowSource {
    pub id: u32,
    pub name: String,
    pub app: String,
    pub bundle: String,
    pub title: String,
    pub layer: i64,
    pub width: f64,
    pub height: f64,
    pub on_screen: bool,
    #[serde(default)]
    pub thumbnail: Option<std::path::PathBuf>,
}

/// Whether a bundle is part of Reel, whose own windows are never offered.
#[must_use]
pub fn is_reel(bundle: &str) -> bool {
    bundle == "dev.reel" || bundle.starts_with("dev.reel.")
}

/// `Screen N (Primary)`.
#[must_use]
pub fn display_name(number: usize, primary: bool) -> String {
    if primary {
        format!("Screen {number} (Primary)")
    } else {
        format!("Screen {number}")
    }
}

/// `App — Title`, or whichever of them there is.
#[must_use]
pub fn window_name(app: &str, title: &str) -> String {
    match (app.is_empty(), title.is_empty()) {
        (false, false) => format!("{app} — {title}"),
        (false, true) => app.to_string(),
        _ => title.to_string(),
    }
}

/// Whether a window belongs in the list: an ordinary window, of a useful size, that isn't ours.
#[must_use]
pub fn should_offer(window: &WindowSource) -> bool {
    window.layer == 0
        && window.on_screen
        && window.width >= SMALLEST_WINDOW
        && window.height >= SMALLEST_WINDOW
        && !is_reel(&window.bundle)
        && !SYSTEM_APPS.contains(&window.bundle.as_str())
        && !(window.app.is_empty() && window.title.is_empty())
}

/// Recording settings for a size. H.264 needs even sizes.
#[must_use]
pub fn even_size(width: f64, height: f64) -> (u32, u32) {
    let even = |value: f64| ((value.round() as u32) & !1).max(2);
    (even(width), even(height))
}

/// The bitrate for a recording: high enough that re-encoding on export doesn't show it.
#[must_use]
pub fn recording_bitrate(width: u32, height: u32) -> u32 {
    let pixels = u64::from(width) * u64::from(height);
    if pixels >= 3840 * 2160 {
        60_750_000
    } else if pixels >= 2560 * 1440 {
        37_800_000
    } else {
        24_300_000
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window(bundle: &str, width: f64, layer: i64) -> WindowSource {
        WindowSource {
            id: 1,
            name: "x".into(),
            app: "App".into(),
            bundle: bundle.into(),
            title: "Title".into(),
            layer,
            width,
            height: width,
            on_screen: true,
            thumbnail: None,
        }
    }

    #[test]
    fn displays_and_windows_are_named_as_documented() {
        assert_eq!(display_name(1, true), "Screen 1 (Primary)");
        assert_eq!(display_name(2, false), "Screen 2");
        assert_eq!(window_name("Safari", "Reel"), "Safari — Reel");
        assert_eq!(window_name("Safari", ""), "Safari");
        assert_eq!(window_name("", "Reel"), "Reel");
    }

    #[test]
    fn ordinary_windows_are_offered() {
        assert!(should_offer(&window("com.apple.Safari", 800.0, 0)));
    }

    #[test]
    fn tiny_windows_and_ones_on_other_layers_arent() {
        assert!(!should_offer(&window("com.apple.Safari", 49.0, 0)));
        assert!(should_offer(&window("com.apple.Safari", 50.0, 0)));
        assert!(!should_offer(&window("com.apple.Safari", 800.0, 25)));
    }

    #[test]
    fn system_items_and_reels_own_windows_arent() {
        assert!(!should_offer(&window("com.apple.dock", 800.0, 0)));
        assert!(!should_offer(&window("com.apple.controlcenter", 800.0, 0)));
        assert!(!should_offer(&window("dev.reel", 800.0, 0)));
        assert!(!should_offer(&window("dev.reel.app-test", 800.0, 0)));
        assert!(
            should_offer(&window("dev.reelly", 800.0, 0)),
            "a similar name is someone else's"
        );
    }

    #[test]
    fn windows_off_screen_or_with_no_name_arent() {
        let mut hidden = window("com.apple.Safari", 800.0, 0);
        hidden.on_screen = false;
        assert!(!should_offer(&hidden));
        let mut nameless = window("com.apple.Safari", 800.0, 0);
        nameless.app = String::new();
        nameless.title = String::new();
        assert!(!should_offer(&nameless));
    }

    #[test]
    fn sizes_come_out_even() {
        assert_eq!(even_size(1921.0, 1081.0), (1920, 1080));
        assert_eq!(even_size(1728.0, 1117.0), (1728, 1116));
    }

    #[test]
    fn bitrates_follow_the_table() {
        assert_eq!(recording_bitrate(1920, 1080), 24_300_000);
        assert_eq!(recording_bitrate(2560, 1440), 37_800_000);
        assert_eq!(recording_bitrate(3840, 2160), 60_750_000);
        assert_eq!(recording_bitrate(3456, 2234), 37_800_000);
    }
}
