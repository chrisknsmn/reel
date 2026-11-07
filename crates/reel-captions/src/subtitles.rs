//! `.srt` and `.vtt` files, timed for the exported video (§12, BUILD_PLAN 9.6).
//!
//! An export skips the gaps between clips, so a caption's time on the timeline isn't its time in the
//! video. This is what maps through trims and speed changes correctly.

use reel_core::time::{Micros, TimelineTime};
use reel_core::timeline::{Caption, Timeline};

/// Where a moment on the timeline lands in the export, or `None` if it's in a gap.
#[must_use]
pub fn output_time(timeline: &Timeline, time: TimelineTime) -> Option<Micros> {
    let mut before = Micros::ZERO;
    for clip in &timeline.clips {
        if time < clip.start {
            return None;
        }
        if time < clip.end() {
            return Some(before + (time - clip.start));
        }
        before = before + clip.duration;
    }
    None
}

/// Where a caption's start and end land. A caption running into a gap is cut off where its clip ends,
/// and one wholly in a gap isn't in the video at all.
#[must_use]
pub fn output_span(timeline: &Timeline, caption: &Caption) -> Option<(Micros, Micros)> {
    let mut before = Micros::ZERO;
    let mut span: Option<(Micros, Micros)> = None;
    for clip in &timeline.clips {
        let start = caption.start.max(clip.start);
        let end = caption.end.min(clip.end());
        if start < end {
            let from = before + (start - clip.start);
            let to = before + (end - clip.start);
            span = Some(match span {
                Some((first, _)) => (first, to),
                None => (from, to),
            });
        }
        before = before + clip.duration;
    }
    span
}

fn parts(time: Micros) -> (i64, i64, i64, i64) {
    let total = (time.0.max(0) + 500) / 1000;
    (
        total / 3_600_000,
        total / 60_000 % 60,
        total / 1000 % 60,
        total % 1000,
    )
}

fn srt_time(time: Micros) -> String {
    let (hours, minutes, seconds, millis) = parts(time);
    format!("{hours:02}:{minutes:02}:{seconds:02},{millis:03}")
}

fn vtt_time(time: Micros) -> String {
    let (hours, minutes, seconds, millis) = parts(time);
    format!("{hours:02}:{minutes:02}:{seconds:02}.{millis:03}")
}

fn cues(timeline: &Timeline) -> Vec<(Micros, Micros, String)> {
    timeline
        .captions
        .iter()
        .filter(|caption| !caption.text.trim().is_empty())
        .filter_map(|caption| {
            let (start, end) = output_span(timeline, caption)?;
            Some((start, end, caption.text.trim().to_string()))
        })
        .collect()
}

/// The captions as a SubRip file.
#[must_use]
pub fn to_srt(timeline: &Timeline) -> String {
    cues(timeline)
        .iter()
        .enumerate()
        .map(|(index, (start, end, text))| {
            format!(
                "{}\n{} --> {}\n{text}\n",
                index + 1,
                srt_time(*start),
                srt_time(*end)
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The captions as a WebVTT file.
#[must_use]
pub fn to_vtt(timeline: &Timeline) -> String {
    let body: Vec<String> = cues(timeline)
        .iter()
        .map(|(start, end, text)| format!("{} --> {}\n{text}\n", vtt_time(*start), vtt_time(*end)))
        .collect();
    format!("WEBVTT\n\n{}", body.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use reel_core::edit::{Edge, Edit};
    use reel_core::timeline::Clip;

    fn at(ms: f64) -> TimelineTime {
        TimelineTime::from_ms(ms)
    }

    fn caption(start: f64, end: f64, text: &str) -> Caption {
        Caption {
            id: format!("c{start}").as_str().into(),
            start: at(start),
            end: at(end),
            text: text.to_string(),
            words: vec![],
        }
    }

    fn timeline() -> Timeline {
        Timeline::for_recording(Micros::from_secs(10))
    }

    #[test]
    fn srt_uses_commas_and_counts_from_one() {
        let mut timeline = timeline();
        timeline.captions = vec![
            caption(1000.0, 2500.0, "Hello"),
            caption(3000.0, 4000.0, "there"),
        ];
        assert_eq!(
            to_srt(&timeline),
            "1\n00:00:01,000 --> 00:00:02,500\nHello\n\n2\n00:00:03,000 --> 00:00:04,000\nthere\n"
        );
    }

    #[test]
    fn vtt_has_its_header_and_uses_dots() {
        let mut timeline = timeline();
        timeline.captions = vec![caption(1000.0, 2500.0, "Hello")];
        assert_eq!(
            to_vtt(&timeline),
            "WEBVTT\n\n00:00:01.000 --> 00:00:02.500\nHello\n"
        );
    }

    #[test]
    fn long_times_show_hours() {
        assert_eq!(srt_time(Micros::from_secs(3725)), "01:02:05,000");
        assert_eq!(vtt_time(Micros::from_ms(1.5)), "00:00:00.002");
    }

    #[test]
    fn a_gap_on_the_timeline_comes_out_of_later_times() {
        let mut timeline = timeline();
        // Cut out 3 s to 5 s, leaving a gap, as trimming does.
        let id = timeline.clips[0].id.clone();
        timeline.apply(&Edit::SplitClip { at: at(5000.0) }).unwrap();
        timeline
            .apply(&Edit::TrimClip {
                id,
                edge: Edge::End,
                to: at(3000.0),
            })
            .unwrap();
        assert_eq!(
            output_time(&timeline, at(1000.0)),
            Some(Micros::from_secs(1))
        );
        assert_eq!(output_time(&timeline, at(4000.0)), None, "in the gap");
        assert_eq!(
            output_time(&timeline, at(6000.0)),
            Some(Micros::from_secs(4))
        );

        timeline.captions = vec![caption(6000.0, 7000.0, "later")];
        assert!(
            to_srt(&timeline).contains("00:00:04,000 --> 00:00:05,000"),
            "{}",
            to_srt(&timeline)
        );
    }

    #[test]
    fn a_caption_in_a_gap_isnt_in_the_file() {
        let mut timeline = timeline();
        let id = timeline.clips[0].id.clone();
        timeline.apply(&Edit::SplitClip { at: at(5000.0) }).unwrap();
        timeline
            .apply(&Edit::TrimClip {
                id,
                edge: Edge::End,
                to: at(3000.0),
            })
            .unwrap();
        timeline.captions = vec![caption(3500.0, 4500.0, "gone")];
        assert_eq!(to_srt(&timeline), "");
    }

    #[test]
    fn a_caption_across_a_gap_has_the_gap_taken_out() {
        let mut timeline = timeline();
        let id = timeline.clips[0].id.clone();
        timeline.apply(&Edit::SplitClip { at: at(5000.0) }).unwrap();
        timeline
            .apply(&Edit::TrimClip {
                id,
                edge: Edge::End,
                to: at(3000.0),
            })
            .unwrap();
        // From 2 s, across the gap, to 6 s. In the video, that's 2 s to 4 s.
        let span = output_span(&timeline, &caption(2000.0, 6000.0, "x"));
        assert_eq!(span, Some((Micros::from_secs(2), Micros::from_secs(4))));
    }

    #[test]
    fn a_faster_clip_is_already_shorter_on_the_timeline() {
        let timeline = Timeline {
            clips: vec![Clip {
                id: "a".into(),
                start: at(0.0),
                duration: Micros::from_secs(5),
                source_start: reel_core::time::SourceTime::ZERO,
                speed: 2.0,
                muted: false,
                show_source_audio: false,
                track_settings: Vec::new(),
            }],
            captions: vec![caption(1000.0, 2000.0, "x")],
            source_duration: Micros::from_secs(10),
            ..Timeline::default()
        };
        assert!(to_srt(&timeline).contains("00:00:01,000 --> 00:00:02,000"));
    }

    #[test]
    fn blank_captions_are_left_out() {
        let mut timeline = timeline();
        timeline.captions = vec![caption(1000.0, 2000.0, "   ")];
        assert_eq!(to_srt(&timeline), "");
        assert_eq!(to_vtt(&timeline), "WEBVTT\n\n");
    }
}
