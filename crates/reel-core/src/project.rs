//! The project: the recording's look, its motion settings and its timeline (§8 to §12, §45).

use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::time::Micros;
use crate::timeline::Timeline;

/// A file a project uses. Recordly keeps only absolute paths, so moving a project breaks it. This
/// keeps both, and looks next to the project first.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MediaRef {
    /// Relative to the project file's folder, using forward slashes on every OS.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relative: Option<String>,
    pub absolute: PathBuf,
}

impl MediaRef {
    #[must_use]
    pub fn absolute(path: impl Into<PathBuf>) -> Self {
        Self {
            relative: None,
            absolute: path.into(),
        }
    }

    /// Where the file is. Prefers the relative path if a file is really there.
    #[must_use]
    pub fn resolve(&self, project_folder: Option<&Path>) -> PathBuf {
        if let (Some(relative), Some(folder)) = (&self.relative, project_folder) {
            let candidate = folder.join(relative);
            if candidate.exists() {
                return candidate;
            }
        }
        self.absolute.clone()
    }

    /// Updates the relative path for a project saved in this folder.
    pub fn rebase(&mut self, project_folder: &Path) {
        self.relative = relative_path(project_folder, &self.absolute);
    }
}

/// The path from one folder to a file, if both are absolute and share a root.
fn relative_path(from_folder: &Path, to: &Path) -> Option<String> {
    let from: Vec<_> = from_folder.components().collect();
    let target: Vec<_> = to.components().collect();
    if !matches!(
        from.first(),
        Some(Component::RootDir | Component::Prefix(_))
    ) {
        return None;
    }
    if from.first() != target.first() {
        return None;
    }
    let shared = from.iter().zip(&target).take_while(|(a, b)| a == b).count();
    let mut parts: Vec<String> = vec!["..".to_string(); from.len() - shared];
    parts.extend(
        target[shared..]
            .iter()
            .map(|part| part.as_os_str().to_string_lossy().into_owned()),
    );
    (!parts.is_empty()).then(|| parts.join("/"))
}

/// What's behind the recording (§8).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum Background {
    Color(String),
    /// CSS gradient text, parsed by the renderer.
    Gradient(String),
    /// The name of an image that ships with the app.
    Bundled(String),
    /// A custom image, embedded in the project.
    DataUrl(String),
    Image(MediaRef),
    Video(MediaRef),
}

impl Default for Background {
    fn default() -> Self {
        // The plan's D8 keeps Recordly's wallpapers out until their origin is confirmed.
        Self::Color("#20303F".to_string())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Padding {
    pub top: f64,
    pub bottom: f64,
    pub left: f64,
    pub right: f64,
    /// All four sides move together.
    pub linked: bool,
}

impl Default for Padding {
    fn default() -> Self {
        Self {
            top: 20.0,
            bottom: 20.0,
            left: 20.0,
            right: 20.0,
            linked: true,
        }
    }
}

impl Padding {
    /// Sets a linked value on all sides.
    #[must_use]
    pub fn uniform(value: f64) -> Self {
        Self {
            top: value,
            bottom: value,
            left: value,
            right: value,
            linked: true,
        }
    }
}

/// Part of a frame, from 0 to 1.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Crop {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Default for Crop {
    fn default() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            width: 1.0,
            height: 1.0,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Look {
    pub background: Background,
    /// 0 to 8.
    pub background_blur: f64,
    /// 0 to 1.
    pub shadow: f64,
    /// 0 to 200.
    pub corner_radius: f64,
    pub padding: Padding,
    pub crop: Crop,
    pub aspect_ratio: String,
}

impl Default for Look {
    fn default() -> Self {
        Self {
            background: Background::default(),
            background_blur: 0.0,
            shadow: 0.67,
            corner_radius: 12.5,
            padding: Padding::default(),
            crop: Crop::default(),
            aspect_ratio: "16:9".to_string(),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Easing {
    #[default]
    Reel,
    Glide,
    Smooth,
    Snappy,
    Linear,
}

/// Springs are multiples of the base spring (§32, §34).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct SpringFactors {
    pub stiffness: f64,
    pub damping: f64,
    pub mass: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ZoomMotion {
    pub motion_blur: f64,
    pub temporal_motion_blur: f64,
    pub motion_blur_samples: u32,
    pub motion_blur_shutter: f64,
    pub connect_zooms: bool,
    pub zoom_in_ms: f64,
    pub zoom_in_overlap_ms: f64,
    pub zoom_out_ms: f64,
    pub connected_gap_ms: f64,
    pub connected_ms: f64,
    pub zoom_in_easing: Easing,
    pub zoom_out_easing: Easing,
    pub connected_easing: Easing,
    pub smoothness: f64,
    pub classic: bool,
    pub camera_spring: SpringFactors,
}

impl Default for ZoomMotion {
    fn default() -> Self {
        Self {
            motion_blur: 0.35,
            temporal_motion_blur: 0.35,
            motion_blur_samples: 13,
            motion_blur_shutter: 0.94,
            connect_zooms: true,
            // The Focused motion preset (§19).
            zoom_in_ms: 200.0,
            zoom_in_overlap_ms: 500.0,
            zoom_out_ms: 200.0,
            connected_gap_ms: 1500.0,
            connected_ms: 1000.0,
            zoom_in_easing: Easing::Reel,
            zoom_out_easing: Easing::Reel,
            connected_easing: Easing::Glide,
            smoothness: 0.5,
            classic: false,
            camera_spring: SpringFactors {
                stiffness: 1.0,
                damping: 1.13,
                mass: 1.12,
            },
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClickEffect {
    #[default]
    None,
    Spotlight,
    Ripple,
    Echo,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CursorLook {
    pub show: bool,
    pub looped: bool,
    pub style: String,
    pub size: f64,
    pub smoothing: f64,
    pub spring: SpringFactors,
    pub motion_blur: f64,
    pub click_effect: ClickEffect,
    pub click_effect_color: String,
    pub click_effect_scale: f64,
    pub click_effect_opacity: f64,
    pub click_effect_ms: f64,
    pub click_bounce: f64,
    pub click_bounce_ms: f64,
    pub sway: f64,
}

impl Default for CursorLook {
    fn default() -> Self {
        Self {
            show: true,
            looped: false,
            style: "tahoe".to_string(),
            // The Focused motion preset (§19).
            size: 2.5,
            smoothing: 0.67,
            spring: SpringFactors {
                stiffness: 1.35,
                damping: 0.79,
                mass: 1.29,
            },
            motion_blur: 0.6,
            click_effect: ClickEffect::None,
            click_effect_color: "#2563EB".to_string(),
            click_effect_scale: 1.0,
            click_effect_opacity: 1.0,
            click_effect_ms: 600.0,
            click_bounce: 3.5,
            click_bounce_ms: 350.0,
            sway: 0.4,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WebcamPosition {
    TopLeft,
    TopCenter,
    TopRight,
    CenterLeft,
    Center,
    CenterRight,
    BottomLeft,
    BottomCenter,
    #[default]
    BottomRight,
    Custom,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Webcam {
    pub enabled: bool,
    pub source: Option<MediaRef>,
    /// How far the webcam's footage is ahead of the recording.
    pub offset: Micros,
    pub mirror: bool,
    pub crop: Crop,
    pub position: WebcamPosition,
    /// Where it sits when the position is custom, from 0 to 1.
    pub custom_x: f64,
    pub custom_y: f64,
    pub width: f64,
    pub height: f64,
    pub react_to_zoom: bool,
    pub corner_radius: f64,
    pub shadow: f64,
    pub margin: f64,
}

impl Default for Webcam {
    fn default() -> Self {
        Self {
            enabled: false,
            source: None,
            offset: Micros::ZERO,
            mirror: true,
            crop: Crop::default(),
            position: WebcamPosition::BottomRight,
            custom_x: 1.0,
            custom_y: 1.0,
            width: 40.0,
            height: 40.0,
            react_to_zoom: true,
            corner_radius: 90.0,
            shadow: 0.67,
            margin: 24.0,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptionAnimation {
    None,
    #[default]
    Fade,
    Rise,
    Pop,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CaptionStyle {
    pub show: bool,
    pub hover_to_add: bool,
    pub language: String,
    pub font_family: String,
    pub font_size: f64,
    pub bottom_offset: f64,
    pub max_width: f64,
    pub max_rows: u32,
    pub animation: CaptionAnimation,
    pub box_radius: f64,
    pub text_color: String,
    pub inactive_text_color: String,
    pub background_opacity: f64,
}

impl Default for CaptionStyle {
    fn default() -> Self {
        Self {
            show: false,
            hover_to_add: true,
            language: "auto".to_string(),
            font_family: "system-ui".to_string(),
            font_size: 30.0,
            bottom_offset: 3.0,
            max_width: 62.0,
            max_rows: 1,
            animation: CaptionAnimation::Fade,
            box_radius: 17.5,
            text_color: "#FFFFFF".to_string(),
            inactive_text_color: "#A3A3A3".to_string(),
            background_opacity: 0.9,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExportFormat {
    #[default]
    Mp4,
    Gif,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExportQuality {
    Medium,
    Good,
    #[default]
    Source,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EncodingMode {
    Fast,
    #[default]
    Balanced,
    Quality,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GifSize {
    Small,
    #[default]
    Medium,
    Large,
    Original,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ExportSettings {
    pub format: ExportFormat,
    pub quality: ExportQuality,
    pub encoding: EncodingMode,
    pub mp4_frame_rate: u32,
    pub gif_frame_rate: u32,
    pub gif_loop: bool,
    pub gif_size: GifSize,
}

impl Default for ExportSettings {
    fn default() -> Self {
        Self {
            format: ExportFormat::Mp4,
            quality: ExportQuality::Source,
            encoding: EncodingMode::Balanced,
            mp4_frame_rate: 30,
            gif_frame_rate: 15,
            gif_loop: true,
            gif_size: GifSize::Medium,
        }
    }
}

#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum SourceTrackKind {
    /// System audio and mic in one track.
    #[default]
    Mixed,
    System,
    Mic,
}

/// Volume for one source track.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct TrackSetting {
    pub kind: SourceTrackKind,
    pub volume: f64,
    /// A ×1.35 boost, which Recordly labels "Normalize" (BUILD_PLAN 7.6).
    #[serde(default)]
    pub boost: bool,
}

/// One audio track of the recording, kept separate so the editor can use both (BUILD_PLAN 4.5).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SourceTrack {
    pub kind: SourceTrackKind,
    /// Its own file, or the video's audio if there's none.
    #[serde(default)]
    pub file: Option<MediaRef>,
    #[serde(default = "full_volume")]
    pub volume: f64,
    #[serde(default)]
    pub boost: bool,
}

fn full_volume() -> f64 {
    1.0
}

/// The files a recording is made of.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Media {
    pub video: MediaRef,
    pub cursor_data: Option<MediaRef>,
    pub source_tracks: Vec<SourceTrack>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Project {
    /// Assigned when the project is created, and regenerated for a copy.
    pub id: String,
    pub media: Media,
    pub look: Look,
    pub zoom_motion: ZoomMotion,
    pub cursor: CursorLook,
    pub webcam: Webcam,
    pub caption_style: CaptionStyle,
    pub export: ExportSettings,
    pub timeline: Timeline,
}

impl Project {
    /// Keeps every setting within the limits in §45, and every list sorted, so a hand-edited or
    /// damaged file can't reach the renderer with values it wasn't built for.
    pub fn normalize(&mut self) {
        let look = &mut self.look;
        look.background_blur = look.background_blur.clamp(0.0, 8.0);
        look.shadow = look.shadow.clamp(0.0, 1.0);
        look.corner_radius = look.corner_radius.clamp(0.0, 200.0);

        let motion = &mut self.zoom_motion;
        motion.motion_blur = motion.motion_blur.clamp(0.0, 2.0);
        motion.temporal_motion_blur = motion.temporal_motion_blur.clamp(0.0, 2.0);
        motion.zoom_in_ms = motion.zoom_in_ms.clamp(60.0, 4000.0);
        motion.zoom_out_ms = motion.zoom_out_ms.clamp(60.0, 4000.0);
        motion.connected_ms = motion.connected_ms.clamp(60.0, 4000.0);
        motion.connected_gap_ms = motion.connected_gap_ms.clamp(0.0, 5000.0);
        clamp_spring(&mut motion.camera_spring);

        let cursor = &mut self.cursor;
        cursor.sway = cursor.sway.clamp(0.0, 2.0);
        clamp_spring(&mut cursor.spring);

        self.webcam.width = self.webcam.width.clamp(1.0, 200.0);
        self.webcam.height = self.webcam.height.clamp(1.0, 200.0);

        let timeline = &mut self.timeline;
        timeline.clips.sort_by_key(|item| item.start);
        timeline.zooms.sort_by_key(|item| item.start);
        timeline.annotations.sort_by_key(|item| item.start);
        timeline.audio.sort_by_key(|item| (item.track, item.start));
        timeline.captions.retain(|cue| !cue.text.trim().is_empty());
        timeline.captions.sort_by_key(|item| item.start);
    }

    /// Updates every file's relative path, ready for saving in this folder.
    pub fn rebase_media(&mut self, project_folder: &Path) {
        self.media.video.rebase(project_folder);
        if let Some(file) = &mut self.media.cursor_data {
            file.rebase(project_folder);
        }
        for track in &mut self.media.source_tracks {
            if let Some(file) = &mut track.file {
                file.rebase(project_folder);
            }
        }
        if let Some(file) = &mut self.webcam.source {
            file.rebase(project_folder);
        }
        if let Background::Image(file) | Background::Video(file) = &mut self.look.background {
            file.rebase(project_folder);
        }
        for item in &mut self.timeline.audio {
            item.file.rebase(project_folder);
        }
    }
}

fn clamp_spring(spring: &mut SpringFactors) {
    spring.stiffness = spring.stiffness.clamp(0.25, 3.0);
    spring.damping = spring.damping.clamp(0.25, 3.0);
    spring.mass = spring.mass.clamp(0.25, 3.0);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_relative_path_is_found_after_the_project_moves() {
        let dir = tempfile::tempdir().unwrap();
        let moved = dir.path().join("moved");
        std::fs::create_dir_all(moved.join("media")).unwrap();
        std::fs::write(moved.join("media/a.mp4"), b"x").unwrap();

        let reference = MediaRef {
            relative: Some("media/a.mp4".to_string()),
            absolute: PathBuf::from("/nowhere/a.mp4"),
        };
        assert_eq!(reference.resolve(Some(&moved)), moved.join("media/a.mp4"));
    }

    #[test]
    fn the_absolute_path_is_the_fallback() {
        let reference = MediaRef {
            relative: Some("missing.mp4".to_string()),
            absolute: PathBuf::from("/somewhere/a.mp4"),
        };
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            reference.resolve(Some(dir.path())),
            PathBuf::from("/somewhere/a.mp4")
        );
    }

    #[test]
    fn relative_paths_climb_out_of_the_project_folder() {
        let mut reference = MediaRef::absolute("/a/b/recordings/x.mp4");
        reference.rebase(Path::new("/a/b/Projects"));
        assert_eq!(reference.relative.as_deref(), Some("../recordings/x.mp4"));
    }

    #[test]
    fn normalizing_keeps_settings_within_their_limits() {
        let mut project = Project::default();
        project.look.background_blur = 99.0;
        project.cursor.sway = 5.0;
        project.zoom_motion.zoom_in_ms = 1.0;
        project.normalize();
        assert_eq!(project.look.background_blur, 8.0);
        assert_eq!(project.cursor.sway, 2.0);
        assert_eq!(project.zoom_motion.zoom_in_ms, 60.0);
    }
}
