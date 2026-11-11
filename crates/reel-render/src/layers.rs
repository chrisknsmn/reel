//! Everything drawn over the video at a moment: cursor, click effects, annotations, webcam, captions.

use std::sync::atomic::{AtomicU64, Ordering};

use reel_core::project::{CaptionStyle, CursorLook, Project, Webcam, ZoomMotion};
use reel_core::time::{Micros, TimelineTime};
use reel_core::timeline::{Annotation, Caption};
use reel_motion::camera::{Camera, CameraPath};
use reel_motion::cursor::CursorTrack;
use reel_motion::layout::Rect;
use reel_motion::pointer::{PointerFrame, PointerPath};

use crate::VideoSource;

/// A project's layers, worked out ahead of time. Preview and export both read from this.
pub struct SceneLayers {
    /// Different for each build, so anything worked out from these layers knows when it's stale.
    pub revision: u64,
    pub camera: CameraPath,
    pub pointer: PointerPath,
    pub cursor: CursorLook,
    pub motion: ZoomMotion,
    pub annotations: Vec<Annotation>,
    pub webcam: Webcam,
    pub captions: Vec<Caption>,
    pub caption_style: CaptionStyle,
}

/// How many copies of the cursor make up its motion blur.
const POINTER_TRAIL: usize = 7;

impl SceneLayers {
    /// Works out a project's paths. `mask` is the recording's area, as fractions of the stage.
    #[must_use]
    pub fn build(project: &Project, mask: &Rect, track: Option<&CursorTrack>) -> Self {
        static REVISION: AtomicU64 = AtomicU64::new(1);
        Self {
            revision: REVISION.fetch_add(1, Ordering::Relaxed),
            camera: CameraPath::build(project, mask, track),
            pointer: PointerPath::build(project, track),
            cursor: project.cursor.clone(),
            motion: project.zoom_motion.clone(),
            annotations: project.timeline.annotations.clone(),
            webcam: project.webcam.clone(),
            captions: project.timeline.captions.clone(),
            caption_style: project.caption_style.clone(),
        }
    }

    /// What's on screen at a time. `webcam` is the webcam's current frame, if there is one.
    #[must_use]
    pub fn at<'a>(
        &'a self,
        time: TimelineTime,
        webcam: Option<(VideoSource<'a>, (u32, u32))>,
    ) -> FrameLayers<'a> {
        let camera = self.camera.sample(time);
        FrameLayers {
            time,
            source: self,
            pointer: self.pointer_trail(time),
            annotations: self
                .annotations
                .iter()
                .filter(|item| time >= item.start && time < item.end)
                .collect(),
            webcam: if self.webcam.enabled { webcam } else { None },
            camera_blur_from: self.camera_blur(time, camera),
            zoom: camera.scale,
            background_video: None,
        }
    }

    /// Where the camera was a moment ago, if it has moved enough to blur.
    ///
    /// The blur is `delta × amount × 4` of a 60 fps frame's motion. As a time, that's the same at
    /// any frame rate, and the same in preview and export.
    fn camera_blur(&self, time: TimelineTime, camera: Camera) -> Option<(Camera, u32)> {
        let amount = self.motion.motion_blur;
        if amount <= 0.0 {
            return None;
        }
        let lookback = Micros((amount * 4.0 / 60.0 * 1_000_000.0) as i64);
        let from = self
            .camera
            .sample(TimelineTime((time.0 - lookback.0).max(0)));
        let moved = (from.scale - camera.scale).abs()
            + (from.x - camera.x).abs()
            + (from.y - camera.y).abs();
        (moved > 0.0005).then_some((from, self.motion.motion_blur_samples.clamp(2, 32)))
    }

    /// The cursor now, and where it's just been, for motion blur.
    fn pointer_trail(&self, time: TimelineTime) -> Vec<PointerFrame> {
        let Some(now) = self.pointer.sample(time) else {
            return Vec::new();
        };
        let amount = self.cursor.motion_blur;
        if amount <= 0.0 || !now.visible {
            return vec![now];
        }
        let lookback = Micros((amount * 2.0 / 60.0 * 1_000_000.0) as i64);
        let oldest = self
            .pointer
            .sample(TimelineTime((time.0 - lookback.0).max(0)))
            .unwrap_or(now);
        // Only blur if it has moved by more than a pixel or so.
        let moved = (oldest.x - now.x).hypot(oldest.y - now.y);
        if moved < 0.001 {
            return vec![now];
        }
        (0..POINTER_TRAIL)
            .filter_map(|step| {
                let back = lookback.scaled(1.0 - step as f64 / (POINTER_TRAIL - 1) as f64);
                self.pointer.sample(TimelineTime((time.0 - back.0).max(0)))
            })
            .collect()
    }
}

/// What's on screen at one moment.
pub struct FrameLayers<'a> {
    pub time: TimelineTime,
    pub source: &'a SceneLayers,
    /// The cursor's motion blur copies, oldest first.
    pub pointer: Vec<PointerFrame>,
    pub annotations: Vec<&'a Annotation>,
    pub webcam: Option<(VideoSource<'a>, (u32, u32))>,
    pub camera_blur_from: Option<(Camera, u32)>,
    /// The camera's scale, which a webcam set to react to zoom shrinks with.
    pub zoom: f64,
    /// The current frame of a video background.
    pub background_video: Option<(VideoSource<'a>, (u32, u32))>,
}
