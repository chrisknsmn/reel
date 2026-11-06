//! Where the recording sits on the stage (§31).

use reel_core::project::{Crop, Padding};

/// Padding is a percentage, and 100% takes up a fifth of the stage on each side.
const PADDING_SCALE: f64 = 0.2;
/// Top and bottom can go up to this when padding isn't linked, to position the frame.
const ADVANCED_VERTICAL_MAX: f64 = 250.0;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Rect {
    #[must_use]
    pub fn center(&self) -> (f64, f64) {
        (self.x + self.width / 2.0, self.y + self.height / 2.0)
    }
}

/// Where everything lands on a stage of a given size.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Layout {
    pub scale: f64,
    /// The visible, cropped part of the recording.
    pub frame: Rect,
    /// Where the whole recording would be if it weren't cropped.
    pub video: Rect,
}

#[must_use]
pub fn compute(stage: (f64, f64), padding: &Padding, crop: &Crop, video: (f64, f64)) -> Layout {
    let (width, height) = stage;
    let advanced = !padding.linked;
    let clamp_percent = |value: f64, max: f64| value.clamp(0.0, max);
    let vertical_max = if advanced { ADVANCED_VERTICAL_MAX } else { 100.0 };
    let (left, right) = (clamp_percent(padding.left, 100.0), clamp_percent(padding.right, 100.0));
    let (top, bottom) = (clamp_percent(padding.top, vertical_max), clamp_percent(padding.bottom, vertical_max));

    let left_fraction = left / 100.0 * PADDING_SCALE;
    let right_fraction = right / 100.0 * PADDING_SCALE;
    let top_fraction = top.min(100.0) / 100.0 * PADDING_SCALE;
    let bottom_fraction = bottom.min(100.0) / 100.0 * PADDING_SCALE;

    let max_width = width * (1.0 - left_fraction - right_fraction).max(0.0);
    let max_height = height * (1.0 - top_fraction - bottom_fraction).max(0.0);

    let (cropped_width, cropped_height) = (video.0 * crop.width, video.1 * crop.height);
    let fit = |available: f64, size: f64| if size > 0.0 { available / size } else { 0.0 };
    let scale = fit(max_width, cropped_width).min(fit(max_height, cropped_height));
    let (frame_width, frame_height) = (cropped_width * scale, cropped_height * scale);

    let center_x = left_fraction * width + max_width / 2.0;
    let center_y = if advanced {
        let travel = (height - frame_height).max(0.0);
        let centered = travel / 2.0;
        let offset = centered + (top - bottom) / ADVANCED_VERTICAL_MAX * centered;
        offset.clamp(0.0, travel) + frame_height / 2.0
    } else {
        top_fraction * height + max_height / 2.0
    };

    let frame = Rect {
        x: center_x - frame_width / 2.0,
        y: center_y - frame_height / 2.0,
        width: frame_width,
        height: frame_height,
    };
    let (full_width, full_height) = (video.0 * scale, video.1 * scale);
    Layout {
        scale,
        frame,
        video: Rect {
            x: frame.x - crop.x * full_width,
            y: frame.y - crop.y * full_height,
            width: full_width,
            height: full_height,
        },
    }
}

/// The corner radius in pixels, scaled so a look is the same at any output size.
#[must_use]
pub fn corner_radius_px(radius: f64, stage: (f64, f64)) -> f64 {
    radius * (stage.0 / 1920.0).min(stage.1 / 1080.0)
}

/// One of the three layers of the frame's shadow, in pixels at 1920×1080 (§31).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShadowLayer {
    pub offset_y: f64,
    pub blur: f64,
    pub alpha: f64,
}

/// The shadow's layers, the same on every path, unlike Recordly's several routes.
#[must_use]
pub fn shadow_layers(intensity: f64) -> [ShadowLayer; 3] {
    let i = intensity.clamp(0.0, 1.0);
    [
        ShadowLayer { offset_y: 12.0 * i, blur: 48.0 * i, alpha: 0.7 * i },
        ShadowLayer { offset_y: 4.0 * i, blur: 16.0 * i, alpha: 0.5 * i },
        ShadowLayer { offset_y: 2.0 * i, blur: 8.0 * i, alpha: 0.3 * i },
    ]
}

/// The output size for an aspect ratio like `16:9`, fitting a video's size.
///
/// `native` keeps the cropped video's own shape. Sizes are always even, as H.264 requires.
#[must_use]
pub fn stage_size(aspect: &str, video: (f64, f64), crop: &Crop) -> (u32, u32) {
    let even = |value: f64| ((value as u32) & !1).max(2);
    let parsed = aspect.split_once(':').and_then(|(w, h)| {
        let (w, h): (f64, f64) = (w.trim().parse().ok()?, h.trim().parse().ok()?);
        (w > 0.0 && h > 0.0).then_some(w / h)
    });
    let Some(ratio) = parsed else {
        return (even(video.0 * crop.width), even(video.1 * crop.height));
    };
    let long = video.0.max(video.1);
    if ratio >= 1.0 {
        (even(long), even(long / ratio))
    } else {
        (even(long * ratio), even(long))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FULL: Crop = Crop { x: 0.0, y: 0.0, width: 1.0, height: 1.0 };

    #[test]
    fn default_padding_takes_four_percent_per_side() {
        // §31: padFrac = pct / 100 × 0.2, so the default of 20 gives 4% per side.
        let layout = compute((1920.0, 1080.0), &Padding::uniform(20.0), &FULL, (1920.0, 1080.0));
        assert!((layout.frame.width - 1920.0 * 0.92).abs() < 1e-6);
        assert!((layout.frame.x - 1920.0 * 0.04).abs() < 1e-6);
        assert!((layout.frame.y - 1080.0 * 0.04).abs() < 1e-6);
    }

    #[test]
    fn no_padding_fills_the_stage() {
        let layout = compute((1920.0, 1080.0), &Padding::uniform(0.0), &FULL, (1920.0, 1080.0));
        assert_eq!(layout.frame, Rect { x: 0.0, y: 0.0, width: 1920.0, height: 1080.0 });
    }

    #[test]
    fn a_video_fits_without_changing_shape() {
        let layout = compute((1920.0, 1080.0), &Padding::uniform(0.0), &FULL, (1000.0, 1000.0));
        assert_eq!(layout.frame.height, 1080.0);
        assert_eq!(layout.frame.width, 1080.0);
        assert_eq!(layout.frame.center(), (960.0, 540.0));
    }

    #[test]
    fn a_crop_shows_only_part_of_the_video_at_full_scale() {
        let crop = Crop { x: 0.25, y: 0.25, width: 0.5, height: 0.5 };
        let layout = compute((1920.0, 1080.0), &Padding::uniform(0.0), &crop, (1920.0, 1080.0));
        assert_eq!(layout.frame.width, 1920.0);
        assert_eq!(layout.video.width, 3840.0);
        assert_eq!(layout.video.x, -960.0);
        assert_eq!(layout.video.y, -540.0);
    }

    #[test]
    fn unlinked_vertical_padding_slides_the_frame() {
        let padding = |top, bottom| Padding { top, bottom, left: 20.0, right: 20.0, linked: false };
        let stage = (1920.0, 1080.0);
        let centered = compute(stage, &padding(0.0, 0.0), &FULL, (1920.0, 1080.0));
        let low = compute(stage, &padding(250.0, 0.0), &FULL, (1920.0, 1080.0));
        let high = compute(stage, &padding(0.0, 250.0), &FULL, (1920.0, 1080.0));
        assert!(low.frame.y > centered.frame.y);
        assert!(high.frame.y < centered.frame.y);
        assert!(high.frame.y >= -1e-9);
        assert!(low.frame.y + low.frame.height <= 1080.0 + 1e-9);
    }

    #[test]
    fn the_corner_radius_scales_with_the_stage() {
        assert_eq!(corner_radius_px(12.5, (1920.0, 1080.0)), 12.5);
        assert_eq!(corner_radius_px(12.5, (3840.0, 2160.0)), 25.0);
        assert_eq!(corner_radius_px(12.5, (960.0, 1080.0)), 6.25);
    }

    #[test]
    fn the_shadow_matches_the_documented_profile() {
        let layers = shadow_layers(1.0);
        assert_eq!(layers[0], ShadowLayer { offset_y: 12.0, blur: 48.0, alpha: 0.7 });
        assert_eq!(layers[2], ShadowLayer { offset_y: 2.0, blur: 8.0, alpha: 0.3 });
        assert_eq!(shadow_layers(0.0)[0].alpha, 0.0);
    }

    #[test]
    fn output_sizes_are_even_and_fit_the_aspect() {
        assert_eq!(stage_size("16:9", (1920.0, 1080.0), &FULL), (1920, 1080));
        assert_eq!(stage_size("9:16", (1920.0, 1080.0), &FULL), (1080, 1920));
        assert_eq!(stage_size("1:1", (1921.0, 1080.0), &FULL), (1920, 1920));
        assert_eq!(stage_size("native", (1921.0, 1081.0), &FULL), (1920, 1080));
    }
}
