//! Recorded cursor data (§28), which lives on the recording's clock.

use reel_core::time::SourceTime;
use serde::Deserialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum Interaction {
    #[default]
    Move,
    Click,
    DoubleClick,
    RightClick,
    MiddleClick,
    Mouseup,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum CursorShape {
    #[default]
    Arrow,
    Text,
    Pointer,
    Crosshair,
    OpenHand,
    ClosedHand,
    ResizeEw,
    ResizeNs,
    NotAllowed,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CursorSample {
    pub time: SourceTime,
    /// From 0 to 1 across the full, uncropped recording.
    pub x: f64,
    pub y: f64,
    pub interaction: Interaction,
    pub shape: CursorShape,
}

#[derive(Deserialize)]
struct FileSample {
    #[serde(rename = "timeMs")]
    time_ms: f64,
    cx: f64,
    cy: f64,
    #[serde(default, rename = "interactionType")]
    interaction: Interaction,
    #[serde(default, rename = "cursorType")]
    shape: CursorShape,
}

#[derive(Deserialize)]
struct File {
    samples: Vec<FileSample>,
}

/// All of a recording's cursor samples, in time order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CursorTrack {
    pub samples: Vec<CursorSample>,
}

impl CursorTrack {
    /// Reads a `.cursor.json` file.
    ///
    /// # Errors
    /// Returns the parse error if the file isn't valid.
    pub fn parse(text: &str) -> Result<Self, serde_json::Error> {
        let file: File = serde_json::from_str(text)?;
        let mut samples: Vec<CursorSample> = file
            .samples
            .into_iter()
            .filter(|sample| sample.time_ms.is_finite() && sample.cx.is_finite() && sample.cy.is_finite())
            .map(|sample| CursorSample {
                time: SourceTime::from_ms(sample.time_ms),
                x: sample.cx,
                y: sample.cy,
                interaction: sample.interaction,
                shape: sample.shape,
            })
            .collect();
        samples.sort_by_key(|sample| sample.time);
        Ok(Self { samples })
    }

    /// Where the cursor is at a time, between the samples either side of it.
    #[must_use]
    pub fn position_at(&self, time: SourceTime) -> Option<(f64, f64)> {
        let first = self.samples.first()?;
        let last = self.samples.last()?;
        if time <= first.time {
            return Some((first.x, first.y));
        }
        if time >= last.time {
            return Some((last.x, last.y));
        }
        let next = self.samples.partition_point(|sample| sample.time <= time);
        let (a, b) = (&self.samples[next - 1], &self.samples[next]);
        let span = (b.time - a.time).0 as f64;
        let t = if span > 0.0 { (time - a.time).0 as f64 / span } else { 0.0 };
        Some((a.x + (b.x - a.x) * t, a.y + (b.y - a.y) * t))
    }

    /// The shape showing at a time: the most recent sample's.
    #[must_use]
    pub fn shape_at(&self, time: SourceTime) -> CursorShape {
        let next = self.samples.partition_point(|sample| sample.time <= time);
        next.checked_sub(1).map_or(CursorShape::Arrow, |index| self.samples[index].shape)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILE: &str = r#"{ "version": 2, "samples": [
        { "timeMs": 100, "cx": 0.2, "cy": 0.2 },
        { "timeMs": 0, "cx": 0.0, "cy": 0.0, "cursorType": "text" },
        { "timeMs": 200, "cx": 0.4, "cy": 0.8, "interactionType": "click", "cursorType": "pointer" }
    ] }"#;

    #[test]
    fn samples_are_read_and_sorted() {
        let track = CursorTrack::parse(FILE).unwrap();
        assert_eq!(track.samples.len(), 3);
        assert_eq!(track.samples[0].time, SourceTime::ZERO);
        assert_eq!(track.samples[2].interaction, Interaction::Click);
        assert_eq!(track.samples[2].shape, CursorShape::Pointer);
    }

    #[test]
    fn positions_are_interpolated_and_clamped_at_the_ends() {
        let track = CursorTrack::parse(FILE).unwrap();
        let (x, y) = track.position_at(SourceTime::from_ms(150.0)).unwrap();
        assert!((x - 0.3).abs() < 1e-9);
        assert!((y - 0.5).abs() < 1e-9);
        assert_eq!(track.position_at(SourceTime::from_ms(-5.0)), Some((0.0, 0.0)));
        assert_eq!(track.position_at(SourceTime::from_ms(999.0)), Some((0.4, 0.8)));
    }

    #[test]
    fn an_empty_track_has_no_position() {
        assert_eq!(CursorTrack::default().position_at(SourceTime::ZERO), None);
    }

    #[test]
    fn the_shape_is_the_most_recent_samples() {
        let track = CursorTrack::parse(FILE).unwrap();
        assert_eq!(track.shape_at(SourceTime::from_ms(50.0)), CursorShape::Text);
        assert_eq!(track.shape_at(SourceTime::from_ms(250.0)), CursorShape::Pointer);
    }

    #[test]
    fn unknown_values_dont_stop_a_real_file_loading() {
        let track = CursorTrack::parse(r#"{ "samples": [] }"#).unwrap();
        assert!(track.samples.is_empty());
    }
}
