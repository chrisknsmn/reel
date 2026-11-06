//! Rendering a project to an MP4, through the same renderer the preview draws with.

use std::path::{Path, PathBuf};
use std::time::Instant;

use reel_core::project::{Project, SourceTrack};
use reel_core::time::{Micros, TimelineTime};
use reel_core::timeline::Clip;
use reel_media::bitrate;
use reel_media::macos::{Decoder, Encoder, EncoderSettings, VideoInfo, probe, read_pcm};
use reel_media::mix::{AddedAudio, CHANNELS, Mixer, Pcm, SAMPLE_RATE, source_audio};
use reel_motion::camera::{Camera, CameraPath};
use reel_motion::cursor::CursorTrack;
use reel_motion::layout::{self, Rect};

use crate::apple::TextureCache;
use crate::{
    FrameInput, Headless, Renderer, VideoSource, WORKING_FORMAT, read_pixels, readable_target,
};

#[derive(Debug)]
pub struct ExportError(pub String);

impl std::fmt::Display for ExportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ExportError {}

impl From<reel_media::macos::MediaError> for ExportError {
    fn from(error: reel_media::macos::MediaError) -> Self {
        Self(error.0)
    }
}

/// How an export went.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ExportReport {
    pub frames: u64,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub bitrate: u32,
    pub output_seconds: f64,
    pub elapsed_seconds: f64,
}

impl ExportReport {
    /// How many times faster than real time it ran.
    #[must_use]
    pub fn realtime_factor(&self) -> f64 {
        self.output_seconds / self.elapsed_seconds.max(1e-9)
    }
}

/// A project ready to render: everything that follows from its settings and its video's size.
pub struct Scene {
    pub project: Project,
    pub folder: Option<PathBuf>,
    pub video_path: PathBuf,
    pub info: VideoInfo,
    pub stage: (u32, u32),
    pub path: CameraPath,
}

impl Scene {
    /// Works out a project's output size and camera path.
    ///
    /// # Errors
    /// Returns an error if its video can't be read.
    pub fn new(project: Project, folder: Option<&Path>) -> Result<Self, ExportError> {
        let video_path = project.media.video.resolve(folder);
        if !video_path.exists() {
            return Err(ExportError(format!(
                "The project's video wasn't found: {}",
                video_path.display()
            )));
        }
        let info = probe(&video_path)?;
        let video = (f64::from(info.width), f64::from(info.height));
        let stage = layout::stage_size(&project.look.aspect_ratio, video, &project.look.crop);
        let stage_f = (f64::from(stage.0), f64::from(stage.1));
        let frame =
            layout::compute(stage_f, &project.look.padding, &project.look.crop, video).frame;
        let mask = Rect {
            x: frame.x / stage_f.0,
            y: frame.y / stage_f.1,
            width: frame.width / stage_f.0,
            height: frame.height / stage_f.1,
        };
        let cursor = load_cursor(&project, folder);
        let path = CameraPath::build(&project, &mask, cursor.as_ref());
        Ok(Self {
            project,
            folder: folder.map(Path::to_path_buf),
            video_path,
            info,
            stage,
            path,
        })
    }
}

fn load_cursor(project: &Project, folder: Option<&Path>) -> Option<CursorTrack> {
    let path = project.media.cursor_data.as_ref()?.resolve(folder);
    let text = std::fs::read_to_string(path).ok()?;
    CursorTrack::parse(&text).ok()
}

fn gpu() -> Result<Headless, ExportError> {
    Headless::new().ok_or_else(|| ExportError("No GPU was found.".to_string()))
}

/// Renders one frame at a timeline time, as RGBA. This is exactly what the preview shows there.
///
/// # Errors
/// Returns an error if the video can't be read or has nothing at that time.
pub fn render_still(scene: &Scene, time: TimelineTime) -> Result<Vec<u8>, ExportError> {
    let gpu = gpu()?;
    let mut renderer = Renderer::new(gpu.device.clone(), gpu.queue.clone(), WORKING_FORMAT);
    renderer.set_project_folder(scene.folder.as_deref());
    let cache = TextureCache::new(&gpu.device);
    let mut decoder = Decoder::open(&scene.video_path)?;
    let source = scene
        .project
        .timeline
        .to_source(time)
        .ok_or_else(|| ExportError(format!("Nothing is showing at {:.3} s.", time.secs())))?;
    let frame = decoder
        .frame_at(source)?
        .ok_or_else(|| ExportError("The video has no frame at that time.".to_string()))?;
    let size = (scene.info.width, scene.info.height);
    let (luma, chroma) = cache.wrap_nv12(&frame.buffer, size);
    let (luma_view, chroma_view) = (
        luma.texture
            .create_view(&wgpu::TextureViewDescriptor::default()),
        chroma
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default()),
    );
    let target = readable_target(&gpu.device, scene.stage, WORKING_FORMAT);
    renderer.render(
        &target,
        &FrameInput {
            look: &scene.project.look,
            video: VideoSource::Nv12 {
                luma: &luma_view,
                chroma: &chroma_view,
            },
            video_size: size,
            camera: scene.path.sample(time),
        },
    );
    let _ = gpu.device.poll(wgpu::PollType::wait_indefinitely());
    Ok(read_pixels(&gpu.device, &gpu.queue, &target))
}

/// Which output frame shows what. Gaps between clips are skipped (§41).
struct OutputClock<'a> {
    clips: &'a [Clip],
    fps: f64,
}

impl OutputClock<'_> {
    fn total_frames(&self) -> u64 {
        let total: Micros = self
            .clips
            .iter()
            .fold(Micros::ZERO, |sum, clip| sum + clip.duration);
        (total.secs() * self.fps).round() as u64
    }

    /// The clip and timeline time for an output frame.
    fn at(&self, frame: u64) -> Option<(&Clip, TimelineTime)> {
        let mut remaining = Micros((frame as f64 / self.fps * 1_000_000.0).round() as i64);
        for clip in self.clips {
            if remaining < clip.duration {
                return Some((clip, clip.start + remaining));
            }
            remaining = remaining - clip.duration;
        }
        // Rounding can leave the very last frame just past the end.
        self.clips.last().map(|clip| (clip, clip.end() - Micros(1)))
    }
}

fn load_mixer(scene: &Scene) -> Option<Mixer> {
    let folder = scene.folder.as_deref();
    let project = &scene.project;
    let video_audio: Option<Pcm> = if scene.info.has_audio {
        read_pcm(&scene.video_path).ok().flatten()
    } else {
        None
    };
    let load = |track: &SourceTrack| {
        track
            .file
            .as_ref()
            .and_then(|file| read_pcm(&file.resolve(folder)).ok().flatten())
    };
    let sources = source_audio(&project.media.source_tracks, video_audio.as_ref(), load);
    let added: Vec<AddedAudio> = project
        .timeline
        .audio
        .iter()
        .filter_map(|item| {
            let pcm = read_pcm(&item.file.resolve(folder)).ok().flatten()?;
            Some(AddedAudio {
                start: item.start,
                end: item.end,
                pcm,
                volume: item.volume as f32,
                boost: item.boost,
            })
        })
        .collect();
    (!sources.is_empty() || !added.is_empty()).then(|| Mixer::new(project, sources, added))
}

/// Writes a project to an MP4. `progress` is told how far along it is, and returning `false` cancels,
/// which leaves nothing behind. The finished file only appears at its final path at the very end.
///
/// # Errors
/// Returns an error if the export fails or is cancelled.
pub fn export_mp4(
    scene: &Scene,
    output: &Path,
    mut progress: impl FnMut(u64, u64) -> bool,
) -> Result<ExportReport, ExportError> {
    let started = Instant::now();
    let project = &scene.project;
    let settings = &project.export;
    let fps = settings.mp4_frame_rate.max(1);
    let (width, height) = scene.stage;
    let bitrate = bitrate::video_bitrate(width, height, fps, settings.quality, settings.encoding);
    let mut mixer = load_mixer(scene);

    let gpu = gpu()?;
    let mut renderer = Renderer::new(
        gpu.device.clone(),
        gpu.queue.clone(),
        wgpu::TextureFormat::Bgra8Unorm,
    );
    renderer.set_project_folder(scene.folder.as_deref());
    let cache = TextureCache::new(&gpu.device);
    let mut decoder = Decoder::open(&scene.video_path)?;

    let mut temporary = output.as_os_str().to_os_string();
    temporary.push(".tmp.mp4");
    let temporary = PathBuf::from(temporary);
    let mut encoder = Encoder::create(
        &temporary,
        EncoderSettings {
            width,
            height,
            fps,
            bitrate,
            keyframe_interval: bitrate::keyframe_interval(fps, settings.encoding),
            audio: mixer.is_some(),
        },
    )?;

    let clock = OutputClock {
        clips: &project.timeline.clips,
        fps: f64::from(fps),
    };
    let total = clock.total_frames();
    let video_size = (scene.info.width, scene.info.height);
    let mut current_clip: Option<&str> = None;
    let mut pcm = Vec::<f32>::new();
    let mut pcm_out = Vec::<i16>::new();

    for frame_index in 0..total {
        if !progress(frame_index, total) {
            encoder.cancel();
            let _ = std::fs::remove_file(&temporary);
            return Err(ExportError("The export was cancelled.".to_string()));
        }
        let Some((clip, time)) = clock.at(frame_index) else {
            break;
        };
        let source = clip.source_at(time);
        let Some(frame) = decoder.frame_at(source)? else {
            return Err(ExportError(format!(
                "The video has no frame at {:.3} s.",
                source.secs()
            )));
        };
        let (luma, chroma) = cache.wrap_nv12(&frame.buffer, video_size);
        let (luma_view, chroma_view) = (
            luma.texture
                .create_view(&wgpu::TextureViewDescriptor::default()),
            chroma
                .texture
                .create_view(&wgpu::TextureViewDescriptor::default()),
        );
        let buffer = encoder.next_buffer()?;
        let target = cache.wrap_target(&buffer, (width, height));
        renderer.render(
            &target.texture,
            &FrameInput {
                look: &project.look,
                video: VideoSource::Nv12 {
                    luma: &luma_view,
                    chroma: &chroma_view,
                },
                video_size,
                camera: scene.path.sample(time),
            },
        );
        // The encoder reads the buffer as soon as it's handed over, so the GPU has to be finished.
        let _ = gpu.device.poll(wgpu::PollType::wait_indefinitely());
        encoder.append_video(&buffer)?;

        if let Some(mixer) = &mut mixer {
            if current_clip != Some(clip.id.0.as_str()) {
                mixer.seek(time);
                current_clip = Some(clip.id.0.as_str());
            }
            // Exactly the right number of samples per frame, so sound can't drift over a long video.
            let start = audio_frame(frame_index, fps);
            let end = audio_frame(frame_index + 1, fps);
            pcm.resize((end - start) as usize * CHANNELS, 0.0);
            mixer.render(&mut pcm);
            pcm_out.clear();
            pcm_out.extend(
                pcm.iter()
                    .map(|sample| (sample.clamp(-1.0, 1.0) * 32767.0) as i16),
            );
            encoder.append_audio(&pcm_out)?;
        }
    }
    encoder.finish()?;
    std::fs::rename(&temporary, output).map_err(|error| {
        ExportError(format!(
            "The finished file couldn't be moved to {}: {error}",
            output.display()
        ))
    })?;

    Ok(ExportReport {
        frames: total,
        width,
        height,
        fps,
        bitrate,
        output_seconds: total as f64 / f64::from(fps),
        elapsed_seconds: started.elapsed().as_secs_f64(),
    })
}

fn audio_frame(video_frame: u64, fps: u32) -> u64 {
    video_frame * u64::from(SAMPLE_RATE) / u64::from(fps)
}

/// The camera at a time, for callers drawing their own frames.
#[must_use]
pub fn camera_at(scene: &Scene, time: TimelineTime) -> Camera {
    scene.path.sample(time)
}
