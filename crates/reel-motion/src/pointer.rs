//! The cursor's whole path through a project, worked out once, like the camera's.

use reel_core::project::{ClickEffect, CursorLook, Project};
use reel_core::time::{Micros, SourceTime, TimelineTime};

use crate::camera::STEP;
use crate::cursor::{CursorSample, CursorShape, CursorTrack, Interaction};
use crate::spring::{Spring, SpringConfig, cursor_config};

/// One step of the path.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Pose {
    x: f32,
    y: f32,
    rotation: f32,
}

/// What to draw for the cursor at a moment.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PointerFrame {
    /// From 0 to 1 across the full, uncropped recording.
    pub x: f64,
    pub y: f64,
    /// In radians.
    pub rotation: f64,
    /// The click bounce, 1 when there isn't one.
    pub scale: f64,
    pub shape: CursorShape,
    /// False when the cursor is outside the crop, or there's nothing showing.
    pub visible: bool,
}

/// A click on the timeline, for bounces and click effects.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Click {
    pub time: TimelineTime,
    pub x: f64,
    pub y: f64,
}

/// One ring of a click effect, in pixels at the size given.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ring {
    pub x: f64,
    pub y: f64,
    pub radius: f64,
    /// Zero means a filled dot.
    pub stroke: f64,
    pub alpha: f64,
}

/// Softer settings for sway than for position.
const SWAY_SPRING: SpringConfig = SpringConfig {
    stiffness: 120.0,
    damping: 16.0,
    mass: 1.0,
    rest_delta: 0.0005,
    rest_speed: 0.01,
};
/// A jump in the cursor's time bigger than this snaps rather than swings.
const JUMP: Micros = Micros(100_000);

/// Everything the renderer needs about the cursor.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PointerPath {
    poses: Vec<Pose>,
    clicks: Vec<Click>,
    look: Option<CursorLook>,
    track: CursorTrack,
    /// Times where the timeline is showing footage, for the shape and visibility.
    timeline: Vec<(TimelineTime, TimelineTime, SourceTime, f64)>,
    crop: (f64, f64, f64, f64),
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

impl PointerPath {
    /// Works out the path, or an empty one for a project with no cursor data.
    #[must_use]
    pub fn build(project: &Project, track: Option<&CursorTrack>) -> Self {
        let timeline = &project.timeline;
        let look = &project.cursor;
        let crop = project.look.crop;
        let spans: Vec<_> = timeline
            .clips
            .iter()
            .map(|clip| (clip.start, clip.end(), clip.source_start, clip.speed))
            .collect();
        let Some(track) = track.filter(|track| !track.samples.is_empty() && look.show) else {
            return Self {
                look: Some(look.clone()),
                timeline: spans,
                crop: (crop.x, crop.y, crop.width, crop.height),
                ..Self::default()
            };
        };
        let track = if look.looped {
            looped(track)
        } else {
            track.clone()
        };

        let config = cursor_config(look.smoothing, look.spring);
        let (mut x, mut y, mut rotation) =
            (Spring::default(), Spring::default(), Spring::default());
        let step_ms = STEP.ms();
        let length = (timeline.duration().0 / STEP.0).max(0) as usize + 2;
        let mut poses = Vec::with_capacity(length);
        let mut previous_source: Option<SourceTime> = None;
        let mut previous_position: Option<(f64, f64)> = None;
        let mut pose = Pose {
            x: 0.0,
            y: 0.0,
            rotation: 0.0,
        };

        for index in 0..length {
            let time = TimelineTime(index as i64 * STEP.0);
            if let Some(source) = timeline.to_source(time)
                && let Some((target_x, target_y)) = track.position_at(source)
            {
                // Cuts and big jumps snap. Otherwise the cursor would swing across the frame.
                let jumped = previous_source.is_none_or(|last| {
                    let difference = source - last;
                    difference > JUMP.scaled(clip_speed(&spans, time).max(1.0))
                        || difference < Micros::ZERO
                });
                if jumped {
                    x.snap(target_x);
                    y.snap(target_y);
                } else {
                    x.step(target_x, step_ms, &config);
                    y.step(target_y, step_ms, &config);
                }

                // Sway leans the cursor into its motion.
                let swing =
                    previous_position
                        .filter(|_| !jumped)
                        .map_or(0.0, |(last_x, last_y)| {
                            let (dx, dy) =
                                ((x.value - last_x) * 1920.0, (y.value - last_y) * 1080.0);
                            let distance = (dx * dx + dy * dy).sqrt();
                            if distance < 0.01 {
                                return 0.0;
                            }
                            let speed = distance / (step_ms / 1000.0);
                            ((dx + 0.65 * dy) / distance).clamp(-1.0, 1.0)
                                * (speed / 1400.0).min(1.0)
                                * (std::f64::consts::PI / 18.0)
                                * look.sway
                                * 3.0
                        });
                if jumped {
                    rotation.snap(0.0);
                } else {
                    rotation.step(swing, step_ms, &SWAY_SPRING);
                }
                previous_position = Some((x.value, y.value));
                previous_source = Some(source);
                pose = Pose {
                    x: x.value as f32,
                    y: y.value as f32,
                    rotation: rotation.value as f32,
                };
            } else {
                previous_source = None;
            }
            poses.push(pose);
        }

        // Clicks are on the recording's clock, so each goes through the clips to reach the timeline.
        // A click in footage that was cut simply doesn't happen.
        let mut clicks: Vec<Click> = track
            .samples
            .iter()
            .filter(|sample| is_click(sample.interaction))
            .filter_map(|sample| {
                timeline.to_timeline(sample.time).map(|time| Click {
                    time,
                    x: sample.x,
                    y: sample.y,
                })
            })
            .collect();
        clicks.sort_by_key(|click| click.time);

        Self {
            poses,
            clicks,
            look: Some(look.clone()),
            track,
            timeline: spans,
            crop: (crop.x, crop.y, crop.width, crop.height),
        }
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.poses.is_empty()
    }

    fn source_at(&self, time: TimelineTime) -> Option<SourceTime> {
        self.timeline
            .iter()
            .find(|(start, end, _, _)| time >= *start && time < *end)
            .map(|(start, _, source_start, speed)| *source_start + (time - *start).scaled(*speed))
    }

    /// The cursor at a time.
    #[must_use]
    pub fn sample(&self, time: TimelineTime) -> Option<PointerFrame> {
        let look = self.look.as_ref()?;
        let last = self.poses.last()?;
        let position = time.0.max(0) as f64 / STEP.0 as f64;
        let index = position as usize;
        let (a, b, t) = if index + 1 < self.poses.len() {
            (
                self.poses[index],
                self.poses[index + 1],
                position - index as f64,
            )
        } else {
            (*last, *last, 0.0)
        };
        let mix = |p: f32, q: f32| f64::from(p) + f64::from(q - p) * t;
        let (x, y) = (mix(a.x, b.x), mix(a.y, b.y));

        let (crop_x, crop_y, crop_w, crop_h) = self.crop;
        let in_crop = x >= crop_x && x <= crop_x + crop_w && y >= crop_y && y <= crop_y + crop_h;
        let source = self.source_at(time);
        Some(PointerFrame {
            x,
            y,
            rotation: mix(a.rotation, b.rotation),
            scale: self.bounce_at(time, look),
            shape: source.map_or(CursorShape::Arrow, |source| self.shape_at(source)),
            visible: source.is_some() && in_crop,
        })
    }

    /// The latest shape, ignoring click samples, which don't reliably report one.
    fn shape_at(&self, source: SourceTime) -> CursorShape {
        let next = self
            .track
            .samples
            .partition_point(|sample| sample.time <= source);
        self.track.samples[..next]
            .iter()
            .rev()
            .find(|sample| !is_click(sample.interaction))
            .map_or(CursorShape::Arrow, |sample| sample.shape)
    }

    /// `progress = 1 − age/duration`, `scale = max(0.72, 1 − sin(progress × π) × 0.08 × bounce)`.
    fn bounce_at(&self, time: TimelineTime, look: &CursorLook) -> f64 {
        let duration = Micros::from_ms(look.click_bounce_ms.max(1.0));
        self.recent_clicks(time, duration)
            .map(|click| {
                let progress = 1.0 - (time - click.time).0 as f64 / duration.0 as f64;
                (1.0 - (progress * std::f64::consts::PI).sin() * 0.08 * look.click_bounce).max(0.72)
            })
            .fold(1.0, f64::min)
    }

    fn recent_clicks(&self, time: TimelineTime, within: Micros) -> impl Iterator<Item = &Click> {
        let end = self.clicks.partition_point(|click| click.time <= time);
        let start = self.clicks[..end].partition_point(|click| click.time < time - within);
        self.clicks[start..end].iter()
    }

    /// The rings to draw for click effects at a time. `height` is the cursor's height in pixels before
    /// any bounce, which is what the effects are sized from.
    #[must_use]
    pub fn rings_at(&self, time: TimelineTime, height: f64) -> Vec<Ring> {
        let Some(look) = &self.look else {
            return Vec::new();
        };
        if look.click_effect == ClickEffect::None {
            return Vec::new();
        }
        // Effects start half a bounce after the click.
        let delay = Micros::from_ms(look.click_bounce_ms / 2.0);
        let duration = Micros::from_ms(look.click_effect_ms.max(1.0));
        let mut rings = Vec::new();
        for click in self.recent_clicks(time, delay + duration) {
            let age = (time - click.time) - delay;
            if age < Micros::ZERO {
                continue;
            }
            let p = (1.0 - age.0 as f64 / duration.0 as f64).clamp(0.0, 1.0);
            rings.extend(click_rings(
                click,
                look.click_effect,
                p,
                height,
                look.click_effect_scale,
                look.click_effect_opacity,
            ));
        }
        rings
    }
}

fn clip_speed(spans: &[(TimelineTime, TimelineTime, SourceTime, f64)], time: TimelineTime) -> f64 {
    spans
        .iter()
        .find(|(start, end, _, _)| time >= *start && time < *end)
        .map_or(1.0, |(_, _, _, speed)| *speed)
}

fn click_rings(
    click: &Click,
    effect: ClickEffect,
    p: f64,
    h: f64,
    scale: f64,
    opacity: f64,
) -> Vec<Ring> {
    let base = (h * 0.55 * scale).max(12.0);
    let sw = (h * 0.08).max(2.0);
    let a = p * opacity;
    let ring = |radius: f64, stroke: f64, alpha: f64| Ring {
        x: click.x,
        y: click.y,
        radius,
        stroke,
        alpha,
    };
    match effect {
        ClickEffect::None => Vec::new(),
        ClickEffect::Ripple => {
            let cubed = p * p * p;
            vec![ring(
                ((1.0 - cubed) * h * 1.95 * scale).max(0.5),
                (2.0 * cubed).max(1.0),
                cubed * 0.6 * opacity,
            )]
        }
        ClickEffect::Spotlight => {
            let glow = base + (1.0 - p) * h * scale;
            vec![
                ring(glow, (0.68 * sw).max(1.25), 0.28 * a),
                ring(
                    (0.72 * base).max(0.76 * glow),
                    (0.75 * sw).max(1.5),
                    0.5 * a,
                ),
            ]
        }
        ClickEffect::Echo => {
            let outer = base + (1.0 - p) * h * 1.22 * scale;
            vec![
                ring(outer, sw, 0.72 * a),
                ring(
                    (0.58 * base).max(0.62 * outer),
                    (0.72 * sw).max(1.4),
                    0.42 * a,
                ),
                ring((0.18 * base).max(3.0), 0.0, 0.14 * a),
            ]
        }
    }
}

const FREEZE: Micros = Micros(670_000);
const RETURN: Micros = Micros(550_000);
const SETTLE: Micros = Micros(120_000);
const TRAILING_MOVEMENT: f64 = 0.0015;

fn ease_out_quint(t: f64) -> f64 {
    1.0 - (1.0 - t).powi(5)
}

/// Rewrites a cursor track so it ends where it began, for videos that loop.
#[must_use]
pub fn looped(track: &CursorTrack) -> CursorTrack {
    let (Some(first), Some(last)) = (
        track.samples.first().copied(),
        track.samples.last().copied(),
    ) else {
        return track.clone();
    };
    let total = last.time - first.time;
    let hold = FREEZE.0 + RETURN.0 + SETTLE.0;
    if total.0 <= hold {
        return track.clone();
    }

    // Real motion is everything up to the last real movement.
    let motion_end = track
        .samples
        .windows(2)
        .rev()
        .find(|pair| {
            (pair[1].x - pair[0].x).abs() > TRAILING_MOVEMENT
                || (pair[1].y - pair[0].y).abs() > TRAILING_MOVEMENT
        })
        .map_or(first.time, |pair| pair[1].time);
    let room = (total.0 - hold) as f64;
    let span = (motion_end - first.time).0.max(1) as f64;
    let squeeze = (room / span).min(1.0);

    let mut samples: Vec<CursorSample> = track
        .samples
        .iter()
        .filter(|sample| sample.time <= motion_end)
        .map(|sample| CursorSample {
            time: first.time + Micros(((sample.time - first.time).0 as f64 * squeeze) as i64),
            ..*sample
        })
        .collect();
    let compressed_end = samples.last().map_or(first.time, |sample| sample.time);
    let from = samples.last().copied().unwrap_or(first);

    // Hold, then ease back, then settle on the start.
    let return_start = last.time - RETURN - SETTLE;
    samples.push(CursorSample {
        time: return_start.max(compressed_end),
        interaction: Interaction::Move,
        ..from
    });
    for step in 1..=21 {
        let progress = f64::from(step) / 21.0;
        let eased = ease_out_quint(progress);
        samples.push(CursorSample {
            time: return_start + RETURN.scaled(progress),
            x: from.x + (first.x - from.x) * eased,
            y: from.y + (first.y - from.y) * eased,
            interaction: Interaction::Move,
            shape: first.shape,
        });
    }
    for step in 1..=4 {
        samples.push(CursorSample {
            time: last.time - SETTLE + SETTLE.scaled(f64::from(step) / 4.0),
            x: first.x,
            y: first.y,
            interaction: Interaction::Move,
            shape: first.shape,
        });
    }
    CursorTrack { samples }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reel_core::edit::Edit;
    use reel_core::timeline::Timeline;

    fn sample(
        ms: f64,
        x: f64,
        y: f64,
        interaction: Interaction,
        shape: CursorShape,
    ) -> CursorSample {
        CursorSample {
            time: SourceTime::from_ms(ms),
            x,
            y,
            interaction,
            shape,
        }
    }

    fn moving(ms: f64, x: f64, y: f64) -> CursorSample {
        sample(ms, x, y, Interaction::Move, CursorShape::Arrow)
    }

    fn project(seconds: i64) -> Project {
        Project {
            timeline: Timeline::for_recording(Micros::from_secs(seconds)),
            ..Project::default()
        }
    }

    fn at(ms: f64) -> TimelineTime {
        TimelineTime::from_ms(ms)
    }

    #[test]
    fn with_no_data_there_is_no_cursor() {
        let path = PointerPath::build(&project(5), None);
        assert!(path.is_empty());
        assert_eq!(path.sample(at(100.0)), None);
    }

    #[test]
    fn turning_the_cursor_off_hides_it() {
        let mut project = project(5);
        project.cursor.show = false;
        let track = CursorTrack {
            samples: vec![moving(0.0, 0.5, 0.5)],
        };
        assert!(PointerPath::build(&project, Some(&track)).is_empty());
    }

    #[test]
    fn the_cursor_settles_on_where_it_really_is() {
        let track = CursorTrack {
            samples: vec![moving(0.0, 0.2, 0.2), moving(100.0, 0.8, 0.6)],
        };
        let path = PointerPath::build(&project(5), Some(&track));
        let settled = path.sample(at(3000.0)).unwrap();
        assert!((settled.x - 0.8).abs() < 1e-3);
        assert!((settled.y - 0.6).abs() < 1e-3);
    }

    #[test]
    fn the_spring_smooths_a_sudden_move() {
        let track = CursorTrack {
            samples: vec![
                moving(0.0, 0.2, 0.2),
                moving(1000.0, 0.2, 0.2),
                moving(1016.0, 0.8, 0.2),
            ],
        };
        let path = PointerPath::build(&project(5), Some(&track));
        let during = path.sample(at(1030.0)).unwrap().x;
        assert!(
            during > 0.2 && during < 0.75,
            "should still be on its way: {during}"
        );
    }

    #[test]
    fn a_cut_snaps_the_cursor_rather_than_swinging_it() {
        // The cursor is at 0.2 until 5 s, then 0.8. Removing the footage between makes a cut.
        let track = CursorTrack {
            samples: vec![
                moving(0.0, 0.2, 0.5),
                moving(4999.0, 0.2, 0.5),
                moving(5000.0, 0.8, 0.5),
                moving(10_000.0, 0.8, 0.5),
            ],
        };
        let mut project = project(10);
        project
            .timeline
            .apply(&Edit::SplitClip { at: at(2000.0) })
            .unwrap();
        let second = project.timeline.clips[1].id.clone();
        project
            .timeline
            .apply(&Edit::TrimClip {
                id: second,
                edge: reel_core::edit::Edge::Start,
                to: at(8000.0),
            })
            .unwrap();
        let path = PointerPath::build(&project, Some(&track));
        let right_after = path.sample(TimelineTime(at(8000.0).0 + STEP.0)).unwrap();
        assert!(
            (right_after.x - 0.8).abs() < 0.02,
            "should be there already: {}",
            right_after.x
        );
    }

    #[test]
    fn a_click_bounces_the_cursor_and_then_it_recovers() {
        let track = CursorTrack {
            samples: vec![
                moving(0.0, 0.5, 0.5),
                sample(1000.0, 0.5, 0.5, Interaction::Click, CursorShape::Arrow),
            ],
        };
        let mut project = project(5);
        project.cursor.click_bounce = 3.5;
        project.cursor.click_bounce_ms = 350.0;
        let path = PointerPath::build(&project, Some(&track));
        assert_eq!(path.sample(at(500.0)).unwrap().scale, 1.0);
        let middle = path.sample(at(1175.0)).unwrap().scale;
        // sin(0.5π) × 0.08 × 3.5 = 0.28, so the limit of 0.72 is exactly reached.
        assert!((middle - 0.72).abs() < 1e-3, "{middle}");
        assert_eq!(path.sample(at(2000.0)).unwrap().scale, 1.0);
    }

    #[test]
    fn a_click_in_cut_footage_never_happens() {
        let track = CursorTrack {
            samples: vec![
                moving(0.0, 0.5, 0.5),
                sample(4000.0, 0.5, 0.5, Interaction::Click, CursorShape::Arrow),
                moving(9000.0, 0.5, 0.5),
            ],
        };
        let mut project = project(10);
        project
            .timeline
            .apply(&Edit::TrimClip {
                id: "clip-1".into(),
                edge: reel_core::edit::Edge::End,
                to: at(2000.0),
            })
            .unwrap();
        let path = PointerPath::build(&project, Some(&track));
        assert!(path.clicks.is_empty());
    }

    #[test]
    fn a_clicks_time_goes_through_the_clip() {
        // Clicks are timed on the recording's clock. Here the clip plays at 2×, so a click at 4 s in
        // the recording lands at 2 s on the timeline.
        let track = CursorTrack {
            samples: vec![
                moving(0.0, 0.5, 0.5),
                sample(4000.0, 0.5, 0.5, Interaction::Click, CursorShape::Arrow),
            ],
        };
        let mut project = project(10);
        project
            .timeline
            .apply(&Edit::SetClipSpeed {
                id: "clip-1".into(),
                speed: 2.0,
            })
            .unwrap();
        let path = PointerPath::build(&project, Some(&track));
        assert_eq!(path.clicks[0].time, at(2000.0));
    }

    #[test]
    fn the_shape_ignores_click_samples() {
        let track = CursorTrack {
            samples: vec![
                sample(0.0, 0.5, 0.5, Interaction::Move, CursorShape::Text),
                sample(100.0, 0.5, 0.5, Interaction::Click, CursorShape::Arrow),
            ],
        };
        let path = PointerPath::build(&project(5), Some(&track));
        assert_eq!(path.sample(at(200.0)).unwrap().shape, CursorShape::Text);
    }

    #[test]
    fn outside_the_crop_the_cursor_is_hidden() {
        let track = CursorTrack {
            samples: vec![moving(0.0, 0.1, 0.1)],
        };
        let mut project = project(5);
        project.look.crop = reel_core::project::Crop {
            x: 0.5,
            y: 0.5,
            width: 0.5,
            height: 0.5,
        };
        let path = PointerPath::build(&project, Some(&track));
        assert!(!path.sample(at(1000.0)).unwrap().visible);
    }

    #[test]
    fn click_effects_start_after_half_a_bounce_and_finish_on_time() {
        let track = CursorTrack {
            samples: vec![
                moving(0.0, 0.5, 0.5),
                sample(1000.0, 0.5, 0.5, Interaction::Click, CursorShape::Arrow),
            ],
        };
        let mut project = project(5);
        project.cursor.click_effect = ClickEffect::Echo;
        project.cursor.click_bounce_ms = 200.0;
        project.cursor.click_effect_ms = 600.0;
        let path = PointerPath::build(&project, Some(&track));
        assert!(
            path.rings_at(at(1050.0), 60.0).is_empty(),
            "not started yet"
        );
        assert_eq!(path.rings_at(at(1200.0), 60.0).len(), 3, "echo draws three");
        assert!(path.rings_at(at(1800.0), 60.0).is_empty(), "finished");
    }

    #[test]
    fn click_effects_match_the_documented_sizes() {
        let click = Click {
            time: at(0.0),
            x: 0.5,
            y: 0.5,
        };
        let ripple = click_rings(&click, ClickEffect::Ripple, 0.5, 100.0, 1.0, 1.0);
        assert!((ripple[0].radius - (1.0 - 0.125) * 195.0).abs() < 1e-9);
        assert!((ripple[0].alpha - 0.125 * 0.6).abs() < 1e-9);
        let spotlight = click_rings(&click, ClickEffect::Spotlight, 1.0, 100.0, 1.0, 1.0);
        assert!((spotlight[0].radius - 55.0).abs() < 1e-9);
        assert!((spotlight[0].alpha - 0.28).abs() < 1e-9);
        let echo = click_rings(&click, ClickEffect::Echo, 1.0, 100.0, 1.0, 0.5);
        assert_eq!(echo[2].stroke, 0.0, "the middle of an echo is a filled dot");
        assert!((echo[0].alpha - 0.36).abs() < 1e-9);
    }

    #[test]
    fn loop_mode_ends_where_it_began() {
        let track = CursorTrack {
            samples: vec![
                moving(0.0, 0.1, 0.2),
                moving(3000.0, 0.9, 0.7),
                moving(5000.0, 0.9, 0.7),
            ],
        };
        let result = looped(&track);
        let last = result.samples.last().unwrap();
        assert_eq!((last.x, last.y), (0.1, 0.2));
        assert_eq!(last.time, SourceTime::from_ms(5000.0));
        assert!(
            result
                .samples
                .windows(2)
                .all(|pair| pair[0].time <= pair[1].time),
            "times stay in order"
        );
        let middle = result.position_at(SourceTime::from_ms(1000.0)).unwrap();
        assert!(middle.0 > 0.1, "the real motion still happens first");
    }

    #[test]
    fn a_track_too_short_to_loop_is_left_alone() {
        let track = CursorTrack {
            samples: vec![moving(0.0, 0.1, 0.2), moving(500.0, 0.9, 0.7)],
        };
        assert_eq!(looped(&track), track);
    }

    #[test]
    fn sway_leans_into_fast_motion_and_stops_when_it_stops() {
        let track = CursorTrack {
            samples: (0..=100)
                .map(|i| moving(f64::from(i) * 20.0, f64::from(i) * 0.008, 0.5))
                .chain([moving(2020.0, 0.8, 0.5)])
                .collect(),
        };
        let mut project = project(6);
        project.cursor.sway = 1.0;
        let path = PointerPath::build(&project, Some(&track));
        assert!(
            path.sample(at(1000.0)).unwrap().rotation > 0.01,
            "leaning right while moving right"
        );
        assert!(
            path.sample(at(5000.0)).unwrap().rotation.abs() < 0.001,
            "upright again once stopped"
        );
    }
}
