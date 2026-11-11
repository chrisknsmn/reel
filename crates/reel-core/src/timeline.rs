//! What's on the timeline: clips, zooms, annotations, added audio and captions.

use serde::{Deserialize, Serialize};

use crate::project::{MediaRef, TrackSetting};
use crate::time::{Micros, SourceTime, TimelineTime};

/// The shortest any timeline item can be.
pub const MIN_ITEM: Micros = Micros(100_000);

/// A stable identifier, unique within a project.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ItemId(pub String);

impl From<&str> for ItemId {
    fn from(value: &str) -> Self {
        Self(value.to_string())
    }
}

/// The speeds a clip can have. There's deliberately no custom value.
pub const CLIP_SPEEDS: [f64; 16] = [
    0.25, 0.5, 0.75, 1.0, 1.25, 1.5, 2.0, 2.5, 3.0, 4.0, 5.0, 8.0, 10.0, 15.0, 20.0, 30.0,
];

/// Part of the recording to keep.
///
/// A clip's place in the recording is independent of its place in the timeline.
/// That's what lets clips really be reordered: moving one doesn't change what footage it shows.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Clip {
    pub id: ItemId,
    /// Where it starts in the timeline.
    pub start: TimelineTime,
    /// How long it lasts on the timeline. This is stored, and the source end derived, so that
    /// clips that touch on the timeline are exact at any speed, and a split lines up at speeds
    /// other than 1× too.
    pub duration: Micros,
    /// Where in the recording it starts.
    pub source_start: SourceTime,
    /// How fast it plays. 2.0 means it takes half as long on the timeline.
    pub speed: f64,
    #[serde(default)]
    pub muted: bool,
    /// Shows read-only waveform rows under the clip.
    #[serde(default)]
    pub show_source_audio: bool,
    /// This clip's own volume per source track, where it differs from the project's. Export honours
    /// these per clip.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub track_settings: Vec<TrackSetting>,
}

impl Clip {
    #[must_use]
    pub fn end(&self) -> TimelineTime {
        self.start + self.duration
    }

    /// Where in the recording it stops.
    #[must_use]
    pub fn source_end(&self) -> SourceTime {
        self.source_start + self.duration.scaled(self.speed)
    }

    #[must_use]
    pub fn contains(&self, time: TimelineTime) -> bool {
        time >= self.start && time < self.end()
    }

    /// What's showing at a timeline position, if this clip covers it.
    #[must_use]
    pub fn to_source(&self, time: TimelineTime) -> Option<SourceTime> {
        self.contains(time).then(|| self.source_at(time))
    }

    /// Maps a timeline position through this clip, without checking it's inside.
    #[must_use]
    pub fn source_at(&self, time: TimelineTime) -> SourceTime {
        self.source_start + (time - self.start).scaled(self.speed)
    }

    /// Where a position in the recording appears on the timeline, if this clip shows it.
    #[must_use]
    pub fn to_timeline(&self, source: SourceTime) -> Option<TimelineTime> {
        (source >= self.source_start && source < self.source_end())
            .then(|| self.start + (source - self.source_start).scaled(1.0 / self.speed))
    }
}

/// The six zoom strengths.
pub const ZOOM_SCALES: [f64; 6] = [1.25, 1.5, 1.8, 2.2, 3.5, 5.0];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ZoomMode {
    /// The camera follows the cursor.
    #[default]
    Auto,
    /// The camera stays on a fixed focus.
    Manual,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

impl Point {
    pub const CENTER: Self = Self { x: 0.5, y: 0.5 };
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Zoom {
    pub id: ItemId,
    pub start: TimelineTime,
    pub end: TimelineTime,
    /// An index into `ZOOM_SCALES`.
    pub depth: u8,
    /// Where the camera looks, from 0 to 1 across the frame.
    pub focus: Point,
    #[serde(default)]
    pub mode: ZoomMode,
}

impl Zoom {
    #[must_use]
    pub fn scale(&self) -> f64 {
        ZOOM_SCALES[usize::from(self.depth).min(ZOOM_SCALES.len() - 1)]
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArrowDirection {
    Up,
    Down,
    Left,
    #[default]
    Right,
    UpRight,
    UpLeft,
    DownRight,
    DownLeft,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextAlign {
    Left,
    #[default]
    Center,
    Right,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TextStyle {
    pub color: String,
    pub background: String,
    pub font_size: f64,
    pub font_family: String,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub align: TextAlign,
    pub corner_radius: f64,
}

impl Default for TextStyle {
    fn default() -> Self {
        Self {
            color: "#FFFFFF".to_string(),
            background: "transparent".to_string(),
            font_size: 32.0,
            font_family: "system-ui".to_string(),
            bold: true,
            italic: false,
            underline: false,
            align: TextAlign::Center,
            corner_radius: 8.0,
        }
    }
}

/// What an annotation shows.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AnnotationContent {
    Text {
        text: String,
        #[serde(default)]
        style: TextStyle,
    },
    /// Embedded in the project as a data URL.
    Image { data_url: String },
    Arrow {
        direction: ArrowDirection,
        color: String,
        stroke_width: f64,
    },
    Blur {
        strength: f64,
        /// A solid fill for censoring, if any.
        #[serde(default)]
        fill: Option<String>,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Annotation {
    pub id: ItemId,
    pub start: TimelineTime,
    pub end: TimelineTime,
    pub content: AnnotationContent,
    /// Percentages of the recording area.
    pub position: Point,
    pub width: f64,
    pub height: f64,
    /// Which annotation row it's on.
    #[serde(default)]
    pub track: u32,
    /// Stacking order, later on top.
    #[serde(default)]
    pub layer: i32,
}

/// An added sound. It always plays from the start of its file.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AudioItem {
    pub id: ItemId,
    pub start: TimelineTime,
    pub end: TimelineTime,
    pub file: MediaRef,
    /// How long the file is, which limits how long the item can be.
    #[serde(default)]
    pub file_duration: Micros,
    pub volume: f64,
    /// A ×1.35 boost, not loudness normalisation (BUILD_PLAN 7.6).
    #[serde(default)]
    pub boost: bool,
    #[serde(default)]
    pub track: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CaptionWord {
    pub text: String,
    pub start: TimelineTime,
    pub end: TimelineTime,
    #[serde(default)]
    pub leading_space: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Caption {
    pub id: ItemId,
    pub start: TimelineTime,
    pub end: TimelineTime,
    pub text: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub words: Vec<CaptionWord>,
}

/// Everything on the timeline, always kept sorted and within the rules in `edit`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Timeline {
    pub clips: Vec<Clip>,
    pub zooms: Vec<Zoom>,
    pub annotations: Vec<Annotation>,
    pub audio: Vec<AudioItem>,
    pub captions: Vec<Caption>,
    /// How long the recording is, which limits how far a clip can extend.
    pub source_duration: Micros,
}

impl Timeline {
    /// A new timeline with one clip covering the whole recording, as a new recording gets.
    #[must_use]
    pub fn for_recording(source_duration: Micros) -> Self {
        Self {
            clips: vec![Clip {
                id: "clip-1".into(),
                start: TimelineTime::ZERO,
                duration: source_duration,
                source_start: SourceTime::ZERO,
                speed: 1.0,
                muted: false,
                show_source_audio: false,
                track_settings: Vec::new(),
            }],
            source_duration,
            ..Self::default()
        }
    }

    /// The end of the last clip. Gaps are allowed, and playback skips them.
    #[must_use]
    pub fn duration(&self) -> Micros {
        let end = self.clips.iter().map(Clip::end).max().unwrap_or_default();
        end - TimelineTime::ZERO
    }

    #[must_use]
    pub fn clip_at(&self, time: TimelineTime) -> Option<&Clip> {
        self.clips.iter().find(|clip| clip.contains(time))
    }

    /// What's showing at a timeline position, or nothing in a gap.
    #[must_use]
    pub fn to_source(&self, time: TimelineTime) -> Option<SourceTime> {
        self.clip_at(time).and_then(|clip| clip.to_source(time))
    }

    /// Where a position in the recording appears on the timeline. A clip can show footage that
    /// another clip also shows, in which case this gives the earliest.
    #[must_use]
    pub fn to_timeline(&self, source: SourceTime) -> Option<TimelineTime> {
        self.clips.iter().find_map(|clip| clip.to_timeline(source))
    }

    /// The next time after this one that any clip is showing, for skipping gaps during playback.
    #[must_use]
    pub fn next_playable(&self, time: TimelineTime) -> Option<TimelineTime> {
        self.clips
            .iter()
            .find(|clip| clip.end() > time)
            .map(|clip| clip.start.max(time))
    }

    /// An ID nothing on this timeline uses yet.
    #[must_use]
    pub fn new_id(&self, prefix: &str) -> ItemId {
        let used = |id: &str| {
            self.clips.iter().any(|item| item.id.0 == id)
                || self.zooms.iter().any(|item| item.id.0 == id)
                || self.annotations.iter().any(|item| item.id.0 == id)
                || self.audio.iter().any(|item| item.id.0 == id)
                || self.captions.iter().any(|item| item.id.0 == id)
        };
        (1..)
            .map(|n| format!("{prefix}-{n}"))
            .find(|id| !used(id))
            .map(ItemId)
            .expect("an unused ID always exists")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clip(start: f64, source_start: f64, source_end: f64, speed: f64) -> Clip {
        Clip {
            id: "c".into(),
            start: TimelineTime::from_ms(start),
            duration: Micros::from_ms((source_end - source_start) / speed),
            source_start: SourceTime::from_ms(source_start),
            speed,
            muted: false,
            show_source_audio: false,
            track_settings: Vec::new(),
        }
    }

    #[test]
    fn a_faster_clip_takes_less_time_and_shows_the_same_footage() {
        let c = clip(1000.0, 5000.0, 7000.0, 2.0);
        assert_eq!(c.duration, Micros::from_ms(1000.0));
        assert_eq!(c.source_end(), SourceTime::from_ms(7000.0));
        assert_eq!(c.end(), TimelineTime::from_ms(2000.0));
        assert_eq!(
            c.to_source(TimelineTime::from_ms(1500.0)),
            Some(SourceTime::from_ms(6000.0))
        );
    }

    #[test]
    fn source_and_timeline_positions_are_independent() {
        // This clip shows footage from 10 s in, but sits at the very start of the timeline.
        let c = clip(0.0, 10_000.0, 12_000.0, 1.0);
        assert_eq!(
            c.to_source(TimelineTime::ZERO),
            Some(SourceTime::from_ms(10_000.0))
        );
        assert_eq!(
            c.to_timeline(SourceTime::from_ms(11_000.0)),
            Some(TimelineTime::from_ms(1000.0))
        );
    }

    #[test]
    fn a_gap_shows_nothing_and_playback_skips_it() {
        let timeline = Timeline {
            clips: vec![
                clip(0.0, 0.0, 1000.0, 1.0),
                clip(2000.0, 1000.0, 2000.0, 1.0),
            ],
            ..Timeline::default()
        };
        let in_gap = TimelineTime::from_ms(1500.0);
        assert_eq!(timeline.to_source(in_gap), None);
        assert_eq!(
            timeline.next_playable(in_gap),
            Some(TimelineTime::from_ms(2000.0))
        );
    }
}
