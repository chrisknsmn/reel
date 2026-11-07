//! The recording's own timeline: time from the first kept frame, with paused time taken out.
//!
//! Sound and frames arrive a little after they happened, so one sample can arrive after a pause even
//! though it was captured before it. That's why this remembers every pause, and places each sample by
//! its own timestamp rather than by when it turned up.

use reel_core::time::Micros;

/// Maps host-clock times, in microseconds, onto the recording.
#[derive(Clone, Debug, Default)]
pub struct RecordingClock {
    /// The host time of the first kept frame.
    zero: Option<i64>,
    pauses: Vec<(i64, i64)>,
    paused_since: Option<i64>,
}

impl RecordingClock {
    #[must_use]
    pub fn new(start_paused: bool) -> Self {
        Self {
            paused_since: start_paused.then_some(i64::MIN),
            ..Self::default()
        }
    }

    #[must_use]
    pub fn is_paused(&self) -> bool {
        self.paused_since.is_some()
    }

    /// Whether a frame has been kept yet.
    #[must_use]
    pub fn has_started(&self) -> bool {
        self.zero.is_some()
    }

    pub fn pause(&mut self, now: i64) {
        if self.paused_since.is_none() {
            self.paused_since = Some(now);
        }
    }

    pub fn resume(&mut self, now: i64) {
        if let Some(since) = self.paused_since.take() {
            self.pauses.push((since, now));
        }
    }

    fn in_a_pause(&self, host: i64) -> bool {
        self.paused_since.is_some_and(|since| host >= since)
            || self
                .pauses
                .iter()
                .any(|(start, end)| host >= *start && host < *end)
    }

    /// Where a video frame goes. The first frame that isn't in a pause becomes time zero, which is what
    /// makes a warm start free of startup lag (§27).
    pub fn place_frame(&mut self, host: i64) -> Option<Micros> {
        if self.in_a_pause(host) {
            return None;
        }
        if self.zero.is_none() {
            self.zero = Some(host);
        }
        self.place(host)
    }

    /// Where anything else goes. Whatever came before the first frame or during a pause has no place.
    #[must_use]
    pub fn place(&self, host: i64) -> Option<Micros> {
        let zero = self.zero?;
        if host < zero || self.in_a_pause(host) {
            return None;
        }
        let paused: i64 = self
            .pauses
            .iter()
            .filter(|(start, end)| *start >= zero && *end <= host)
            .map(|(start, end)| end - start)
            .sum();
        Some(Micros(host - zero - paused))
    }

    /// How long the recording is at a moment, which stops growing during a pause.
    #[must_use]
    pub fn length_at(&self, now: i64) -> Micros {
        let end = self.paused_since.map_or(now, |since| now.min(since));
        let Some(zero) = self.zero else {
            return Micros::ZERO;
        };
        if end < zero {
            return Micros::ZERO;
        }
        let paused: i64 = self
            .pauses
            .iter()
            .filter(|(start, pause_end)| *start >= zero && *pause_end <= end)
            .map(|(start, pause_end)| pause_end - start)
            .sum();
        Micros(end - zero - paused)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_frame_is_time_zero() {
        let mut clock = RecordingClock::new(false);
        assert_eq!(clock.place_frame(5_000), Some(Micros(0)));
        assert_eq!(clock.place_frame(5_100), Some(Micros(100)));
    }

    #[test]
    fn sound_before_the_first_frame_has_no_place() {
        let mut clock = RecordingClock::new(false);
        assert_eq!(clock.place(4_000), None);
        clock.place_frame(5_000);
        assert_eq!(clock.place(4_999), None);
        assert_eq!(clock.place(5_200), Some(Micros(200)));
    }

    #[test]
    fn paused_time_comes_out_of_everything_after_it() {
        let mut clock = RecordingClock::new(false);
        clock.place_frame(0);
        clock.pause(1_000);
        assert_eq!(
            clock.place_frame(1_500),
            None,
            "frames during a pause are dropped"
        );
        clock.resume(2_000);
        assert_eq!(clock.place_frame(2_000), Some(Micros(1_000)));
        assert_eq!(clock.place(3_000), Some(Micros(2_000)));
    }

    #[test]
    fn sound_captured_before_a_pause_keeps_its_place_even_if_it_arrives_after() {
        let mut clock = RecordingClock::new(false);
        clock.place_frame(0);
        clock.pause(1_000);
        clock.resume(5_000);
        assert_eq!(clock.place(900), Some(Micros(900)));
        assert_eq!(
            clock.place(2_000),
            None,
            "and sound from during it still goes"
        );
        assert_eq!(clock.place(5_100), Some(Micros(1_100)));
    }

    #[test]
    fn several_pauses_add_up() {
        let mut clock = RecordingClock::new(false);
        clock.place_frame(0);
        for (start, end) in [(100, 200), (300, 500), (600, 1_000)] {
            clock.pause(start);
            clock.resume(end);
        }
        assert_eq!(clock.place(1_100), Some(Micros(400)));
    }

    #[test]
    fn a_warm_start_begins_at_the_first_frame_after_resuming() {
        let mut clock = RecordingClock::new(true);
        assert_eq!(
            clock.place_frame(1_000),
            None,
            "the warm-up frames are dropped"
        );
        assert!(!clock.has_started());
        clock.resume(4_000);
        assert_eq!(
            clock.place_frame(3_900),
            None,
            "one from before the resume still counts as warm-up"
        );
        assert_eq!(clock.place_frame(4_050), Some(Micros(0)));
        assert_eq!(
            clock.place(4_150),
            Some(Micros(100)),
            "and the countdown isn't counted as paused time"
        );
    }

    #[test]
    fn pausing_twice_or_resuming_without_a_pause_changes_nothing() {
        let mut clock = RecordingClock::new(false);
        clock.place_frame(0);
        clock.resume(10);
        clock.pause(100);
        clock.pause(200);
        clock.resume(300);
        assert_eq!(clock.place(400), Some(Micros(200)));
    }

    #[test]
    fn the_length_stops_growing_during_a_pause() {
        let mut clock = RecordingClock::new(false);
        assert_eq!(clock.length_at(100), Micros::ZERO);
        clock.place_frame(1_000);
        assert_eq!(clock.length_at(1_500), Micros(500));
        clock.pause(2_000);
        assert_eq!(clock.length_at(9_000), Micros(1_000));
        clock.resume(10_000);
        assert_eq!(clock.length_at(10_500), Micros(1_500));
    }
}
