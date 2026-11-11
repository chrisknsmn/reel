//! Suggesting zooms from where the user clicked.

use reel_core::time::{Micros, TimelineTime};
use reel_core::timeline::{Point, Timeline};

use crate::cursor::{CursorSample, CursorTrack, Interaction};

/// Clicks this close together become one zoom.
const GROUP_GAP: Micros = Micros(2_500_000);
/// How long a zoom starts before its first click, and runs after its last.
const MARGIN: Micros = Micros(500_000);

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Suggestion {
    pub start: TimelineTime,
    pub end: TimelineTime,
    pub focus: Point,
}

fn is_click(interaction: Interaction) -> bool {
    matches!(
        interaction,
        Interaction::Click
            | Interaction::DoubleClick
            | Interaction::RightClick
            | Interaction::MiddleClick
    )
}

/// How interesting a click looks. A double-click means more than a plain one.
fn strength(track: &CursorTrack, index: usize) -> u32 {
    let samples = &track.samples;
    let click = &samples[index];
    if click.interaction == Interaction::DoubleClick {
        return 1500;
    }
    // A drag sideways after pressing down is selecting text.
    let release = samples[index + 1..]
        .iter()
        .take_while(|sample| sample.time - click.time <= Micros::from_secs(3))
        .find(|sample| sample.interaction == Interaction::Mouseup);
    if let Some(up) = release {
        let (dx, dy) = ((up.x - click.x).abs(), (up.y - click.y).abs());
        if up.time - click.time >= Micros::from_ms(200.0) && dx > 0.03 && dx > 1.8 * dy {
            return 1300;
        }
    }

    // Otherwise, look at how the cursor moved just afterwards.
    let moves: Vec<&CursorSample> = samples[index + 1..]
        .iter()
        .filter(|sample| {
            let age = sample.time - click.time;
            sample.interaction == Interaction::Move
                && age >= Micros::from_ms(100.0)
                && age <= Micros::from_ms(2000.0)
        })
        .collect();
    let furthest = moves
        .iter()
        .map(|sample| (sample.x - click.x).hypot(sample.y - click.y))
        .fold(0.0, f64::max);
    if moves.len() < 3 || furthest < 0.02 {
        // It stayed put: probably clicked into a text field to type.
        return 1100;
    }
    let mut last = click;
    let (mut total_dx, mut total_dy) = (0.0, 0.0);
    for sample in &moves {
        total_dx += (sample.x - last.x).abs();
        total_dy += (sample.y - last.y).abs();
        last = sample;
    }
    let net_dy = (last.y - click.y).abs();
    if net_dy > 0.03 && total_dy > 1.5 * total_dx {
        return 1200;
    }
    if total_dx > 0.03 && total_dx > 1.8 * total_dy {
        return 1300;
    }
    900
}

/// Zooms around clusters of clicks, skipping anything that would overlap a zoom already there.
#[must_use]
pub fn suggest_zooms(track: &CursorTrack, timeline: &Timeline) -> Vec<Suggestion> {
    let total = TimelineTime::ZERO + timeline.duration();

    // Clicks are on the recording's clock. Each goes through the clips, which drops any that were cut.
    let clicks: Vec<(usize, TimelineTime)> = track
        .samples
        .iter()
        .enumerate()
        .filter(|(_, sample)| is_click(sample.interaction))
        .filter_map(|(index, sample)| timeline.to_timeline(sample.time).map(|time| (index, time)))
        .collect();

    let mut groups: Vec<Vec<(usize, TimelineTime)>> = Vec::new();
    for click in clicks {
        match groups.last_mut() {
            Some(group) if click.1 - group[group.len() - 1].1 <= GROUP_GAP => group.push(click),
            _ => groups.push(vec![click]),
        }
    }

    let mut suggestions: Vec<Suggestion> = Vec::new();
    for group in groups {
        let first = group[0].1;
        let last = group[group.len() - 1].1;
        let start = (first - MARGIN).max(TimelineTime::ZERO);
        let end = (last + MARGIN).min(total);
        let overlaps = timeline
            .zooms
            .iter()
            .any(|zoom| start < zoom.end && zoom.start < end)
            || suggestions
                .iter()
                .any(|other| start < other.end && other.start < end);
        if overlaps || end <= start {
            continue;
        }
        // The strongest click sets the focus, and the earliest wins a tie.
        let mut best = (group[0].0, strength(track, group[0].0));
        for &(index, _) in &group[1..] {
            let value = strength(track, index);
            if value > best.1 {
                best = (index, value);
            }
        }
        let sample = &track.samples[best.0];
        suggestions.push(Suggestion {
            start,
            end,
            focus: Point {
                x: sample.x.clamp(0.0, 1.0),
                y: sample.y.clamp(0.0, 1.0),
            },
        });
    }
    suggestions
}

/// Suggestions only make sense on landscape video. A source near square or portrait gets none.
#[must_use]
pub fn suits_auto_zoom(width: u32, height: u32) -> bool {
    f64::from(width) / f64::from(height.max(1)) >= 1.2
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cursor::CursorShape;
    use reel_core::edit::Edit;
    use reel_core::time::SourceTime;
    use reel_core::timeline::ZoomMode;

    fn sample(ms: f64, x: f64, y: f64, interaction: Interaction) -> CursorSample {
        CursorSample {
            time: SourceTime::from_ms(ms),
            x,
            y,
            interaction,
            shape: CursorShape::Arrow,
        }
    }

    fn click(ms: f64, x: f64, y: f64) -> CursorSample {
        sample(ms, x, y, Interaction::Click)
    }

    fn timeline() -> Timeline {
        Timeline::for_recording(Micros::from_secs(30))
    }

    fn at(ms: f64) -> TimelineTime {
        TimelineTime::from_ms(ms)
    }

    #[test]
    fn a_lone_click_gets_half_a_second_either_side() {
        let track = CursorTrack {
            samples: vec![click(5000.0, 0.3, 0.7)],
        };
        let found = suggest_zooms(&track, &timeline());
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].start, found[0].end), (at(4500.0), at(5500.0)));
        assert_eq!(found[0].focus, Point { x: 0.3, y: 0.7 });
    }

    #[test]
    fn nearby_clicks_group_and_distant_ones_dont() {
        let track = CursorTrack {
            samples: vec![
                click(5000.0, 0.1, 0.1),
                click(7000.0, 0.2, 0.2),
                click(15_000.0, 0.3, 0.3),
            ],
        };
        let found = suggest_zooms(&track, &timeline());
        assert_eq!(found.len(), 2);
        assert_eq!((found[0].start, found[0].end), (at(4500.0), at(7500.0)));
        assert_eq!(found[1].start, at(14_500.0));
    }

    #[test]
    fn moves_alone_suggest_nothing() {
        let track = CursorTrack {
            samples: vec![sample(5000.0, 0.3, 0.7, Interaction::Move)],
        };
        assert!(suggest_zooms(&track, &timeline()).is_empty());
    }

    #[test]
    fn the_strongest_click_sets_the_focus() {
        let track = CursorTrack {
            samples: vec![
                // A plain click that then wanders, so it's classed as plain.
                click(5000.0, 0.1, 0.1),
                sample(5200.0, 0.15, 0.15, Interaction::Move),
                sample(5400.0, 0.2, 0.2, Interaction::Move),
                sample(5600.0, 0.25, 0.25, Interaction::Move),
                sample(6000.0, 0.8, 0.8, Interaction::DoubleClick),
            ],
        };
        let found = suggest_zooms(&track, &timeline());
        assert_eq!(found[0].focus, Point { x: 0.8, y: 0.8 });
    }

    #[test]
    fn the_first_click_wins_a_tie() {
        let track = CursorTrack {
            samples: vec![click(5000.0, 0.1, 0.1), click(6000.0, 0.9, 0.9)],
        };
        assert_eq!(suggest_zooms(&track, &timeline())[0].focus.x, 0.1);
    }

    #[test]
    fn nothing_overlaps_a_zoom_already_there() {
        let mut timeline = timeline();
        timeline
            .apply(&Edit::AddZoom {
                start: at(5000.0),
                depth: 1,
                focus: Point::CENTER,
                mode: ZoomMode::Auto,
            })
            .unwrap();
        let track = CursorTrack {
            samples: vec![click(5500.0, 0.5, 0.5), click(20_000.0, 0.5, 0.5)],
        };
        let found = suggest_zooms(&track, &timeline);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].start, at(19_500.0));
    }

    #[test]
    fn the_region_stops_at_the_end_of_the_video() {
        let track = CursorTrack {
            samples: vec![click(100.0, 0.5, 0.5), click(29_900.0, 0.5, 0.5)],
        };
        let found = suggest_zooms(&track, &timeline());
        assert_eq!(found[0].start, TimelineTime::ZERO);
        assert_eq!(found[1].end, at(30_000.0));
    }

    #[test]
    fn click_times_go_through_the_clips() {
        // At 2×, a click 10 s into the recording belongs at 5 s on the timeline.
        let mut timeline = timeline();
        timeline
            .apply(&Edit::SetClipSpeed {
                id: "clip-1".into(),
                speed: 2.0,
            })
            .unwrap();
        let track = CursorTrack {
            samples: vec![click(10_000.0, 0.5, 0.5)],
        };
        assert_eq!(suggest_zooms(&track, &timeline)[0].start, at(4500.0));
    }

    #[test]
    fn clicks_in_cut_footage_are_ignored() {
        let mut timeline = timeline();
        timeline
            .apply(&Edit::TrimClip {
                id: "clip-1".into(),
                edge: reel_core::edit::Edge::End,
                to: at(5000.0),
            })
            .unwrap();
        let track = CursorTrack {
            samples: vec![click(20_000.0, 0.5, 0.5)],
        };
        assert!(suggest_zooms(&track, &timeline).is_empty());
    }

    #[test]
    fn portrait_sources_get_no_suggestions() {
        assert!(suits_auto_zoom(1920, 1080));
        assert!(!suits_auto_zoom(1080, 1920));
        assert!(!suits_auto_zoom(1000, 1000));
    }

    #[test]
    fn classifying_clicks_follows_the_documented_rules() {
        let score = |samples: Vec<CursorSample>| strength(&CursorTrack { samples }, 0);
        assert_eq!(
            score(vec![sample(0.0, 0.5, 0.5, Interaction::DoubleClick)]),
            1500
        );
        assert_eq!(
            score(vec![click(0.0, 0.5, 0.5)]),
            1100,
            "stays put: a text field"
        );
        assert_eq!(
            score(vec![
                click(0.0, 0.2, 0.5),
                sample(500.0, 0.5, 0.5, Interaction::Mouseup)
            ]),
            1300,
            "drags across: a selection"
        );
        assert_eq!(
            score(vec![
                click(0.0, 0.5, 0.2),
                sample(200.0, 0.5, 0.3, Interaction::Move),
                sample(400.0, 0.5, 0.4, Interaction::Move),
                sample(600.0, 0.5, 0.5, Interaction::Move),
            ]),
            1200,
            "moves down: a dropdown"
        );
    }
}
