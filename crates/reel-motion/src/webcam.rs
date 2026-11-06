//! Where the webcam bubble goes (§35).

use reel_core::project::{Crop, Webcam, WebcamPosition};
use reel_core::time::{SourceTime, TimelineTime};

use crate::layout::Rect;

const SMALLEST: f64 = 56.0;

/// Where the bubble lands on a stage.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BubbleLayout {
    /// The bubble itself.
    pub frame: Rect,
    /// Where the whole webcam frame would go, so that it covers the bubble after cropping.
    pub video: Rect,
    pub radius: f64,
    /// Y offset, blur and alpha, for a shadow like the frame's.
    pub shadow: (f64, f64, f64),
}

fn preset_fractions(webcam: &Webcam) -> (f64, f64) {
    match webcam.position {
        WebcamPosition::TopLeft => (0.0, 0.0),
        WebcamPosition::TopCenter => (0.5, 0.0),
        WebcamPosition::TopRight => (1.0, 0.0),
        WebcamPosition::CenterLeft => (0.0, 0.5),
        WebcamPosition::Center => (0.5, 0.5),
        WebcamPosition::CenterRight => (1.0, 0.5),
        WebcamPosition::BottomLeft => (0.0, 1.0),
        WebcamPosition::BottomCenter => (0.5, 1.0),
        WebcamPosition::BottomRight => (1.0, 1.0),
        WebcamPosition::Custom => (
            webcam.custom_x.clamp(0.0, 1.0),
            webcam.custom_y.clamp(0.0, 1.0),
        ),
    }
}

/// Works out the bubble's place. `zoom` is the camera's current scale: a bubble set to react to zoom
/// shrinks as the camera moves in, to stay out of the way. `scale` is pixels per CSS pixel, so that a
/// look comes out the same at any output size.
#[must_use]
pub fn layout(
    webcam: &Webcam,
    stage: (f64, f64),
    webcam_size: (f64, f64),
    zoom: f64,
    scale: f64,
) -> BubbleLayout {
    let dimension = stage.0.min(stage.1);
    let margin = webcam.margin * scale;
    let react = if webcam.react_to_zoom {
        1.0 / zoom.max(0.01)
    } else {
        1.0
    };
    let largest = (dimension - 2.0 * margin).max(SMALLEST * scale);
    let size =
        |percent: f64| (percent / 100.0 * dimension * react).clamp(SMALLEST * scale, largest);
    let (mut width, height) = (size(webcam.width), size(webcam.height));

    // A bubble set roughly square follows the crop's shape instead of squashing it.
    let crop = webcam.crop;
    let is_default_crop = crop == Crop::default();
    let cropped_w = webcam_size.0 * crop.width;
    let cropped_h = webcam_size.1 * crop.height;
    if !is_default_crop && (webcam.width - webcam.height).abs() < 1.0 && cropped_h > 0.0 {
        width = (height * cropped_w / cropped_h).clamp(SMALLEST * scale, largest);
    }

    let (px, py) = preset_fractions(webcam);
    let frame = Rect {
        x: margin + (stage.0 - width - 2.0 * margin) * px,
        y: margin + (stage.1 - height - 2.0 * margin) * py,
        width,
        height,
    };

    // Cover fit: the cropped footage fills the bubble, overflowing on the long side.
    let cover = (width / cropped_w.max(1.0)).max(height / cropped_h.max(1.0));
    let (full_w, full_h) = (webcam_size.0 * cover, webcam_size.1 * cover);
    let center_x = frame.x + width / 2.0;
    let center_y = frame.y + height / 2.0;
    let crop_center_x = (crop.x + crop.width / 2.0) * full_w;
    let crop_center_y = (crop.y + crop.height / 2.0) * full_h;
    let mut video = Rect {
        x: center_x - crop_center_x,
        y: center_y - crop_center_y,
        width: full_w,
        height: full_h,
    };
    if webcam.mirror {
        // A negative width flips the picture about its own right edge, so mirror about the centre.
        video.x = center_x + crop_center_x;
        video.width = -full_w;
    }

    let smaller = width.min(height);
    BubbleLayout {
        frame,
        video,
        radius: (webcam.corner_radius * scale).min(smaller / 2.0),
        shadow: (
            (0.06 * smaller).round(),
            (0.22 * smaller).round(),
            webcam.shadow,
        ),
    }
}

/// Where the webcam's footage should be for a moment in the recording. Its offset says how far ahead
/// it is. Each clip's own mapping means it stays in step after cuts and speed changes.
#[must_use]
pub fn webcam_time(webcam: &Webcam, source: SourceTime) -> SourceTime {
    SourceTime((source.0 - webcam.offset.0).max(0))
}

/// Whether a time is one the bubble should show at all.
#[must_use]
pub fn shows_at(webcam: &Webcam, _time: TimelineTime) -> bool {
    webcam.enabled && webcam.source.is_some()
}

#[cfg(test)]
mod tests {
    use super::*;
    use reel_core::project::MediaRef;
    use reel_core::time::Micros;

    const STAGE: (f64, f64) = (1920.0, 1080.0);
    const CAMERA: (f64, f64) = (1280.0, 720.0);

    fn webcam() -> Webcam {
        Webcam {
            enabled: true,
            source: Some(MediaRef::absolute("/c.mp4")),
            mirror: false,
            react_to_zoom: false,
            ..Webcam::default()
        }
    }

    #[test]
    fn the_size_is_a_percentage_of_the_smaller_side() {
        let bubble = layout(&webcam(), STAGE, CAMERA, 1.0, 1.0);
        assert_eq!(bubble.frame.width, 432.0);
        assert_eq!(bubble.frame.height, 432.0);
    }

    #[test]
    fn the_default_position_is_the_bottom_right_inside_the_margin() {
        let bubble = layout(&webcam(), STAGE, CAMERA, 1.0, 1.0);
        assert_eq!(bubble.frame.x, 1920.0 - 432.0 - 24.0);
        assert_eq!(bubble.frame.y, 1080.0 - 432.0 - 24.0);
    }

    #[test]
    fn every_preset_lands_where_it_says() {
        let at = |position| {
            layout(
                &Webcam {
                    position,
                    ..webcam()
                },
                STAGE,
                CAMERA,
                1.0,
                1.0,
            )
            .frame
        };
        assert_eq!(
            (at(WebcamPosition::TopLeft).x, at(WebcamPosition::TopLeft).y),
            (24.0, 24.0)
        );
        let center = at(WebcamPosition::Center);
        assert_eq!(center.center(), (960.0, 540.0));
    }

    #[test]
    fn a_custom_position_uses_its_fractions() {
        let bubble = layout(
            &Webcam {
                position: WebcamPosition::Custom,
                custom_x: 0.0,
                custom_y: 1.0,
                ..webcam()
            },
            STAGE,
            CAMERA,
            1.0,
            1.0,
        );
        assert_eq!(bubble.frame.x, 24.0);
    }

    #[test]
    fn reacting_to_zoom_shrinks_it_as_the_camera_moves_in() {
        let reacting = Webcam {
            react_to_zoom: true,
            ..webcam()
        };
        let wide = layout(&reacting, STAGE, CAMERA, 1.0, 1.0).frame.width;
        let zoomed = layout(&reacting, STAGE, CAMERA, 2.0, 1.0).frame.width;
        assert_eq!(zoomed, wide / 2.0);
        let not_reacting = layout(&webcam(), STAGE, CAMERA, 2.0, 1.0).frame.width;
        assert_eq!(not_reacting, wide);
    }

    #[test]
    fn it_never_gets_smaller_than_the_minimum() {
        let bubble = layout(
            &Webcam {
                width: 1.0,
                height: 1.0,
                ..webcam()
            },
            STAGE,
            CAMERA,
            1.0,
            1.0,
        );
        assert_eq!(bubble.frame.width, 56.0);
    }

    #[test]
    fn footage_covers_the_bubble_without_changing_shape() {
        let bubble = layout(&webcam(), STAGE, CAMERA, 1.0, 1.0);
        assert_eq!(bubble.video.height, 432.0);
        assert_eq!(bubble.video.width, 768.0);
        assert_eq!(
            bubble.video.x + bubble.video.width / 2.0,
            bubble.frame.x + 216.0,
            "centred on the bubble"
        );
    }

    #[test]
    fn mirroring_flips_the_picture_about_the_bubbles_centre() {
        let bubble = layout(
            &Webcam {
                mirror: true,
                ..webcam()
            },
            STAGE,
            CAMERA,
            1.0,
            1.0,
        );
        assert_eq!(bubble.video.width, -768.0);
        assert_eq!(
            bubble.video.x + bubble.video.width / 2.0,
            bubble.frame.x + 216.0
        );
    }

    #[test]
    fn a_crop_shows_that_part_of_the_footage_in_the_bubble() {
        let crop = Crop {
            x: 0.5,
            y: 0.0,
            width: 0.5,
            height: 1.0,
        };
        let bubble = layout(&Webcam { crop, ..webcam() }, STAGE, CAMERA, 1.0, 1.0);
        // The bubble follows the crop's shape, and the right half of the footage fills it.
        assert_eq!(bubble.frame.height, 432.0);
        assert!((bubble.frame.width - 384.0).abs() < 1e-6);
        let left_edge_of_crop = bubble.video.x + 0.5 * bubble.video.width;
        assert!((left_edge_of_crop - bubble.frame.x).abs() < 1e-6);
    }

    #[test]
    fn the_shadow_follows_the_documented_formula() {
        let bubble = layout(
            &Webcam {
                shadow: 0.67,
                ..webcam()
            },
            STAGE,
            CAMERA,
            1.0,
            1.0,
        );
        assert_eq!(
            bubble.shadow,
            ((0.06_f64 * 432.0).round(), (0.22_f64 * 432.0).round(), 0.67)
        );
    }

    #[test]
    fn the_radius_cant_exceed_a_circle() {
        let bubble = layout(
            &Webcam {
                corner_radius: 9999.0,
                ..webcam()
            },
            STAGE,
            CAMERA,
            1.0,
            1.0,
        );
        assert_eq!(bubble.radius, 216.0);
    }

    #[test]
    fn everything_scales_with_the_output_size() {
        let small = layout(&webcam(), (960.0, 540.0), CAMERA, 1.0, 0.5);
        let large = layout(&webcam(), STAGE, CAMERA, 1.0, 1.0);
        assert_eq!(small.frame.x * 2.0, large.frame.x);
        assert_eq!(small.radius * 2.0, large.radius);
    }

    #[test]
    fn the_offset_moves_the_webcams_clock() {
        let offset = Webcam {
            offset: Micros::from_ms(250.0),
            ..webcam()
        };
        assert_eq!(
            webcam_time(&offset, SourceTime::from_ms(1000.0)),
            SourceTime::from_ms(750.0)
        );
        assert_eq!(
            webcam_time(&offset, SourceTime::from_ms(100.0)),
            SourceTime::ZERO
        );
    }
}
