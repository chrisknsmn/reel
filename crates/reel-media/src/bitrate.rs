//! How many bits an export gets (§39).

use reel_core::project::{EncodingMode, ExportQuality};

const P720: f64 = 1280.0 * 720.0;
const P1080: f64 = 1920.0 * 1080.0;
const P2160: f64 = 3840.0 * 2160.0;

/// The bitrate for an MP4, in bits per second.
#[must_use]
pub fn video_bitrate(width: u32, height: u32, fps: u32, quality: ExportQuality, mode: EncodingMode) -> u32 {
    let pixels = f64::from(width) * f64::from(height);
    let original = quality == ExportQuality::Source;
    let (at_720, at_1080, at_4k) = if original { (8e6, 12e6, 45e6) } else { (5e6, 8e6, 35e6) };
    let base = if pixels <= P720 {
        at_720
    } else if pixels <= P1080 {
        at_1080
    } else if pixels >= P2160 {
        at_4k
    } else {
        at_1080 + (at_4k - at_1080) * (pixels - P1080) / (P2160 - P1080)
    };
    let multiplier = match mode {
        EncodingMode::Fast => 0.5,
        EncodingMode::Balanced => 0.8,
        EncodingMode::Quality => 1.0,
    };
    let scaled = base * (f64::from(fps) / 30.0).max(1.0).sqrt() * multiplier;
    scaled.round().max(2_000_000.0) as u32
}

/// How many frames apart keyframes are.
#[must_use]
pub fn keyframe_interval(fps: u32, mode: EncodingMode) -> u32 {
    let seconds = match mode {
        EncodingMode::Fast => 4.0,
        EncodingMode::Balanced => 3.0,
        EncodingMode::Quality => 2.5,
    };
    (f64::from(fps) * seconds).round() as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_documented_example_comes_out_right() {
        // §39: 1080p30 Original Balanced is 9.6 Mbps, and 60 fps multiplies that by 1.414.
        let at_30 = video_bitrate(1920, 1080, 30, ExportQuality::Source, EncodingMode::Balanced);
        assert_eq!(at_30, 9_600_000);
        let at_60 = video_bitrate(1920, 1080, 60, ExportQuality::Source, EncodingMode::Balanced);
        assert!((f64::from(at_60) / f64::from(at_30) - std::f64::consts::SQRT_2).abs() < 0.001);
    }

    #[test]
    fn sizes_between_1080p_and_4k_interpolate() {
        let quality = ExportQuality::Source;
        let mode = EncodingMode::Quality;
        let low = video_bitrate(1920, 1080, 30, quality, mode);
        let middle = video_bitrate(2560, 1440, 30, quality, mode);
        let high = video_bitrate(3840, 2160, 30, quality, mode);
        assert_eq!((low, high), (12_000_000, 45_000_000));
        assert!(middle > low && middle < high);
    }

    #[test]
    fn it_never_goes_below_the_floor() {
        assert_eq!(video_bitrate(320, 180, 15, ExportQuality::Medium, EncodingMode::Fast), 2_500_000);
        assert!(video_bitrate(16, 16, 1, ExportQuality::Medium, EncodingMode::Fast) >= 2_000_000);
    }

    #[test]
    fn low_frame_rates_get_no_discount() {
        let quality = ExportQuality::Source;
        let mode = EncodingMode::Balanced;
        assert_eq!(video_bitrate(1920, 1080, 15, quality, mode), video_bitrate(1920, 1080, 30, quality, mode));
    }

    #[test]
    fn keyframes_match_the_documented_spacing() {
        assert_eq!(keyframe_interval(30, EncodingMode::Fast), 120);
        assert_eq!(keyframe_interval(30, EncodingMode::Balanced), 90);
        assert_eq!(keyframe_interval(60, EncodingMode::Quality), 150);
    }
}
