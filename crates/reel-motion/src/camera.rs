//! The camera's whole path through a project, worked out once, ahead of time.
//!
//! Preview and export both sample this same path, at a fixed step on the timeline's own clock.
//! Recordly's preview runs its springs on wall-clock time while its export uses content time, so the
//! same project can move differently in each (§37). Here they can't.

use reel_core::project::Project;
use reel_core::time::{Micros, TimelineTime};
use reel_core::timeline::{Point, ZoomMode};

use crate::cursor::CursorTrack;
use crate::layout::Rect;
use crate::spring::{Spring, zoom_config};
use crate::zoom::{ZoomOptions, dominant};

/// The path is worked out 240 times a second.
pub const STEP: Micros = Micros(4167);

/// Where the camera is, as fractions of the stage, so it doesn't depend on the output size.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    pub scale: f64,
    pub x: f64,
    pub y: f64,
}

impl Camera {
    pub const IDENTITY: Self = Self {
        scale: 1.0,
        x: 0.0,
        y: 0.0,
    };
}

/// The camera at a target zoom and strength (§32). `mask` is the recording's area on the stage, as
/// fractions of the stage.
#[must_use]
pub fn target_camera(mask: &Rect, zoom_scale: f64, strength: f64, focus: Point) -> Camera {
    let progress = strength.clamp(0.0, 1.0);
    let focus_x = mask.x + focus.x * mask.width;
    let focus_y = mask.y + focus.y * mask.height;
    Camera {
        scale: 1.0 + (zoom_scale - 1.0) * progress,
        x: (0.5 - focus_x * zoom_scale) * progress,
        y: (0.5 - focus_y * zoom_scale) * progress,
    }
}

/// Keeps a zoomed view inside the frame: a focus can't get closer to the edge than half the view.
#[must_use]
pub fn clamp_focus(focus: Point, scale: f64) -> Point {
    let margin = 1.0 / (2.0 * scale);
    Point {
        x: focus.x.clamp(0.0, 1.0).clamp(margin, 1.0 - margin),
        y: focus.y.clamp(0.0, 1.0).clamp(margin, 1.0 - margin),
    }
}

/// Follows the cursor in auto mode, recentring when it leaves a safe zone (§32).
#[derive(Default)]
struct FollowState {
    active: bool,
    focus: Option<Point>,
    frozen: Option<Point>,
    reached_full: bool,
}

/// How far in from the view's edge the cursor has to go before the camera moves.
const SAFE_ZONE: f64 = 0.25;

impl FollowState {
    fn reset(&mut self) {
        *self = Self::default();
    }

    fn update(
        &mut self,
        cursor: Option<Point>,
        scale: f64,
        strength: f64,
        region_focus: Point,
    ) -> Point {
        let start = clamp_focus(region_focus, scale);
        if strength < 0.01 {
            self.reset();
            return start;
        }
        let Some(cursor) = cursor else {
            return self.focus.unwrap_or(start);
        };
        if strength >= 0.99 {
            self.reached_full = true;
        }
        // Zooming back out: hold still.
        if self.reached_full && strength < 0.99 {
            return self.frozen.or(self.focus).unwrap_or(start);
        }
        if !self.active {
            self.active = true;
            self.focus = Some(start);
            self.frozen = Some(start);
            return start;
        }
        let current = self.focus.unwrap_or(start);
        let half_span = 1.0 / (2.0 * scale);
        let inset = 2.0 * half_span * SAFE_ZONE;
        let leaves = |value: f64, center: f64| {
            value < center - half_span + inset || value > center + half_span - inset
        };
        let next = clamp_focus(
            Point {
                x: if leaves(cursor.x, current.x) {
                    cursor.x
                } else {
                    current.x
                },
                y: if leaves(cursor.y, current.y) {
                    cursor.y
                } else {
                    current.y
                },
            },
            scale,
        );
        self.focus = Some(next);
        self.frozen = Some(next);
        next
    }
}

/// The camera at every step of the timeline.
#[derive(Clone, Debug, PartialEq)]
pub struct CameraPath {
    samples: Vec<Camera>,
}

impl CameraPath {
    /// Works out the path. `mask` is the recording's area on the stage, as fractions of the stage.
    #[must_use]
    pub fn build(project: &Project, mask: &Rect, cursor: Option<&CursorTrack>) -> Self {
        let timeline = &project.timeline;
        let motion = &project.zoom_motion;
        let options = ZoomOptions {
            zoom_in_ms: motion.zoom_in_ms,
            zoom_out_ms: motion.zoom_out_ms,
            connect: motion.connect_zooms,
        };
        let config = zoom_config(motion.smoothness, motion.camera_spring);
        let use_spring = !motion.classic;
        let crop = &project.look.crop;

        let (mut scale, mut x, mut y) = (Spring::default(), Spring::default(), Spring::default());
        scale.snap(1.0);
        x.snap(0.0);
        y.snap(0.0);
        let mut follow = FollowState::default();
        let mut was_in_gap = false;

        let length = (timeline.duration().0 / STEP.0).max(0) as usize + 2;
        let mut samples = Vec::with_capacity(length);
        let step_ms = STEP.ms();
        for index in 0..length {
            let time = TimelineTime(index as i64 * STEP.0);
            let source = timeline.to_source(time);
            let in_gap = source.is_none();

            let mut target = Camera::IDENTITY;
            match dominant(&timeline.zooms, time, &options) {
                Some((zoom_index, strength)) if strength > 0.0 => {
                    let zoom = &timeline.zooms[zoom_index];
                    let zoom_scale = zoom.scale();
                    let focus = if !motion.classic && zoom.mode == ZoomMode::Auto {
                        // Cursor data is on the recording's clock, so it goes through the clip. The
                        // focus is relative to the cropped frame, so the crop is taken out too.
                        let at = source
                            .and(cursor)
                            .zip(source)
                            .and_then(|(track, source)| track.position_at(source));
                        let position = at.map(|(cx, cy)| Point {
                            x: (cx - crop.x) / crop.width.max(1e-6),
                            y: (cy - crop.y) / crop.height.max(1e-6),
                        });
                        follow.update(position, zoom_scale, strength, zoom.focus)
                    } else {
                        follow.reset();
                        clamp_focus(zoom.focus, zoom_scale)
                    };
                    target = target_camera(mask, zoom_scale, strength, focus);
                }
                _ => follow.reset(),
            }

            if use_spring && !was_in_gap {
                scale.step(target.scale, step_ms, &config);
                x.step(target.x, step_ms, &config);
                y.step(target.y, step_ms, &config);
            } else {
                // A cut in the timeline cuts the camera too, as does Classic mode.
                scale.snap(target.scale);
                x.snap(target.x);
                y.snap(target.y);
            }
            was_in_gap = in_gap;
            samples.push(Camera {
                scale: scale.value,
                x: x.value,
                y: y.value,
            });
        }
        Self { samples }
    }

    /// The camera at a time, between the steps either side of it.
    #[must_use]
    pub fn sample(&self, time: TimelineTime) -> Camera {
        let Some(last) = self.samples.last() else {
            return Camera::IDENTITY;
        };
        let position = time.0.max(0) as f64 / STEP.0 as f64;
        let index = position as usize;
        if index + 1 >= self.samples.len() {
            return *last;
        }
        let (a, b) = (self.samples[index], self.samples[index + 1]);
        let t = position - index as f64;
        Camera {
            scale: a.scale + (b.scale - a.scale) * t,
            x: a.x + (b.x - a.x) * t,
            y: a.y + (b.y - a.y) * t,
        }
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.samples.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cursor::{CursorSample, CursorShape, Interaction};
    use reel_core::edit::Edit;
    use reel_core::time::SourceTime;
    use reel_core::timeline::Timeline;

    const WHOLE: Rect = Rect {
        x: 0.0,
        y: 0.0,
        width: 1.0,
        height: 1.0,
    };

    fn project_with_zoom(start: f64, end: f64, mode: ZoomMode, focus: Point) -> Project {
        let mut project = Project {
            timeline: Timeline::for_recording(Micros::from_secs(30)),
            ..Project::default()
        };
        project
            .timeline
            .apply(&Edit::AddZoom {
                start: TimelineTime::from_ms(start),
                depth: 1,
                focus,
                mode,
            })
            .unwrap();
        project
            .timeline
            .apply(&Edit::ResizeZoom {
                id: "zoom-1".into(),
                edge: reel_core::edit::Edge::End,
                to: TimelineTime::from_ms(end),
            })
            .unwrap();
        project
    }

    #[test]
    fn the_transform_centres_the_focus() {
        // At full strength, the focus lands in the middle of the stage.
        let camera = target_camera(&WHOLE, 2.0, 1.0, Point { x: 0.25, y: 0.25 });
        assert_eq!(camera.scale, 2.0);
        assert!((camera.x + 0.25 * camera.scale - 0.5).abs() < 1e-12);
        assert!((camera.y + 0.25 * camera.scale - 0.5).abs() < 1e-12);
    }

    #[test]
    fn at_no_strength_the_camera_does_nothing() {
        assert_eq!(
            target_camera(&WHOLE, 2.0, 0.0, Point::CENTER),
            Camera::IDENTITY
        );
    }

    #[test]
    fn a_focus_near_the_edge_keeps_the_view_inside_the_frame() {
        let focus = clamp_focus(Point { x: 0.0, y: 1.0 }, 2.0);
        assert_eq!((focus.x, focus.y), (0.25, 0.75));
    }

    #[test]
    fn the_path_zooms_in_and_back_out() {
        let project = project_with_zoom(5000.0, 11_000.0, ZoomMode::Manual, Point::CENTER);
        let path = CameraPath::build(&project, &WHOLE, None);
        let at = |ms| path.sample(TimelineTime::from_ms(ms));
        assert_eq!(at(1000.0), Camera::IDENTITY);
        assert!(
            (at(9000.0).scale - 1.5).abs() < 1e-3,
            "settled at the zoom's scale"
        );
        assert!(
            (at(20_000.0).scale - 1.0).abs() < 1e-3,
            "back out afterwards"
        );
        let mid = at(4900.0).scale;
        assert!(mid > 1.0 && mid < 1.5, "the spring takes time: {mid}");
    }

    #[test]
    fn the_same_project_always_gives_the_same_path() {
        let project =
            project_with_zoom(5000.0, 11_000.0, ZoomMode::Manual, Point { x: 0.3, y: 0.7 });
        assert_eq!(
            CameraPath::build(&project, &WHOLE, None),
            CameraPath::build(&project, &WHOLE, None)
        );
    }

    #[test]
    fn classic_mode_has_no_spring() {
        let mut project = project_with_zoom(5000.0, 11_000.0, ZoomMode::Manual, Point::CENTER);
        project.zoom_motion.classic = true;
        let classic = CameraPath::build(&project, &WHOLE, None);
        project.zoom_motion.classic = false;
        let sprung = CameraPath::build(&project, &WHOLE, None);
        let time = TimelineTime::from_ms(4800.0);
        assert!(
            classic.sample(time).scale > sprung.sample(time).scale,
            "the spring lags behind the target"
        );
    }

    #[test]
    fn a_gap_in_the_timeline_cuts_the_camera() {
        let mut project = project_with_zoom(1000.0, 2500.0, ZoomMode::Manual, Point::CENTER);
        project
            .timeline
            .apply(&Edit::SplitClip {
                at: TimelineTime::from_ms(3000.0),
            })
            .unwrap();
        project
            .timeline
            .apply(&Edit::TrimClip {
                id: "clip-1".into(),
                edge: reel_core::edit::Edge::End,
                to: TimelineTime::from_ms(3000.0),
            })
            .unwrap();
        let second = project.timeline.clips[1].id.clone();
        project
            .timeline
            .apply(&Edit::MoveClip {
                id: second,
                to: TimelineTime::from_ms(8000.0),
            })
            .unwrap();
        let path = CameraPath::build(&project, &WHOLE, None);
        // Whatever was happening, the first frame after the gap is exactly on target.
        assert_eq!(path.sample(TimelineTime::from_ms(8010.0)), Camera::IDENTITY);
    }

    fn cursor_at(x: f64, y: f64) -> CursorTrack {
        CursorTrack {
            samples: vec![CursorSample {
                time: SourceTime::ZERO,
                x,
                y,
                interaction: Interaction::Move,
                shape: CursorShape::Arrow,
            }],
        }
    }

    #[test]
    fn auto_mode_follows_the_cursor_and_manual_mode_doesnt() {
        let cursor = cursor_at(0.8, 0.8);
        let camera_at = |mode| {
            let project = project_with_zoom(5000.0, 15_000.0, mode, Point::CENTER);
            let path = CameraPath::build(&project, &WHOLE, Some(&cursor));
            path.sample(TimelineTime::from_ms(10_000.0))
        };
        let (auto, manual) = (camera_at(ZoomMode::Auto), camera_at(ZoomMode::Manual));
        assert!(
            auto.x < manual.x - 0.05,
            "auto should pan toward the cursor: {auto:?} vs {manual:?}"
        );
        assert!(auto.y < manual.y - 0.05);
    }

    #[test]
    fn a_cursor_inside_the_safe_zone_doesnt_move_the_camera() {
        let cursor = cursor_at(0.55, 0.45);
        let auto = {
            let project = project_with_zoom(5000.0, 15_000.0, ZoomMode::Auto, Point::CENTER);
            CameraPath::build(&project, &WHOLE, Some(&cursor))
                .sample(TimelineTime::from_ms(10_000.0))
        };
        let manual = {
            let project = project_with_zoom(5000.0, 15_000.0, ZoomMode::Manual, Point::CENTER);
            CameraPath::build(&project, &WHOLE, None).sample(TimelineTime::from_ms(10_000.0))
        };
        assert!((auto.x - manual.x).abs() < 1e-6);
    }

    #[test]
    fn cursor_data_goes_through_the_clip_not_the_timeline() {
        // §58: Recordly compares source times with timeline times. Move the footage, and the camera
        // should still follow the cursor as it was at that point in the recording.
        let track = CursorTrack {
            samples: vec![
                CursorSample {
                    time: SourceTime::ZERO,
                    x: 0.9,
                    y: 0.9,
                    interaction: Interaction::Move,
                    shape: CursorShape::Arrow,
                },
                CursorSample {
                    time: SourceTime::from_ms(9999.0),
                    x: 0.9,
                    y: 0.9,
                    interaction: Interaction::Move,
                    shape: CursorShape::Arrow,
                },
                CursorSample {
                    time: SourceTime::from_ms(10_000.0),
                    x: 0.1,
                    y: 0.1,
                    interaction: Interaction::Move,
                    shape: CursorShape::Arrow,
                },
                CursorSample {
                    time: SourceTime::from_ms(30_000.0),
                    x: 0.1,
                    y: 0.1,
                    interaction: Interaction::Move,
                    shape: CursorShape::Arrow,
                },
            ],
        };
        let mut project = Project {
            timeline: Timeline::for_recording(Micros::from_secs(30)),
            ..Project::default()
        };
        // Show footage from 15 s onward at the start of the timeline: the cursor is at 0.1 throughout.
        project.timeline.clips[0].source_start = SourceTime::from_ms(15_000.0);
        project.timeline.clips[0].duration = Micros::from_secs(15);
        project
            .timeline
            .apply(&Edit::AddZoom {
                start: TimelineTime::from_ms(1000.0),
                depth: 1,
                focus: Point::CENTER,
                mode: ZoomMode::Auto,
            })
            .unwrap();
        project
            .timeline
            .apply(&Edit::ResizeZoom {
                id: "zoom-1".into(),
                edge: reel_core::edit::Edge::End,
                to: TimelineTime::from_ms(9000.0),
            })
            .unwrap();
        let path = CameraPath::build(&project, &WHOLE, Some(&track));
        let camera = path.sample(TimelineTime::from_ms(5000.0));
        // Panned toward 0.1, so the frame moves right: x is above the centred value of -0.25.
        assert!(
            camera.x > -0.2,
            "should follow the footage's cursor, got {camera:?}"
        );
    }
}
