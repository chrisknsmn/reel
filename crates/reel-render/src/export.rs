//! Rendering a project to an MP4, through the same renderer the preview draws with.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use reel_core::project::{Background, GifSize, Project, SourceTrack};
use reel_core::time::{Micros, SourceTime, TimelineTime};
use reel_core::timeline::Clip;
use reel_media::bitrate;
use reel_media::macos::{Decoder, Encoder, EncoderSettings, VideoInfo, probe, read_pcm};
use reel_media::mix::{AddedAudio, CHANNELS, Mixer, Pcm, SAMPLE_RATE, source_audio};
use reel_motion::camera::Camera;
use reel_motion::cursor::CursorTrack;
use reel_motion::layout::{self, Rect};

use crate::apple::{TextureCache, WrappedTexture};
use crate::layers::SceneLayers;
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
    pub layers: SceneLayers,
    /// The webcam's footage, if it's turned on and can be read.
    pub webcam: Option<WebcamFile>,
    /// A video background, if the look has one that can be read.
    pub background_video: Option<VideoFile>,
    /// The recorded cursor, kept so that an edit doesn't have to read it again.
    cursor: Option<CursorTrack>,
}

/// A webcam recording that's ready to decode.
#[derive(Clone, Debug)]
pub struct WebcamFile {
    pub path: PathBuf,
    pub size: (u32, u32),
}

/// A video background that's ready to decode.
#[derive(Clone, Debug, PartialEq)]
pub struct VideoFile {
    pub path: PathBuf,
    pub size: (u32, u32),
    pub duration: Micros,
}

impl VideoFile {
    /// Which of its moments shows at a time on the timeline. It loops (§31).
    #[must_use]
    pub fn time_at(&self, time: TimelineTime) -> SourceTime {
        SourceTime(time.0.max(0).rem_euclid(self.duration.0.max(1)))
    }
}

fn load_background_video(project: &Project, folder: Option<&Path>) -> Option<VideoFile> {
    let Background::Video(media) = &project.look.background else {
        return None;
    };
    let path = media.resolve(folder);
    let info = probe(&path).ok()?;
    Some(VideoFile {
        path,
        size: (info.width, info.height),
        duration: info.duration,
    })
}

fn measure(
    project: &Project,
    info: &VideoInfo,
    cursor: Option<&CursorTrack>,
) -> ((u32, u32), SceneLayers) {
    let video = (f64::from(info.width), f64::from(info.height));
    let stage = layout::stage_size(&project.look.aspect_ratio, video, &project.look.crop);
    let stage_f = (f64::from(stage.0), f64::from(stage.1));
    let frame = layout::compute(stage_f, &project.look.padding, &project.look.crop, video).frame;
    let mask = Rect {
        x: frame.x / stage_f.0,
        y: frame.y / stage_f.1,
        width: frame.width / stage_f.0,
        height: frame.height / stage_f.1,
    };
    (stage, SceneLayers::build(project, &mask, cursor))
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
        let cursor = load_cursor(&project, folder);
        let (stage, layers) = measure(&project, &info, cursor.as_ref());
        let webcam = load_webcam(&project, folder);
        let background_video = load_background_video(&project, folder);
        Ok(Self {
            project,
            folder: folder.map(Path::to_path_buf),
            video_path,
            info,
            stage,
            layers,
            webcam,
            background_video,
            cursor,
        })
    }

    /// The recorded cursor, if there is one.
    #[must_use]
    pub fn cursor_track(&self) -> Option<&CursorTrack> {
        self.cursor.as_ref()
    }

    /// The same video with an edited project. This skips reading files it already has, which makes
    /// it quick enough to do while a slider is moving. A changed webcam file does get read.
    #[must_use]
    pub fn with_project(&self, project: Project) -> Self {
        let (stage, layers) = measure(&project, &self.info, self.cursor.as_ref());
        let webcam = if project.webcam == self.project.webcam {
            self.webcam.clone()
        } else {
            load_webcam(&project, self.folder.as_deref())
        };
        let background_video = if project.look.background == self.project.look.background {
            self.background_video.clone()
        } else {
            load_background_video(&project, self.folder.as_deref())
        };
        Self {
            project,
            folder: self.folder.clone(),
            video_path: self.video_path.clone(),
            info: self.info,
            stage,
            layers,
            webcam,
            background_video,
            cursor: self.cursor.clone(),
        }
    }
}

fn load_cursor(project: &Project, folder: Option<&Path>) -> Option<CursorTrack> {
    let path = project.media.cursor_data.as_ref()?.resolve(folder);
    let text = std::fs::read_to_string(path).ok()?;
    CursorTrack::parse(&text).ok()
}

fn load_webcam(project: &Project, folder: Option<&Path>) -> Option<WebcamFile> {
    let settings = &project.webcam;
    if !settings.enabled {
        return None;
    }
    let path = settings.source.as_ref()?.resolve(folder);
    let info = probe(&path).ok()?;
    Some(WebcamFile {
        path,
        size: (info.width, info.height),
    })
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
    let mut webcam = WebcamReader::open(scene);
    let webcam_frame = webcam
        .as_mut()
        .and_then(|reader| reader.frame_at(&cache, source));
    let webcam_views = webcam_frame.as_ref().map(WebcamFrame::views);
    let webcam_source = webcam_views.as_ref().map(WebcamViews::source);
    let mut background = BackgroundReader::open(scene);
    let background_frame = background
        .as_mut()
        .and_then(|reader| reader.frame_at(&cache, time));
    let background_views = background_frame.as_ref().map(WebcamFrame::views);
    let background_source = background_views.as_ref().map(WebcamViews::source);
    let mut frame_layers = scene.layers.at(time, webcam_source);
    frame_layers.background_video = background_source;
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
            camera: scene.layers.camera.sample(time),
            layers: Some(frame_layers),
        },
    );
    let _ = gpu.device.poll(wgpu::PollType::wait_indefinitely());
    Ok(read_pixels(&gpu.device, &gpu.queue, &target))
}

/// The webcam's footage, read alongside the recording.
struct WebcamReader {
    decoder: Decoder,
    settings: reel_core::project::Webcam,
    size: (u32, u32),
}

struct WebcamFrame {
    luma: WrappedTexture,
    chroma: WrappedTexture,
    size: (u32, u32),
}

struct WebcamViews {
    luma: wgpu::TextureView,
    chroma: wgpu::TextureView,
    size: (u32, u32),
}

impl WebcamViews {
    fn source(&self) -> (VideoSource<'_>, (u32, u32)) {
        (
            VideoSource::Nv12 {
                luma: &self.luma,
                chroma: &self.chroma,
            },
            self.size,
        )
    }
}

impl WebcamFrame {
    fn views(&self) -> WebcamViews {
        let view = |plane: &WrappedTexture| {
            plane
                .texture
                .create_view(&wgpu::TextureViewDescriptor::default())
        };
        WebcamViews {
            luma: view(&self.luma),
            chroma: view(&self.chroma),
            size: self.size,
        }
    }
}

impl WebcamReader {
    fn open(scene: &Scene) -> Option<Self> {
        let file = scene.webcam.as_ref()?;
        Some(Self {
            decoder: Decoder::open(&file.path).ok()?,
            settings: scene.project.webcam.clone(),
            size: file.size,
        })
    }

    /// The webcam's frame for a moment in the recording.
    fn frame_at(&mut self, cache: &TextureCache, source: SourceTime) -> Option<WebcamFrame> {
        let time = reel_motion::webcam::webcam_time(&self.settings, source);
        let frame = self.decoder.frame_at(time).ok()??;
        let (luma, chroma) = cache.wrap_nv12(&frame.buffer, self.size);
        Some(WebcamFrame {
            luma,
            chroma,
            size: self.size,
        })
    }
}

/// A video background's frames.
struct BackgroundReader {
    decoder: Decoder,
    file: VideoFile,
}

impl BackgroundReader {
    fn open(scene: &Scene) -> Option<Self> {
        let file = scene.background_video.clone()?;
        Some(Self {
            decoder: Decoder::open(&file.path).ok()?,
            file,
        })
    }

    fn frame_at(&mut self, cache: &TextureCache, time: TimelineTime) -> Option<WebcamFrame> {
        let frame = self.decoder.frame_at(self.file.time_at(time)).ok()??;
        let (luma, chroma) = cache.wrap_nv12(&frame.buffer, self.file.size);
        Some(WebcamFrame {
            luma,
            chroma,
            size: self.file.size,
        })
    }
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

/// How long the encoder can refuse everything before the export gives up, rather than wait forever.
const ENCODER_STALL: Duration = Duration::from_secs(30);

/// Writes an export's sound, a video frame's worth at a time.
///
/// It's kept apart from the frames because it doesn't always go in step with them. AVAssetWriter
/// interleaves its tracks, so it can refuse a frame until it has more sound. Writing sound only after
/// each frame was taken used to mean each side waiting for the other, for good.
struct SoundWriter<'a> {
    mixer: Mixer,
    clock: &'a OutputClock<'a>,
    fps: u32,
    total: u64,
    /// The next video frame whose sound hasn't been written.
    next: u64,
    current_clip: Option<&'a str>,
    pcm: Vec<f32>,
    pcm_out: Vec<i16>,
}

impl SoundWriter<'_> {
    fn write_next(&mut self, encoder: &mut Encoder) -> Result<(), ExportError> {
        if let Some((clip, time)) = self.clock.at(self.next)
            && self.current_clip != Some(clip.id.0.as_str())
        {
            self.mixer.seek(time);
            self.current_clip = Some(clip.id.0.as_str());
        }
        // Exactly the right number of samples per frame, so sound can't drift over a long video.
        let start = audio_frame(self.next, self.fps);
        let end = audio_frame(self.next + 1, self.fps);
        self.pcm.resize((end - start) as usize * CHANNELS, 0.0);
        self.mixer.render(&mut self.pcm);
        self.pcm_out.clear();
        self.pcm_out.extend(
            self.pcm
                .iter()
                .map(|sample| (sample.clamp(-1.0, 1.0) * 32767.0) as i16),
        );
        encoder.append_audio(&self.pcm_out)?;
        self.next += 1;
        Ok(())
    }

    /// Writes sound if the encoder will take it. Returns whether it did.
    fn feed(&mut self, encoder: &mut Encoder) -> Result<bool, ExportError> {
        if self.next < self.total && encoder.audio_is_ready()? {
            self.write_next(encoder)?;
            if self.next == self.total {
                encoder.finish_audio();
            }
            return Ok(true);
        }
        Ok(false)
    }
}

/// Writes a project to an MP4. `progress` is told how far along it is, and returning `false` cancels,
/// which leaves nothing behind. The finished file only appears at its final path at the very end.
///
/// # Errors
/// Returns an error if the export fails or is cancelled.
pub fn export_mp4(
    scene: &Scene,
    output: &Path,
    progress: impl FnMut(u64, u64) -> bool,
) -> Result<ExportReport, ExportError> {
    let mut temporary = output.as_os_str().to_os_string();
    temporary.push(".tmp.mp4");
    let temporary = PathBuf::from(temporary);
    let result = write_mp4(scene, output, &temporary, progress);
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

fn write_mp4(
    scene: &Scene,
    output: &Path,
    temporary: &Path,
    mut progress: impl FnMut(u64, u64) -> bool,
) -> Result<ExportReport, ExportError> {
    let started = Instant::now();
    let project = &scene.project;
    let settings = &project.export;
    let fps = settings.mp4_frame_rate.max(1);
    // Everything on the frame is measured against the stage, so it all scales with this.
    let (width, height) = settings.quality.scaled_size(scene.stage);
    let bitrate = bitrate::video_bitrate(width, height, fps, settings.quality, settings.encoding);
    let mut mixer = load_mixer(scene);
    let has_sound = mixer.is_some();

    let gpu = gpu()?;
    let mut renderer = Renderer::new(
        gpu.device.clone(),
        gpu.queue.clone(),
        wgpu::TextureFormat::Bgra8Unorm,
    );
    renderer.set_project_folder(scene.folder.as_deref());
    let cache = TextureCache::new(&gpu.device);
    let mut decoder = Decoder::open(&scene.video_path)?;
    let mut webcam = WebcamReader::open(scene);
    let mut background = BackgroundReader::open(scene);

    let mut encoder = Encoder::create(
        temporary,
        EncoderSettings {
            width,
            height,
            fps,
            bitrate,
            keyframe_interval: bitrate::keyframe_interval(fps, settings.encoding),
            audio: has_sound,
        },
    )?;

    let clock = OutputClock {
        clips: &project.timeline.clips,
        fps: f64::from(fps),
    };
    let total = clock.total_frames();
    let video_size = (scene.info.width, scene.info.height);
    let mut sound = mixer.take().map(|mixer| SoundWriter {
        mixer,
        clock: &clock,
        fps,
        total,
        next: 0,
        current_clip: None,
        pcm: Vec::new(),
        pcm_out: Vec::new(),
    });

    for frame_index in 0..total {
        if !progress(frame_index, total) {
            encoder.cancel();
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
        let webcam_frame = webcam
            .as_mut()
            .and_then(|reader| reader.frame_at(&cache, source));
        let webcam_views = webcam_frame.as_ref().map(WebcamFrame::views);
        let webcam_source = webcam_views.as_ref().map(WebcamViews::source);
        let background_frame = background
            .as_mut()
            .and_then(|reader| reader.frame_at(&cache, time));
        let background_views = background_frame.as_ref().map(WebcamFrame::views);
        let background_source = background_views.as_ref().map(WebcamViews::source);
        let mut frame_layers = scene.layers.at(time, webcam_source);
        frame_layers.background_video = background_source;
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
                camera: scene.layers.camera.sample(time),
                layers: Some(frame_layers),
            },
        );
        // The encoder reads the buffer as soon as it's handed over, so the GPU has to be finished.
        let _ = gpu.device.poll(wgpu::PollType::wait_indefinitely());

        // Until the frame is taken, give the encoder whatever else it wants.
        let waiting_since = Instant::now();
        while !encoder.video_is_ready()? {
            let fed = match &mut sound {
                Some(sound) => sound.feed(&mut encoder)?,
                None => false,
            };
            if fed {
                continue;
            }
            if waiting_since.elapsed() > ENCODER_STALL {
                encoder.cancel();
                return Err(ExportError(
                    "The video encoder stopped responding, so the export has stopped.".to_string(),
                ));
            }
            std::thread::sleep(Duration::from_micros(200));
        }
        encoder.append_video(&buffer)?;

        // Keep the sound level with the picture, if the encoder will take it now.
        if let Some(sound) = &mut sound {
            while sound.next <= frame_index && sound.feed(&mut encoder)? {}
        }
    }
    // Any sound still to go. With all the frames in, nothing is waiting on anything else.
    if let Some(sound) = &mut sound {
        while sound.next < total {
            sound.write_next(&mut encoder)?;
        }
        encoder.finish_audio();
    }
    encoder.finish()?;
    std::fs::rename(temporary, output).map_err(|error| {
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

/// The size a GIF comes out at: the stage's shape, with its height capped, and even (§43).
#[must_use]
pub fn gif_size(stage: (u32, u32), size: GifSize) -> (u32, u32) {
    let cap = match size {
        GifSize::Small => 480,
        GifSize::Medium => 720,
        GifSize::Large => 1080,
        GifSize::Original => u32::MAX,
    };
    if stage.1 <= cap {
        return (stage.0.max(2), stage.1.max(2));
    }
    let scale = f64::from(cap) / f64::from(stage.1);
    let even = |value: f64| ((value.round() as u32) & !1).max(2);
    (even(f64::from(stage.0) * scale), even(f64::from(cap)))
}

/// How long each frame of a GIF shows, in the hundredths of a second it has to be given in. Rounding
/// each by itself would change the frame rate, as it does in Recordly (§43). Rounding the running total
/// keeps the average right.
#[must_use]
pub fn gif_delay(frame: u64, fps: u32) -> u16 {
    let at = |index: u64| (index as f64 * 100.0 / f64::from(fps.max(1))).round() as u64;
    u16::try_from(at(frame + 1) - at(frame)).unwrap_or(u16::MAX)
}

/// Writes a project to a GIF. Each frame is written as soon as it's made, so a long one doesn't fill
/// memory (BUILD_PLAN 8.5). Like an MP4, the file only appears at its final path when it's complete.
///
/// # Errors
/// Returns an error if the export fails or is cancelled.
pub fn export_gif(
    scene: &Scene,
    output: &Path,
    mut progress: impl FnMut(u64, u64) -> bool,
) -> Result<ExportReport, ExportError> {
    let started = Instant::now();
    let project = &scene.project;
    let settings = &project.export;
    let fps = settings.gif_frame_rate.max(1);
    let (width, height) = gif_size(scene.stage, settings.gif_size);
    let (gif_width, gif_height) = (
        u16::try_from(width).map_err(|_| ExportError("That's too wide for a GIF.".to_string()))?,
        u16::try_from(height).map_err(|_| ExportError("That's too tall for a GIF.".to_string()))?,
    );

    let gpu = gpu()?;
    let mut renderer = Renderer::new(gpu.device.clone(), gpu.queue.clone(), WORKING_FORMAT);
    renderer.set_project_folder(scene.folder.as_deref());
    let cache = TextureCache::new(&gpu.device);
    let mut decoder = Decoder::open(&scene.video_path)?;
    let mut webcam = WebcamReader::open(scene);
    let mut background = BackgroundReader::open(scene);
    let target = readable_target(&gpu.device, (width, height), WORKING_FORMAT);

    let mut temporary = output.as_os_str().to_os_string();
    temporary.push(".tmp");
    let temporary = PathBuf::from(temporary);
    let result = (|| {
        let file = std::fs::File::create(&temporary).map_err(|error| {
            ExportError(format!("{} can't be written: {error}", temporary.display()))
        })?;
        let mut writer =
            gif::Encoder::new(std::io::BufWriter::new(file), gif_width, gif_height, &[])
                .map_err(|error| ExportError(format!("The GIF couldn't be started: {error}")))?;
        // A GIF with no repeat block plays once. Asking for a repeat count of 0 would loop forever, in
        // every viewer, which is how "loop off" still loops in Recordly (§58).
        if settings.gif_loop {
            writer
                .set_repeat(gif::Repeat::Infinite)
                .map_err(|error| ExportError(format!("The GIF couldn't be started: {error}")))?;
        }

        let clock = OutputClock {
            clips: &project.timeline.clips,
            fps: f64::from(fps),
        };
        let total = clock.total_frames();
        let video_size = (scene.info.width, scene.info.height);
        for frame_index in 0..total {
            if !progress(frame_index, total) {
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
            let webcam_frame = webcam
                .as_mut()
                .and_then(|reader| reader.frame_at(&cache, source));
            let webcam_views = webcam_frame.as_ref().map(WebcamFrame::views);
            let webcam_source = webcam_views.as_ref().map(WebcamViews::source);
            let background_frame = background
                .as_mut()
                .and_then(|reader| reader.frame_at(&cache, time));
            let background_views = background_frame.as_ref().map(WebcamFrame::views);
            let background_source = background_views.as_ref().map(WebcamViews::source);
            let mut frame_layers = scene.layers.at(time, webcam_source);
            frame_layers.background_video = background_source;
            renderer.render(
                &target,
                &FrameInput {
                    look: &project.look,
                    video: VideoSource::Nv12 {
                        luma: &luma_view,
                        chroma: &chroma_view,
                    },
                    video_size,
                    camera: scene.layers.camera.sample(time),
                    layers: Some(frame_layers),
                },
            );
            let _ = gpu.device.poll(wgpu::PollType::wait_indefinitely());
            let pixels = read_pixels(&gpu.device, &gpu.queue, &target);
            let mut image = crate::gif_frame::quantize(&pixels, width as usize, height as usize);
            image.width = gif_width;
            image.height = gif_height;
            image.delay = gif_delay(frame_index, fps);
            writer
                .write_frame(&image)
                .map_err(|error| ExportError(format!("The GIF couldn't be written: {error}")))?;
        }
        Ok(total)
    })();
    match result {
        Ok(total) => {
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
                bitrate: 0,
                output_seconds: total as f64 / f64::from(fps),
                elapsed_seconds: started.elapsed().as_secs_f64(),
            })
        }
        Err(error) => {
            let _ = std::fs::remove_file(&temporary);
            Err(error)
        }
    }
}

fn audio_frame(video_frame: u64, fps: u32) -> u64 {
    video_frame * u64::from(SAMPLE_RATE) / u64::from(fps)
}

/// The camera at a time, for callers drawing their own frames.
#[must_use]
pub fn camera_at(scene: &Scene, time: TimelineTime) -> Camera {
    scene.layers.camera.sample(time)
}
