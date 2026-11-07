//! The arithmetic behind the timeline, kept apart from the UI so it can be tested on its own.

use reel_core::time::{Micros, TimelineTime};

/// The shortest stretch the view can zoom in to (§14).
pub const SMALLEST_VIEW: Micros = Micros(300_000);
/// How close a dragged playhead has to be to a keyframe to land on it (§14).
pub const KEYFRAME_SNAP: Micros = Micros(150_000);
/// How far the skip buttons go when there's no keyframe to go to (§17).
pub const SKIP: Micros = Micros(5_000_000);
const LABELS_WANTED: f64 = 12.0;

/// The stretch of the timeline that's showing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct View {
    pub start: TimelineTime,
    pub length: Micros,
}

impl View {
    /// The whole of a timeline.
    #[must_use]
    pub fn whole(total: Micros) -> Self {
        Self {
            start: TimelineTime::ZERO,
            length: total.max(SMALLEST_VIEW),
        }
    }

    /// Where a time falls, from 0 to `width`.
    #[must_use]
    pub fn x_of(&self, time: TimelineTime, width: f64) -> f64 {
        (time - self.start).0 as f64 / self.length.0.max(1) as f64 * width
    }

    /// How wide a stretch of time is.
    #[must_use]
    pub fn width_of(&self, length: Micros, width: f64) -> f64 {
        length.0 as f64 / self.length.0.max(1) as f64 * width
    }

    /// The time at a place along the view.
    #[must_use]
    pub fn time_at(&self, x: f64, width: f64) -> TimelineTime {
        self.start + Micros((x / width.max(1.0) * self.length.0 as f64) as i64)
    }

    fn clamped(self, total: Micros) -> Self {
        let length = self.length.clamp(SMALLEST_VIEW, total.max(SMALLEST_VIEW));
        let latest = (total.max(length) - length).0;
        Self {
            start: TimelineTime(self.start.0.clamp(0, latest.max(0))),
            length,
        }
    }

    /// Zooms toward a place, which stays where it is on screen (§14).
    #[must_use]
    pub fn zoomed(self, factor: f64, x: f64, width: f64, total: Micros) -> Self {
        let anchor = self.time_at(x, width);
        let length = Micros((self.length.0 as f64 * factor) as i64)
            .clamp(SMALLEST_VIEW, total.max(SMALLEST_VIEW));
        let fraction = x / width.max(1.0);
        let start = anchor.0 - (length.0 as f64 * fraction) as i64;
        Self {
            start: TimelineTime(start),
            length,
        }
        .clamped(total)
    }

    /// Slides along by a number of pixels.
    #[must_use]
    pub fn panned(self, pixels: f64, width: f64, total: Micros) -> Self {
        let by = (pixels / width.max(1.0) * self.length.0 as f64) as i64;
        Self {
            start: TimelineTime(self.start.0 + by),
            length: self.length,
        }
        .clamped(total)
    }

    /// Scrolls just enough to bring a time into view, as a dragged playhead does at the edges.
    #[must_use]
    pub fn following(self, time: TimelineTime, total: Micros) -> Self {
        let end = self.start + self.length;
        if time < self.start {
            Self {
                start: time,
                ..self
            }
            .clamped(total)
        } else if time > end {
            Self {
                start: time - self.length,
                ..self
            }
            .clamped(total)
        } else {
            self
        }
    }
}

/// A mark on the ruler.
#[derive(Clone, Debug, PartialEq)]
pub struct Tick {
    pub time: TimelineTime,
    pub label: Option<String>,
}

/// A round step that gives about 12 labels across the view.
#[must_use]
pub fn ruler_step(length: Micros) -> Micros {
    const STEPS_MS: [f64; 16] = [
        50.0,
        100.0,
        200.0,
        500.0,
        1000.0,
        2000.0,
        5000.0,
        10_000.0,
        15_000.0,
        30_000.0,
        60_000.0,
        120_000.0,
        300_000.0,
        600_000.0,
        1_800_000.0,
        3_600_000.0,
    ];
    let wanted = length.ms() / LABELS_WANTED;
    let step = STEPS_MS
        .iter()
        .copied()
        .find(|step| *step >= wanted)
        .unwrap_or(STEPS_MS[STEPS_MS.len() - 1]);
    Micros::from_ms(step)
}

/// A time as a ruler shows it, with only as much detail as its step needs.
#[must_use]
pub fn ruler_label(time: TimelineTime, step: Micros) -> String {
    let millis = (time.0 / 1000).max(0);
    let (minutes, seconds, rest) = (millis / 60_000, millis / 1000 % 60, millis % 1000);
    if step.0 < 1_000_000 {
        format!("{minutes}:{seconds:02}.{:01}", rest / 100)
    } else {
        format!("{minutes}:{seconds:02}")
    }
}

/// The ruler's labels and the smaller marks between them.
#[must_use]
pub fn ruler_ticks(view: &View) -> Vec<Tick> {
    let step = ruler_step(view.length);
    let minor = Micros(step.0 / 5);
    let first = view.start.0 / minor.0;
    let last = (view.start.0 + view.length.0) / minor.0;
    (first..=last)
        .map(|index| {
            let time = TimelineTime(index * minor.0);
            let major = index % 5 == 0;
            Tick {
                time,
                label: major.then(|| ruler_label(time, step)),
            }
        })
        .collect()
}

/// Lands a dragged time on a keyframe if it's close enough.
#[must_use]
pub fn snap_to_keyframes(time: TimelineTime, keyframes: &[TimelineTime]) -> TimelineTime {
    keyframes
        .iter()
        .copied()
        .filter(|key| (*key - time).0.abs() <= KEYFRAME_SNAP.0)
        .min_by_key(|key| (*key - time).0.abs())
        .unwrap_or(time)
}

/// Where a skip button goes: the next keyframe that way, or else five seconds along (§17).
#[must_use]
pub fn skip_target(
    position: TimelineTime,
    keyframes: &[TimelineTime],
    forward: bool,
    total: Micros,
) -> TimelineTime {
    let found = if forward {
        keyframes
            .iter()
            .copied()
            .filter(|key| *key > position)
            .min()
    } else {
        keyframes
            .iter()
            .copied()
            .filter(|key| *key < position)
            .max()
    };
    let target = found.unwrap_or_else(|| {
        if forward {
            position + SKIP
        } else {
            position - SKIP
        }
    });
    TimelineTime(target.0.clamp(0, total.0.max(0)))
}

/// `m:ss.mmm`, which is how a caption's times are shown and typed (§12).
#[must_use]
pub fn exact_time(time: TimelineTime) -> String {
    let millis = (time.0 / 1000).max(0);
    format!(
        "{}:{:02}.{:03}",
        millis / 60_000,
        millis / 1000 % 60,
        millis % 1000
    )
}

/// Reads a time typed as `m:ss.mmm`, or just seconds.
#[must_use]
pub fn parse_time(text: &str) -> Option<TimelineTime> {
    let text = text.trim();
    let (minutes, seconds) = match text.split_once(':') {
        Some((minutes, seconds)) => (minutes.parse::<i64>().ok()?, seconds),
        None => (0, text),
    };
    let seconds: f64 = seconds.parse().ok()?;
    if minutes < 0 || seconds < 0.0 || !seconds.is_finite() {
        return None;
    }
    Some(TimelineTime(
        minutes * 60_000_000 + (seconds * 1_000_000.0).round() as i64,
    ))
}

/// `h:mm:ss` or `m:ss`, for the transport's readout.
#[must_use]
pub fn clock_text(time: Micros) -> String {
    let total = time.secs().max(0.0);
    let minutes = (total / 60.0) as u64;
    let seconds = total - minutes as f64 * 60.0;
    format!("{minutes}:{seconds:05.2}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(ms: f64) -> TimelineTime {
        TimelineTime::from_ms(ms)
    }

    fn secs(value: i64) -> Micros {
        Micros::from_secs(value)
    }

    #[test]
    fn the_whole_view_maps_ends_to_ends() {
        let view = View::whole(secs(10));
        assert_eq!(view.x_of(at(0.0), 1000.0), 0.0);
        assert_eq!(view.x_of(at(10_000.0), 1000.0), 1000.0);
        assert_eq!(view.time_at(500.0, 1000.0), at(5000.0));
        assert_eq!(view.width_of(secs(1), 1000.0), 100.0);
    }

    #[test]
    fn a_very_short_timeline_still_has_a_usable_view() {
        assert_eq!(View::whole(Micros(10)).length, SMALLEST_VIEW);
    }

    #[test]
    fn zooming_keeps_the_pointer_over_the_same_moment() {
        let view = View::whole(secs(100));
        let (x, width) = (250.0, 1000.0);
        let before = view.time_at(x, width);
        let zoomed = view.zoomed(0.5, x, width, secs(100));
        assert_eq!(zoomed.length, secs(50));
        assert!(
            (zoomed.time_at(x, width).0 - before.0).abs() < 2,
            "{zoomed:?}"
        );
    }

    #[test]
    fn zooming_stops_at_the_limits() {
        let total = secs(100);
        let mut view = View::whole(total);
        for _ in 0..50 {
            view = view.zoomed(0.5, 500.0, 1000.0, total);
        }
        assert_eq!(view.length, SMALLEST_VIEW);
        for _ in 0..50 {
            view = view.zoomed(2.0, 500.0, 1000.0, total);
        }
        assert_eq!(view, View::whole(total));
    }

    #[test]
    fn panning_stops_at_the_ends() {
        let total = secs(100);
        let view = View {
            start: at(10_000.0),
            length: secs(20),
        };
        assert_eq!(view.panned(500.0, 1000.0, total).start, at(20_000.0));
        assert_eq!(view.panned(-5000.0, 1000.0, total).start, at(0.0));
        assert_eq!(view.panned(100_000.0, 1000.0, total).start, at(80_000.0));
    }

    #[test]
    fn following_only_scrolls_when_it_has_to() {
        let total = secs(100);
        let view = View {
            start: at(10_000.0),
            length: secs(20),
        };
        assert_eq!(view.following(at(15_000.0), total), view);
        assert_eq!(view.following(at(5000.0), total).start, at(5000.0));
        assert_eq!(view.following(at(40_000.0), total).start, at(20_000.0));
    }

    #[test]
    fn the_ruler_has_about_a_dozen_labels_at_any_zoom() {
        for seconds in [1, 7, 30, 60, 90, 300, 1000, 3600] {
            let view = View::whole(secs(seconds));
            let labels = ruler_ticks(&view)
                .iter()
                .filter(|tick| tick.label.is_some())
                .count();
            assert!(
                (5..=14).contains(&labels),
                "{labels} labels for {seconds} s"
            );
        }
    }

    #[test]
    fn ruler_labels_show_as_much_detail_as_the_step_needs() {
        assert_eq!(ruler_label(at(90_000.0), secs(10)), "1:30");
        assert_eq!(ruler_label(at(1500.0), Micros::from_ms(100.0)), "0:01.5");
    }

    #[test]
    fn the_playhead_snaps_to_a_close_keyframe_only() {
        let keys = [at(1000.0), at(5000.0)];
        assert_eq!(snap_to_keyframes(at(1100.0), &keys), at(1000.0));
        assert_eq!(snap_to_keyframes(at(4900.0), &keys), at(5000.0));
        assert_eq!(snap_to_keyframes(at(3000.0), &keys), at(3000.0));
        assert_eq!(snap_to_keyframes(at(1200.0), &keys), at(1200.0));
    }

    #[test]
    fn skipping_prefers_keyframes_then_goes_five_seconds() {
        let keys = [at(2000.0), at(8000.0)];
        let total = secs(60);
        assert_eq!(skip_target(at(3000.0), &keys, true, total), at(8000.0));
        assert_eq!(skip_target(at(3000.0), &keys, false, total), at(2000.0));
        assert_eq!(skip_target(at(9000.0), &keys, true, total), at(14_000.0));
        assert_eq!(skip_target(at(1000.0), &keys, false, total), at(0.0));
        assert_eq!(skip_target(at(58_000.0), &[], true, total), at(60_000.0));
    }

    #[test]
    fn exact_times_read_back_as_they_were() {
        assert_eq!(exact_time(at(83_250.0)), "1:23.250");
        assert_eq!(parse_time("1:23.250"), Some(at(83_250.0)));
        assert_eq!(parse_time("5.5"), Some(at(5500.0)));
        assert_eq!(parse_time(" 0:01 "), Some(at(1000.0)));
    }

    #[test]
    fn nonsense_times_are_refused() {
        for text in ["", "abc", "1:xx", "-1:00", "1:-5", "inf"] {
            assert_eq!(parse_time(text), None, "{text}");
        }
    }

    #[test]
    fn the_readout_is_minutes_and_seconds() {
        assert_eq!(clock_text(secs(0)), "0:00.00");
        assert_eq!(clock_text(Micros::from_ms(83_250.0)), "1:23.25");
    }
}
