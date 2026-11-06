//! One-way import of `.recordly` and `.openscreen` projects, with all the legacy conversions in §45.

use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::project::{
    Background, CaptionAnimation, ClickEffect, Crop, Easing, EncodingMode, ExportFormat,
    ExportQuality, GifSize, MediaRef, Padding, Project, SourceTrack, SourceTrackKind,
    SpringFactors, TrackSetting, WebcamPosition,
};
use crate::time::{Micros, SourceTime, TimelineTime};
use crate::timeline::{
    Annotation, AnnotationContent, ArrowDirection, AudioItem, Caption, CaptionWord, Clip, ItemId,
    MIN_ITEM, Point, TextAlign, TextStyle, Zoom, ZoomMode,
};

#[derive(Debug)]
pub enum ImportError {
    Malformed(String),
    NotAProject,
}

impl std::fmt::Display for ImportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Malformed(reason) => write!(f, "This project file is damaged: {reason}"),
            Self::NotAProject => write!(f, "This isn't a Recordly project."),
        }
    }
}

impl std::error::Error for ImportError {}

/// The converted project, and a note of anything that couldn't come across.
#[derive(Debug)]
pub struct Imported {
    pub project: Project,
    pub warnings: Vec<String>,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct ImportOptions {
    /// How long the recording really is. Needed for accurate results on old projects that have
    /// trim regions but no clips. Without it, the last edit's end is used.
    pub source_duration: Option<Micros>,
    /// Look next to the video for its cursor and audio files. Turned off in tests.
    pub find_sidecars: bool,
}

/// Reads a Recordly project.
///
/// # Errors
/// Returns why it can't be read. Odd values inside a readable project become warnings instead.
pub fn import(text: &str, options: ImportOptions) -> Result<Imported, ImportError> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let root: Value =
        serde_json::from_str(text).map_err(|error| ImportError::Malformed(error.to_string()))?;
    let video = root
        .get("videoPath")
        .and_then(Value::as_str)
        .ok_or(ImportError::NotAProject)?;
    let empty = Value::Object(serde_json::Map::new());
    let editor = root.get("editor").unwrap_or(&empty);

    let mut warnings = Vec::new();
    let mut project = Project {
        id: root
            .get("projectId")
            .and_then(Value::as_str)
            .map_or_else(new_id, str::to_string),
        ..Project::default()
    };
    project.media.video = MediaRef::absolute(video);

    read_look(editor, &mut project);
    read_zoom_motion(editor, &mut project);
    read_cursor(editor, &mut project);
    read_webcam(editor, &mut project);
    read_caption_style(editor, &mut project);
    read_export(editor, &mut project);

    read_clips(editor, &options, &mut project, &mut warnings);
    read_zooms(editor, &mut project, &mut warnings);
    read_annotations(editor, &mut project, &mut warnings);
    read_audio(editor, &mut project);
    read_captions(editor, &mut project);
    read_source_tracks(editor, &mut project);
    if options.find_sidecars {
        find_sidecars(Path::new(video), &mut project);
    }

    repair(&mut project, &mut warnings);
    project.normalize();
    Ok(Imported { project, warnings })
}

/// A random ID, used when a project doesn't have one yet.
#[must_use]
pub fn new_id() -> String {
    let part = || RandomState::new().build_hasher().finish();
    format!("{:016x}{:016x}", part(), part())
}

// ---- Small readers ----

fn number(value: &Value, key: &str) -> Option<f64> {
    value
        .get(key)
        .and_then(Value::as_f64)
        .filter(|n| n.is_finite())
}

fn boolean(value: &Value, key: &str) -> Option<bool> {
    value.get(key).and_then(Value::as_bool)
}

fn text<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(Value::as_str)
}

fn set_number(target: &mut f64, value: &Value, key: &str) {
    if let Some(found) = number(value, key) {
        *target = found;
    }
}

fn set_bool(target: &mut bool, value: &Value, key: &str) {
    if let Some(found) = boolean(value, key) {
        *target = found;
    }
}

fn set_text(target: &mut String, value: &Value, key: &str) {
    if let Some(found) = text(value, key) {
        *target = found.to_string();
    }
}

fn list<'a>(value: &'a Value, key: &str) -> &'a [Value] {
    value
        .get(key)
        .and_then(Value::as_array)
        .map_or(&[], Vec::as_slice)
}

fn crop(value: &Value) -> Option<Crop> {
    Some(Crop {
        x: number(value, "x")?,
        y: number(value, "y")?,
        width: number(value, "width")?,
        height: number(value, "height")?,
    })
}

fn easing(value: Option<&str>, fallback: Easing) -> Easing {
    match value {
        Some("recordly") => Easing::Reel,
        Some("glide") => Easing::Glide,
        Some("smooth") => Easing::Smooth,
        Some("snappy") => Easing::Snappy,
        Some("linear") => Easing::Linear,
        _ => fallback,
    }
}

// ---- Sections ----

fn background(wallpaper: &str) -> Background {
    let lower = wallpaper.to_ascii_lowercase();
    let is_video = [".mp4", ".mov", ".m4v", ".webm", ".mkv"]
        .iter()
        .any(|ext| lower.ends_with(ext));
    if wallpaper.starts_with('#') {
        Background::Color(wallpaper.to_string())
    } else if lower.contains("gradient(") {
        Background::Gradient(wallpaper.to_string())
    } else if lower.starts_with("data:") {
        Background::DataUrl(wallpaper.to_string())
    } else if is_video && !wallpaper.starts_with("/wallpapers/") {
        Background::Video(MediaRef::absolute(strip_file_url(wallpaper)))
    } else if wallpaper.starts_with("/wallpapers/") || wallpaper.starts_with("wallpapers/") {
        let name = wallpaper.rsplit('/').next().unwrap_or(wallpaper);
        Background::Bundled(name.to_string())
    } else {
        Background::Image(MediaRef::absolute(strip_file_url(wallpaper)))
    }
}

fn strip_file_url(path: &str) -> PathBuf {
    PathBuf::from(path.strip_prefix("file://").unwrap_or(path))
}

fn read_look(editor: &Value, project: &mut Project) {
    let look = &mut project.look;
    if let Some(wallpaper) = text(editor, "wallpaper") {
        look.background = background(wallpaper);
    }
    set_number(&mut look.shadow, editor, "shadowIntensity");
    // `showBlur` is the old on/off switch for background blur.
    if boolean(editor, "showBlur") == Some(true) {
        look.background_blur = 2.0;
    }
    set_number(&mut look.background_blur, editor, "backgroundBlur");
    set_number(&mut look.corner_radius, editor, "borderRadius");
    match editor.get("padding") {
        // A plain number is the old linked padding.
        Some(Value::Number(value)) => {
            look.padding = Padding::uniform(value.as_f64().unwrap_or(20.0));
        }
        Some(padding @ Value::Object(_)) => {
            set_number(&mut look.padding.top, padding, "top");
            set_number(&mut look.padding.bottom, padding, "bottom");
            set_number(&mut look.padding.left, padding, "left");
            set_number(&mut look.padding.right, padding, "right");
            set_bool(&mut look.padding.linked, padding, "linked");
        }
        _ => {}
    }
    if let Some(found) = editor.get("cropRegion").and_then(crop) {
        look.crop = found;
    }
    set_text(&mut look.aspect_ratio, editor, "aspectRatio");
}

fn read_zoom_motion(editor: &Value, project: &mut Project) {
    let motion = &mut project.zoom_motion;
    // `motionBlurEnabled` is the old on/off switch for zoom motion blur.
    if boolean(editor, "motionBlurEnabled") == Some(true) {
        motion.motion_blur = 0.35;
    }
    set_number(&mut motion.motion_blur, editor, "zoomMotionBlur");
    set_number(
        &mut motion.temporal_motion_blur,
        editor,
        "zoomTemporalMotionBlur",
    );
    if let Some(samples) = number(editor, "zoomMotionBlurSampleCount") {
        motion.motion_blur_samples = samples as u32;
    }
    set_number(
        &mut motion.motion_blur_shutter,
        editor,
        "zoomMotionBlurShutterFraction",
    );
    set_bool(&mut motion.connect_zooms, editor, "connectZooms");
    set_number(&mut motion.zoom_in_ms, editor, "zoomInDurationMs");
    set_number(&mut motion.zoom_in_overlap_ms, editor, "zoomInOverlapMs");
    set_number(&mut motion.zoom_out_ms, editor, "zoomOutDurationMs");
    set_number(&mut motion.connected_gap_ms, editor, "connectedZoomGapMs");
    set_number(&mut motion.connected_ms, editor, "connectedZoomDurationMs");
    motion.zoom_in_easing = easing(text(editor, "zoomInEasing"), motion.zoom_in_easing);
    motion.zoom_out_easing = easing(text(editor, "zoomOutEasing"), motion.zoom_out_easing);
    motion.connected_easing = easing(text(editor, "connectedZoomEasing"), motion.connected_easing);
    set_number(&mut motion.smoothness, editor, "zoomSmoothness");
    set_bool(&mut motion.classic, editor, "zoomClassicMode");
    read_spring(&mut motion.camera_spring, editor, "cameraSpring");
}

fn read_spring(spring: &mut SpringFactors, editor: &Value, prefix: &str) {
    set_number(
        &mut spring.stiffness,
        editor,
        &format!("{prefix}StiffnessMultiplier"),
    );
    set_number(
        &mut spring.damping,
        editor,
        &format!("{prefix}DampingMultiplier"),
    );
    set_number(&mut spring.mass, editor, &format!("{prefix}MassMultiplier"));
}

fn read_cursor(editor: &Value, project: &mut Project) {
    let cursor = &mut project.cursor;
    set_bool(&mut cursor.show, editor, "showCursor");
    set_bool(&mut cursor.looped, editor, "loopCursor");
    if let Some(style) = text(editor, "cursorStyle") {
        // `mono` is the old name.
        cursor.style = if style == "mono" {
            "tahoe-inverted".to_string()
        } else {
            style.to_string()
        };
    }
    // These are read as they are. Recordly snaps them to a preset on load, which loses custom values
    // (§58). Keeping them is one of Milestone 2's exit criteria.
    set_number(&mut cursor.size, editor, "cursorSize");
    set_number(&mut cursor.smoothing, editor, "cursorSmoothing");
    read_spring(&mut cursor.spring, editor, "cursorSpring");
    set_number(&mut cursor.motion_blur, editor, "cursorMotionBlur");
    cursor.click_effect = match text(editor, "cursorClickEffect") {
        Some("spotlight") => ClickEffect::Spotlight,
        Some("ripple") => ClickEffect::Ripple,
        // `burst` is the old name.
        Some("echo" | "burst") => ClickEffect::Echo,
        Some("none") => ClickEffect::None,
        _ => cursor.click_effect,
    };
    set_text(
        &mut cursor.click_effect_color,
        editor,
        "cursorClickEffectColor",
    );
    set_number(
        &mut cursor.click_effect_scale,
        editor,
        "cursorClickEffectScale",
    );
    set_number(
        &mut cursor.click_effect_opacity,
        editor,
        "cursorClickEffectOpacity",
    );
    set_number(
        &mut cursor.click_effect_ms,
        editor,
        "cursorClickEffectDurationMs",
    );
    set_number(&mut cursor.click_bounce, editor, "cursorClickBounce");
    set_number(
        &mut cursor.click_bounce_ms,
        editor,
        "cursorClickBounceDuration",
    );
    set_number(&mut cursor.sway, editor, "cursorSway");
}

fn webcam_position(name: &str) -> Option<WebcamPosition> {
    Some(match name {
        "top-left" => WebcamPosition::TopLeft,
        "top-center" => WebcamPosition::TopCenter,
        "top-right" => WebcamPosition::TopRight,
        "center-left" => WebcamPosition::CenterLeft,
        "center" => WebcamPosition::Center,
        "center-right" => WebcamPosition::CenterRight,
        "bottom-left" => WebcamPosition::BottomLeft,
        "bottom-center" => WebcamPosition::BottomCenter,
        "bottom-right" => WebcamPosition::BottomRight,
        "custom" => WebcamPosition::Custom,
        _ => return None,
    })
}

fn read_webcam(editor: &Value, project: &mut Project) {
    let Some(source) = editor.get("webcam") else {
        return;
    };
    let webcam = &mut project.webcam;
    set_bool(&mut webcam.enabled, source, "enabled");
    webcam.source = text(source, "sourcePath").map(MediaRef::absolute);
    if let Some(ms) = number(source, "timeOffsetMs") {
        webcam.offset = Micros::from_ms(ms);
    }
    set_bool(&mut webcam.mirror, source, "mirror");
    if let Some(found) = source.get("cropRegion").and_then(crop) {
        webcam.crop = found;
    }
    // `corner` is the old name for the position.
    let position = text(source, "positionPreset").or_else(|| text(source, "corner"));
    if let Some(found) = position.and_then(webcam_position) {
        webcam.position = found;
    }
    set_number(&mut webcam.custom_x, source, "positionX");
    set_number(&mut webcam.custom_y, source, "positionY");
    // `size` is the old single value for width and height.
    set_number(&mut webcam.width, source, "size");
    set_number(&mut webcam.height, source, "size");
    set_number(&mut webcam.width, source, "width");
    set_number(&mut webcam.height, source, "height");
    // `zoomScaleEffect` is the old name for this.
    set_bool(&mut webcam.react_to_zoom, source, "zoomScaleEffect");
    set_bool(&mut webcam.react_to_zoom, source, "reactToZoom");
    set_number(&mut webcam.corner_radius, source, "cornerRadius");
    set_number(&mut webcam.shadow, source, "shadow");
    set_number(&mut webcam.margin, source, "margin");
}

fn read_caption_style(editor: &Value, project: &mut Project) {
    let Some(source) = editor.get("autoCaptionSettings") else {
        return;
    };
    let style = &mut project.caption_style;
    set_bool(&mut style.show, source, "enabled");
    set_bool(&mut style.hover_to_add, source, "timelineQuickAdd");
    set_text(&mut style.language, source, "language");
    set_text(&mut style.font_family, source, "fontFamily");
    set_number(&mut style.font_size, source, "fontSize");
    set_number(&mut style.bottom_offset, source, "bottomOffset");
    set_number(&mut style.max_width, source, "maxWidth");
    if let Some(rows) = number(source, "maxRows") {
        style.max_rows = rows as u32;
    }
    style.animation = match text(source, "animationStyle") {
        Some("none") => CaptionAnimation::None,
        Some("rise") => CaptionAnimation::Rise,
        Some("pop") => CaptionAnimation::Pop,
        Some("fade") => CaptionAnimation::Fade,
        _ => style.animation,
    };
    set_number(&mut style.box_radius, source, "boxRadius");
    set_text(&mut style.text_color, source, "textColor");
    set_text(&mut style.inactive_text_color, source, "inactiveTextColor");
    set_number(&mut style.background_opacity, source, "backgroundOpacity");
}

fn read_export(editor: &Value, project: &mut Project) {
    let export = &mut project.export;
    export.format = match text(editor, "exportFormat") {
        Some("gif") => ExportFormat::Gif,
        Some("mp4") => ExportFormat::Mp4,
        _ => export.format,
    };
    export.quality = match text(editor, "exportQuality") {
        Some("medium") => ExportQuality::Medium,
        Some("good") => ExportQuality::Good,
        Some("source") => ExportQuality::Source,
        _ => export.quality,
    };
    export.encoding = match text(editor, "exportEncodingMode") {
        Some("fast") => EncodingMode::Fast,
        Some("quality") => EncodingMode::Quality,
        Some("balanced") => EncodingMode::Balanced,
        _ => export.encoding,
    };
    if let Some(rate) = number(editor, "mp4FrameRate") {
        export.mp4_frame_rate = rate as u32;
    }
    if let Some(rate) = number(editor, "gifFrameRate") {
        export.gif_frame_rate = rate as u32;
    }
    set_bool(&mut export.gif_loop, editor, "gifLoop");
    export.gif_size = match text(editor, "gifSizePreset") {
        Some("small") => GifSize::Small,
        Some("medium") => GifSize::Medium,
        Some("large") => GifSize::Large,
        Some("original") => GifSize::Original,
        _ => export.gif_size,
    };
}

fn span(item: &Value) -> Option<(f64, f64)> {
    let start = number(item, "startMs")?;
    let end = number(item, "endMs")?;
    (end >= start).then_some((start, end))
}

fn item_id(item: &Value, fallback: &str, index: usize) -> ItemId {
    ItemId(text(item, "id").map_or_else(|| format!("{fallback}-{}", index + 1), str::to_string))
}

fn read_clips(
    editor: &Value,
    options: &ImportOptions,
    project: &mut Project,
    warnings: &mut Vec<String>,
) {
    let by_clip = editor.get("sourceAudioTrackSettingsByClip");
    let mut clips: Vec<Clip> = Vec::new();
    for (index, item) in list(editor, "clipRegions").iter().enumerate() {
        let Some((start, end)) = span(item) else {
            warnings.push(format!(
                "Left out clip {}, which has no valid times.",
                index + 1
            ));
            continue;
        };
        let speed = number(item, "speed")
            .filter(|speed| *speed > 0.0)
            .unwrap_or(1.0);
        let id = item_id(item, "clip", index);
        // A Recordly clip's start is both where it sits and where its footage begins.
        clips.push(Clip {
            start: TimelineTime::from_ms(start),
            duration: Micros::from_ms(end - start),
            source_start: SourceTime::from_ms(start),
            speed,
            muted: boolean(item, "muted").unwrap_or(false),
            show_source_audio: boolean(item, "showSourceAudio").unwrap_or(false),
            track_settings: by_clip
                .and_then(|all| all.get(&id.0))
                .map_or_else(Vec::new, track_settings),
            id,
        });
    }

    let trims = list(editor, "trimRegions");
    if clips.is_empty() && !trims.is_empty() {
        // An old project: its clips are whatever the trims leave. Speed regions are cleared (§45).
        let mut removed: Vec<(f64, f64)> = trims.iter().filter_map(span).collect();
        removed.sort_by(|a, b| a.0.total_cmp(&b.0));
        let last_trim = removed.iter().map(|(_, end)| *end).fold(0.0, f64::max);
        let total = options.source_duration.map_or(last_trim, Micros::ms);
        if options.source_duration.is_none() {
            warnings.push(
                "The recording's length wasn't known, so footage after the last trim may be missing.".to_string(),
            );
        }
        let mut cursor = 0.0;
        for (start, end) in removed.into_iter().chain([(total, total)]) {
            if start > cursor {
                clips.push(new_clip(clips.len(), cursor, start));
            }
            cursor = cursor.max(end);
        }
        if !list(editor, "speedRegions").is_empty() {
            warnings.push("Left out the old speed regions, as Recordly itself does.".to_string());
        }
    } else if clips.is_empty() {
        let total = options
            .source_duration
            .map_or_else(|| latest_end(editor), Micros::ms);
        if total > 0.0 {
            clips.push(new_clip(0, 0.0, total));
        }
    }

    let duration = options.source_duration.unwrap_or_else(|| {
        let longest = clips
            .iter()
            .map(|clip| clip.source_end() - SourceTime::ZERO)
            .max();
        longest.unwrap_or_default()
    });
    project.timeline.clips = clips;
    project.timeline.source_duration = duration;
}

fn new_clip(index: usize, start: f64, end: f64) -> Clip {
    Clip {
        id: ItemId(format!("clip-{}", index + 1)),
        start: TimelineTime::from_ms(start),
        duration: Micros::from_ms(end - start),
        source_start: SourceTime::from_ms(start),
        speed: 1.0,
        muted: false,
        show_source_audio: false,
        track_settings: Vec::new(),
    }
}

/// The furthest time anything reaches, for a project with no clips or trims at all.
fn latest_end(editor: &Value) -> f64 {
    [
        "zoomRegions",
        "annotationRegions",
        "audioRegions",
        "autoCaptions",
        "speedRegions",
    ]
    .iter()
    .flat_map(|key| list(editor, key))
    .filter_map(|item| number(item, "endMs"))
    .fold(0.0, f64::max)
}

fn read_zooms(editor: &Value, project: &mut Project, warnings: &mut Vec<String>) {
    for (index, item) in list(editor, "zoomRegions").iter().enumerate() {
        let Some((start, end)) = span(item) else {
            warnings.push(format!(
                "Left out zoom {}, which has no valid times.",
                index + 1
            ));
            continue;
        };
        let focus = item.get("focus");
        project.timeline.zooms.push(Zoom {
            id: item_id(item, "zoom", index),
            start: TimelineTime::from_ms(start),
            end: TimelineTime::from_ms(end),
            // Recordly's depths run from 1 to 6.
            depth: number(item, "depth").map_or(2, |depth| (depth as i64 - 1).clamp(0, 5) as u8),
            focus: Point {
                x: focus
                    .and_then(|focus| number(focus, "cx"))
                    .unwrap_or(0.5)
                    .clamp(0.0, 1.0),
                y: focus
                    .and_then(|focus| number(focus, "cy"))
                    .unwrap_or(0.5)
                    .clamp(0.0, 1.0),
            },
            mode: if text(item, "mode") == Some("manual") {
                ZoomMode::Manual
            } else {
                ZoomMode::Auto
            },
        });
    }
}

fn arrow_direction(name: Option<&str>) -> ArrowDirection {
    match name {
        Some("up") => ArrowDirection::Up,
        Some("down") => ArrowDirection::Down,
        Some("left") => ArrowDirection::Left,
        Some("up-right") => ArrowDirection::UpRight,
        Some("up-left") => ArrowDirection::UpLeft,
        Some("down-right") => ArrowDirection::DownRight,
        Some("down-left") => ArrowDirection::DownLeft,
        _ => ArrowDirection::Right,
    }
}

fn text_style(style: Option<&Value>) -> TextStyle {
    let mut result = TextStyle::default();
    let Some(style) = style else { return result };
    set_text(&mut result.color, style, "color");
    set_text(&mut result.background, style, "backgroundColor");
    set_number(&mut result.font_size, style, "fontSize");
    set_text(&mut result.font_family, style, "fontFamily");
    result.bold = text(style, "fontWeight").map_or(result.bold, |weight| weight == "bold");
    result.italic = text(style, "fontStyle") == Some("italic");
    result.underline = text(style, "textDecoration") == Some("underline");
    result.align = match text(style, "textAlign") {
        Some("left") => TextAlign::Left,
        Some("right") => TextAlign::Right,
        _ => TextAlign::Center,
    };
    set_number(&mut result.corner_radius, style, "borderRadius");
    result
}

fn read_annotations(editor: &Value, project: &mut Project, warnings: &mut Vec<String>) {
    for (index, item) in list(editor, "annotationRegions").iter().enumerate() {
        let Some((start, end)) = span(item) else {
            warnings.push(format!(
                "Left out annotation {}, which has no valid times.",
                index + 1
            ));
            continue;
        };
        let content = match text(item, "type") {
            Some("image") => {
                let data = text(item, "imageContent")
                    .or_else(|| text(item, "content"))
                    .unwrap_or_default();
                AnnotationContent::Image {
                    data_url: data.to_string(),
                }
            }
            Some("figure") => {
                let figure = item.get("figureData");
                AnnotationContent::Arrow {
                    direction: arrow_direction(
                        figure.and_then(|figure| text(figure, "arrowDirection")),
                    ),
                    color: figure
                        .and_then(|figure| text(figure, "color"))
                        .unwrap_or("#2563EB")
                        .to_string(),
                    stroke_width: figure
                        .and_then(|figure| number(figure, "strokeWidth"))
                        .unwrap_or(4.0),
                }
            }
            Some("blur") => AnnotationContent::Blur {
                strength: number(item, "blurIntensity").unwrap_or(20.0),
                fill: text(item, "blurColor").map(str::to_string),
            },
            _ => AnnotationContent::Text {
                text: text(item, "textContent")
                    .or_else(|| text(item, "content"))
                    .unwrap_or_default()
                    .to_string(),
                style: text_style(item.get("style")),
            },
        };
        let position = item.get("position");
        let size = item.get("size");
        project.timeline.annotations.push(Annotation {
            id: item_id(item, "annotation", index),
            start: TimelineTime::from_ms(start),
            end: TimelineTime::from_ms(end),
            content,
            position: Point {
                x: position
                    .and_then(|position| number(position, "x"))
                    .unwrap_or(50.0),
                y: position
                    .and_then(|position| number(position, "y"))
                    .unwrap_or(50.0),
            },
            width: size.and_then(|size| number(size, "width")).unwrap_or(30.0),
            height: size.and_then(|size| number(size, "height")).unwrap_or(20.0),
            track: number(item, "trackIndex").map_or(0, |track| track as u32),
            layer: number(item, "zIndex").map_or(0, |layer| layer as i32),
        });
    }
}

fn read_audio(editor: &Value, project: &mut Project) {
    for (index, item) in list(editor, "audioRegions").iter().enumerate() {
        let (Some((start, end)), Some(path)) = (span(item), text(item, "audioPath")) else {
            continue;
        };
        project.timeline.audio.push(AudioItem {
            id: item_id(item, "audio", index),
            start: TimelineTime::from_ms(start),
            end: TimelineTime::from_ms(end),
            file: MediaRef::absolute(path),
            // Recordly doesn't record this, so there's no limit until the file is opened.
            file_duration: Micros::ZERO,
            volume: number(item, "volume").unwrap_or(1.0).clamp(0.0, 1.0),
            boost: boolean(item, "normalize").unwrap_or(false),
            track: number(item, "trackIndex").map_or(0, |track| track as u32),
        });
    }
}

fn read_captions(editor: &Value, project: &mut Project) {
    for (index, item) in list(editor, "autoCaptions").iter().enumerate() {
        let (Some((start, end)), Some(caption)) = (span(item), text(item, "text")) else {
            continue;
        };
        let words = list(item, "words")
            .iter()
            .filter_map(|word| {
                let (start, end) = span(word)?;
                Some(CaptionWord {
                    text: text(word, "text")?.to_string(),
                    start: TimelineTime::from_ms(start),
                    end: TimelineTime::from_ms(end),
                    leading_space: boolean(word, "leadingSpace").unwrap_or(false),
                })
            })
            .collect();
        project.timeline.captions.push(Caption {
            id: item_id(item, "caption", index),
            start: TimelineTime::from_ms(start),
            end: TimelineTime::from_ms(end),
            text: caption.to_string(),
            words,
        });
    }
}

fn track_kind(name: &str) -> Option<SourceTrackKind> {
    match name {
        "mixed" => Some(SourceTrackKind::Mixed),
        "system" => Some(SourceTrackKind::System),
        "mic" => Some(SourceTrackKind::Mic),
        _ => None,
    }
}

fn track_settings(settings: &Value) -> Vec<TrackSetting> {
    let mut result: Vec<TrackSetting> = settings
        .as_object()
        .into_iter()
        .flatten()
        .filter_map(|(name, value)| {
            Some(TrackSetting {
                kind: track_kind(name)?,
                volume: number(value, "volume").unwrap_or(1.0),
                boost: boolean(value, "normalize").unwrap_or(false),
            })
        })
        .collect();
    result.sort_by_key(|setting| setting.kind);
    result
}

fn read_source_tracks(editor: &Value, project: &mut Project) {
    let defaults = editor
        .get("defaultSourceAudioTrackSettings")
        .map_or_else(Vec::new, track_settings);
    project.media.source_tracks = defaults
        .into_iter()
        .map(|setting| SourceTrack {
            kind: setting.kind,
            file: None,
            volume: setting.volume,
            boost: setting.boost,
        })
        .collect();
}

/// Links the files Recordly leaves next to a video (§29). This is what brings a separately recorded
/// mic back. Recordly can lose it in its own editor, using only the system audio in the video (§58).
fn find_sidecars(video: &Path, project: &mut Project) {
    let beside = |suffix: &str| {
        let mut name = video.as_os_str().to_os_string();
        name.push(suffix);
        let path = PathBuf::from(name);
        path.exists().then(|| MediaRef::absolute(path))
    };
    project.media.cursor_data = beside(".cursor.json");

    let mut link = |kind: SourceTrackKind, file: Option<MediaRef>| {
        let Some(file) = file else { return };
        match project
            .media
            .source_tracks
            .iter_mut()
            .find(|track| track.kind == kind)
        {
            Some(track) => track.file = Some(file),
            None => project.media.source_tracks.push(SourceTrack {
                kind,
                file: Some(file),
                volume: 1.0,
                boost: false,
            }),
        }
    };
    let stem_beside = |suffix: &str| {
        let path = video.with_extension(suffix);
        path.exists().then(|| MediaRef::absolute(path))
    };
    link(
        SourceTrackKind::Mic,
        beside(".mic.m4a").or_else(|| stem_beside("mic.m4a")),
    );
    link(
        SourceTrackKind::System,
        beside(".system.m4a").or_else(|| stem_beside("system.m4a")),
    );
}

/// Recordly allows things Reel's timeline doesn't, so bring them within the rules, and say so.
fn repair(project: &mut Project, warnings: &mut Vec<String>) {
    let timeline = &mut project.timeline;

    timeline.clips.sort_by_key(|clip| clip.start);
    let before = timeline.clips.len();
    timeline.clips.retain(|clip| clip.duration >= MIN_ITEM);
    note(
        warnings,
        before - timeline.clips.len(),
        "clip",
        "shorter than 100 ms",
    );
    let mut cut = 0;
    let mut previous_end = None;
    for clip in &mut timeline.clips {
        if let Some(end) = previous_end
            && clip.start < end
        {
            let overlap = end - clip.start;
            clip.start = end;
            clip.source_start += overlap.scaled(clip.speed);
            clip.duration = clip.duration - overlap;
            cut += 1;
        }
        previous_end = Some(clip.end());
    }
    if cut > 0 {
        warnings.push(format!("Shortened {cut} overlapping clips."));
    }
    timeline.clips.retain(|clip| clip.duration >= MIN_ITEM);

    timeline.zooms.sort_by_key(|zoom| zoom.start);
    let before = timeline.zooms.len();
    timeline
        .zooms
        .retain(|zoom| zoom.end - zoom.start >= MIN_ITEM);
    note(
        warnings,
        before - timeline.zooms.len(),
        "zoom",
        "shorter than 100 ms",
    );
    let mut cut = 0;
    for index in 1..timeline.zooms.len() {
        let limit = timeline.zooms[index].start;
        if timeline.zooms[index - 1].end > limit {
            timeline.zooms[index - 1].end = limit;
            cut += 1;
        }
    }
    if cut > 0 {
        warnings.push(format!("Shortened {cut} overlapping zooms."));
    }
    timeline
        .zooms
        .retain(|zoom| zoom.end - zoom.start >= MIN_ITEM);

    timeline.captions.sort_by_key(|cue| cue.start);
    let mut cut = 0;
    for index in 1..timeline.captions.len() {
        let limit = timeline.captions[index].start;
        if timeline.captions[index - 1].end > limit {
            timeline.captions[index - 1].end = limit;
            cut += 1;
        }
    }
    if cut > 0 {
        warnings.push(format!("Shortened {cut} overlapping captions."));
    }

    timeline
        .annotations
        .retain(|item| item.end - item.start >= MIN_ITEM);
    timeline
        .audio
        .retain(|item| item.end - item.start >= MIN_ITEM);
    timeline.audio.sort_by_key(|item| (item.track, item.start));
    for index in 1..timeline.audio.len() {
        let (track, limit) = (timeline.audio[index].track, timeline.audio[index].start);
        if timeline.audio[index - 1].track == track && timeline.audio[index - 1].end > limit {
            timeline.audio[index - 1].end = limit;
        }
    }
    timeline
        .audio
        .retain(|item| item.end - item.start >= MIN_ITEM);
}

fn note(warnings: &mut Vec<String>, count: usize, kind: &str, why: &str) {
    if count > 0 {
        warnings.push(format!("Left out {count} {kind}(s) {why}."));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::format;
    use serde_json::json;

    fn import_value(value: &Value) -> Imported {
        import(&value.to_string(), ImportOptions::default()).unwrap()
    }

    fn with_editor(editor: Value) -> Imported {
        import_value(&json!({ "version": 1, "videoPath": "/v.mp4", "editor": editor }))
    }

    #[test]
    fn other_json_is_not_a_project() {
        assert!(matches!(
            import("{}", ImportOptions::default()),
            Err(ImportError::NotAProject)
        ));
        assert!(matches!(
            import("{", ImportOptions::default()),
            Err(ImportError::Malformed(_))
        ));
    }

    #[test]
    fn a_byte_order_mark_is_tolerated() {
        let text = format!("\u{feff}{}", json!({ "videoPath": "/v.mp4" }));
        assert!(import(&text, ImportOptions::default()).is_ok());
    }

    #[test]
    fn custom_cursor_values_survive() {
        // Recordly snaps these to a preset on load (§58), and Milestone 2's exit criteria need them.
        let project = with_editor(json!({
            "cursorSize": 4.35,
            "cursorClickBounce": 1.25,
            "cursorClickBounceDuration": 120,
            "cursorSpringStiffnessMultiplier": 2.2,
            "zoomInDurationMs": 333,
        }))
        .project;
        assert_eq!(project.cursor.size, 4.35);
        assert_eq!(project.cursor.click_bounce, 1.25);
        assert_eq!(project.cursor.click_bounce_ms, 120.0);
        assert_eq!(project.cursor.spring.stiffness, 2.2);
        assert_eq!(project.zoom_motion.zoom_in_ms, 333.0);
    }

    #[test]
    fn old_on_off_switches_become_their_values() {
        let project = with_editor(json!({ "motionBlurEnabled": true, "showBlur": true })).project;
        assert_eq!(project.zoom_motion.motion_blur, 0.35);
        assert_eq!(project.look.background_blur, 2.0);
    }

    #[test]
    fn numeric_padding_becomes_linked() {
        let padding = with_editor(json!({ "padding": 30 })).project.look.padding;
        assert_eq!(padding, Padding::uniform(30.0));
    }

    #[test]
    fn old_webcam_fields_are_converted() {
        let webcam = with_editor(json!({
            "webcam": { "zoomScaleEffect": false, "corner": "top-left", "size": 25 }
        }))
        .project
        .webcam;
        assert!(!webcam.react_to_zoom);
        assert_eq!(webcam.position, WebcamPosition::TopLeft);
        assert_eq!((webcam.width, webcam.height), (25.0, 25.0));
    }

    #[test]
    fn old_cursor_and_click_names_are_converted() {
        let cursor = with_editor(json!({ "cursorStyle": "mono", "cursorClickEffect": "burst" }))
            .project
            .cursor;
        assert_eq!(cursor.style, "tahoe-inverted");
        assert_eq!(cursor.click_effect, ClickEffect::Echo);
    }

    #[test]
    fn trims_with_no_clips_become_the_complement_and_speed_regions_are_cleared() {
        let imported = import(
            &json!({
                "videoPath": "/v.mp4",
                "editor": {
                    "trimRegions": [
                        { "id": "t2", "startMs": 7000, "endMs": 8000 },
                        { "id": "t1", "startMs": 2000, "endMs": 3000 },
                    ],
                    "speedRegions": [{ "id": "s", "startMs": 0, "endMs": 1000, "speed": 2 }],
                },
            })
            .to_string(),
            ImportOptions {
                source_duration: Some(Micros::from_secs(10)),
                find_sidecars: false,
            },
        )
        .unwrap();
        let clips = &imported.project.timeline.clips;
        let spans: Vec<_> = clips
            .iter()
            .map(|clip| (clip.start.ms(), clip.end().ms()))
            .collect();
        assert_eq!(spans, [(0.0, 2000.0), (3000.0, 7000.0), (8000.0, 10_000.0)]);
        assert!(clips.iter().all(|clip| clip.speed == 1.0));
        assert!(
            imported
                .warnings
                .iter()
                .any(|warning| warning.contains("speed"))
        );
    }

    #[test]
    fn a_clips_footage_matches_what_recordly_showed() {
        // Recordly: start is both timeline and source position, and source end is start + length × speed.
        let timeline = with_editor(json!({
            "clipRegions": [{ "id": "c", "startMs": 1000, "endMs": 3000, "speed": 2 }]
        }))
        .project
        .timeline;
        let clip = &timeline.clips[0];
        assert_eq!(clip.start, TimelineTime::from_ms(1000.0));
        assert_eq!(clip.source_start, SourceTime::from_ms(1000.0));
        assert_eq!(clip.source_end(), SourceTime::from_ms(5000.0));
        assert_eq!(
            timeline.to_source(TimelineTime::from_ms(2000.0)),
            Some(SourceTime::from_ms(3000.0))
        );
    }

    #[test]
    fn empty_captions_are_dropped() {
        let timeline = with_editor(json!({
            "autoCaptions": [
                { "id": "a", "startMs": 0, "endMs": 1000, "text": "  " },
                { "id": "b", "startMs": 1000, "endMs": 2000, "text": "hi" },
            ]
        }))
        .project
        .timeline;
        assert_eq!(timeline.captions.len(), 1);
    }

    #[test]
    fn annotation_types_all_convert() {
        let timeline = with_editor(json!({
            "annotationRegions": [
                { "id": "t", "startMs": 0, "endMs": 1000, "type": "text", "textContent": "hi",
                  "style": { "fontWeight": "normal", "fontStyle": "italic", "textAlign": "left" } },
                { "id": "i", "startMs": 0, "endMs": 1000, "type": "image", "imageContent": "data:x" },
                { "id": "f", "startMs": 0, "endMs": 1000, "type": "figure",
                  "figureData": { "arrowDirection": "up-left", "color": "#111111", "strokeWidth": 2 } },
                { "id": "b", "startMs": 0, "endMs": 1000, "type": "blur", "blurIntensity": 40, "blurColor": "#000000" },
            ]
        }))
        .project
        .timeline;
        let by_id = |id: &str| {
            timeline
                .annotations
                .iter()
                .find(|item| item.id.0 == id)
                .unwrap()
                .content
                .clone()
        };
        assert!(
            matches!(by_id("t"), AnnotationContent::Text { style, .. } if !style.bold && style.italic && style.align == TextAlign::Left)
        );
        assert!(
            matches!(by_id("i"), AnnotationContent::Image { data_url } if data_url == "data:x")
        );
        assert!(matches!(
            by_id("f"),
            AnnotationContent::Arrow {
                direction: ArrowDirection::UpLeft,
                ..
            }
        ));
        assert!(
            matches!(by_id("b"), AnnotationContent::Blur { strength, fill: Some(_) } if strength == 40.0)
        );
    }

    #[test]
    fn wallpapers_are_sorted_by_what_they_are() {
        assert!(matches!(background("#20303f"), Background::Color(_)));
        assert!(matches!(
            background("linear-gradient(90deg, #000, #fff)"),
            Background::Gradient(_)
        ));
        assert!(matches!(
            background("data:image/jpeg;base64,xx"),
            Background::DataUrl(_)
        ));
        assert!(
            matches!(background("/wallpapers/tahoe-light.jpg"), Background::Bundled(name) if name == "tahoe-light.jpg")
        );
        assert!(matches!(
            background("/Users/a/sky.mp4"),
            Background::Video(_)
        ));
        assert!(matches!(
            background("/Users/a/sky.jpg"),
            Background::Image(_)
        ));
    }

    #[test]
    fn each_clip_keeps_its_own_track_settings() {
        // Export must use each clip's own settings, where Recordly applies one set to everything (§58).
        let timeline = with_editor(json!({
            "clipRegions": [
                { "id": "a", "startMs": 0, "endMs": 1000, "speed": 1 },
                { "id": "b", "startMs": 1000, "endMs": 2000, "speed": 1 },
            ],
            "sourceAudioTrackSettingsByClip": {
                "b": { "mic": { "volume": 0.25, "normalize": true } }
            },
        }))
        .project
        .timeline;
        assert!(timeline.clips[0].track_settings.is_empty());
        assert_eq!(timeline.clips[1].track_settings[0].volume, 0.25);
        assert!(timeline.clips[1].track_settings[0].boost);
    }

    #[test]
    fn a_separately_recorded_mic_is_linked() {
        let dir = tempfile::tempdir().unwrap();
        let video = dir.path().join("r.mp4");
        std::fs::write(&video, b"x").unwrap();
        std::fs::write(dir.path().join("r.mp4.cursor.json"), b"{}").unwrap();
        std::fs::write(dir.path().join("r.mic.m4a"), b"x").unwrap();
        let project = import(
            &json!({ "videoPath": video, "editor": {} }).to_string(),
            ImportOptions {
                source_duration: Some(Micros::from_secs(1)),
                find_sidecars: true,
            },
        )
        .unwrap()
        .project;
        assert!(project.media.cursor_data.is_some());
        let mic = project
            .media
            .source_tracks
            .iter()
            .find(|track| track.kind == SourceTrackKind::Mic)
            .unwrap();
        assert!(mic.file.is_some());
    }

    #[test]
    fn things_reels_timeline_forbids_are_repaired_with_a_warning() {
        let imported = with_editor(json!({
            "clipRegions": [
                { "id": "a", "startMs": 0, "endMs": 2000, "speed": 1 },
                { "id": "b", "startMs": 1500, "endMs": 3000, "speed": 1 },
            ],
            "zoomRegions": [
                { "id": "z1", "startMs": 0, "endMs": 1500, "depth": 2 },
                { "id": "z2", "startMs": 1000, "endMs": 2000, "depth": 2 },
                { "id": "tiny", "startMs": 2500, "endMs": 2510, "depth": 2 },
            ],
        }));
        imported.project.timeline.check().unwrap();
        assert_eq!(imported.project.timeline.zooms.len(), 2);
        assert!(!imported.warnings.is_empty());
    }

    #[test]
    fn a_converted_project_round_trips_through_reel_without_changes() {
        let imported = with_editor(json!({
            "cursorSize": 4.35,
            "clipRegions": [{ "id": "a", "startMs": 0, "endMs": 2000, "speed": 2 }],
            "zoomRegions": [{ "id": "z", "startMs": 0, "endMs": 1500, "depth": 3 }],
        }));
        let text = format::to_string(&imported.project);
        let loaded = format::from_str(&text).unwrap();
        assert_eq!(loaded, imported.project);
        assert_eq!(format::to_string(&loaded), text);
    }
}
