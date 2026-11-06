//! The live preview: the same renderer as export, drawing to a wgpu surface under the webview.
//!
//! This follows what T1 and T2 found (`docs/spikes.md`). The surface is made before the webview, so
//! its layer sits underneath. It renders on its own thread, so a busy UI can't stall the picture. The
//! drawable is pinned to the top left and only one frame queues up, so resizing stays aligned. Frames
//! reach the GPU with no copying.

use std::collections::VecDeque;
use std::ffi::c_void;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;

use dioxus::desktop::tao::platform::macos::WindowExtMacOS;
use dioxus::desktop::tao::window::Window;
use objc2::runtime::AnyObject;
use objc2::{class, msg_send};
use objc2_foundation::{NSArray, NSString};
use reel_core::project::{Project, SourceTrack};
use reel_core::time::{Micros, SourceTime, TimelineTime};
use reel_media::macos::{Decoder, Frame, read_pcm};
use reel_media::mix::{AddedAudio, Mixer, Pcm, source_audio};
use reel_media::player::{Clock, Player};
use reel_render::apple::{TextureCache, WrappedTexture};
use reel_render::export::Scene;
use reel_render::{FrameInput, Renderer, VideoSource};

/// A rectangle in physical pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PixelRect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

/// The well the preview sits in, when the frame doesn't fill it.
const WELL: wgpu::Color = wgpu::Color {
    r: 0.07,
    g: 0.07,
    b: 0.08,
    a: 1.0,
};

#[derive(Default)]
pub struct Shared {
    surface_size: AtomicU64,
    hole: Mutex<Option<PixelRect>>,
    scene: Mutex<Option<Arc<Scene>>>,
    clock: Mutex<Option<Clock>>,
    /// Goes up whenever a new scene is loaded, which tells the decode thread to start over.
    generation: AtomicU64,
    /// What the decode thread should be showing, in source microseconds.
    wanted: AtomicI64,
    latest_frame: Mutex<Option<FrameSlot>>,
    /// Something changed that a paused picture needs to redraw for.
    dirty: AtomicBool,
    #[cfg(feature = "self-test")]
    pub stats: Mutex<Stats>,
}

struct FrameSlot {
    frame: Arc<Frame>,
    generation: u64,
}

pub static SHARED: LazyLock<Shared> = LazyLock::new(Shared::default);

impl Shared {
    pub fn set_surface_size(&self, width: u32, height: u32) {
        self.surface_size.store(
            u64::from(width) << 32 | u64::from(height),
            Ordering::Relaxed,
        );
        self.mark_dirty();
    }

    fn surface_size(&self) -> (u32, u32) {
        let packed = self.surface_size.load(Ordering::Relaxed);
        ((packed >> 32) as u32, packed as u32)
    }

    pub fn set_hole(&self, hole: PixelRect) {
        *self
            .hole
            .lock()
            .expect("no thread panics while holding this") = Some(hole);
        self.mark_dirty();
    }

    pub fn mark_dirty(&self) {
        self.dirty.store(true, Ordering::Relaxed);
    }

    fn scene(&self) -> Option<Arc<Scene>> {
        self.scene
            .lock()
            .expect("no thread panics while holding this")
            .clone()
    }

    fn clock(&self) -> Option<Clock> {
        self.clock
            .lock()
            .expect("no thread panics while holding this")
            .clone()
    }

    pub fn position(&self) -> TimelineTime {
        self.clock().map_or(TimelineTime::ZERO, |clock| clock.now())
    }

    pub fn is_playing(&self) -> bool {
        self.clock().is_some_and(|clock| clock.is_playing())
    }
}

thread_local! {
    /// Sound streams have to stay on the thread that made them, which is the UI's.
    static PLAYER: std::cell::RefCell<Option<Player>> = const { std::cell::RefCell::new(None) };
}

pub fn play() {
    PLAYER.with_borrow(|player| {
        if let Some(player) = player {
            player.play();
        }
    });
    SHARED.mark_dirty();
}

pub fn pause() {
    PLAYER.with_borrow(|player| {
        if let Some(player) = player {
            player.pause();
        }
    });
    SHARED.mark_dirty();
}

pub fn seek(time: TimelineTime) {
    PLAYER.with_borrow(|player| {
        if let Some(player) = player {
            player.seek(time);
        }
    });
    SHARED.mark_dirty();
}

pub fn set_volume(volume: f64) {
    PLAYER.with_borrow(|player| {
        if let Some(player) = player {
            player.set_volume(volume);
        }
    });
}

/// A project and its sound, loaded off the UI thread, ready to start.
pub struct Prepared {
    scene: Arc<Scene>,
    mixer: Mixer,
}

fn load_mixer(scene: &Scene) -> Mixer {
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
    Mixer::new(project, sources, added)
}

/// Does the slow part of opening a project. This belongs off the UI thread.
///
/// # Errors
/// Returns why the project can't be opened, in words for the user.
pub fn prepare(project: Project, folder: Option<&Path>) -> Result<Prepared, String> {
    let scene = Scene::new(project, folder).map_err(|error| error.0)?;
    let mixer = load_mixer(&scene);
    Ok(Prepared {
        scene: Arc::new(scene),
        mixer,
    })
}

/// Starts showing a prepared project. This has to run on the UI thread.
pub fn show(prepared: Prepared) {
    let Prepared { scene, mixer } = prepared;
    let player = Player::new(mixer, scene.project.timeline.clips.clone());
    *SHARED
        .clock
        .lock()
        .expect("no thread panics while holding this") = Some(player.clock());
    *SHARED
        .scene
        .lock()
        .expect("no thread panics while holding this") = Some(scene);
    SHARED.generation.fetch_add(1, Ordering::Relaxed);
    PLAYER.with_borrow_mut(|slot| *slot = Some(player));
    SHARED.mark_dirty();
}

/// Sets up the surface and starts the threads. This has to run before the webview exists, so that the
/// surface's layer ends up beneath it.
pub fn start(window: Arc<Window>) {
    let size = window.inner_size();
    SHARED.set_surface_size(size.width, size.height);

    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let view = window.ns_view();
    let surface = instance
        .create_surface(window)
        .expect("couldn't make a surface for the window");
    pin_layer(view);

    std::thread::Builder::new()
        .name("reel-render".into())
        .spawn(move || render_thread(&instance, &surface))
        .expect("couldn't start the render thread");
    std::thread::Builder::new()
        .name("reel-decode".into())
        .spawn(decode_thread)
        .expect("couldn't start the decode thread");
}

#[link(name = "QuartzCore", kind = "framework")]
unsafe extern "C" {
    static kCAGravityTopLeft: &'static NSString;
}

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGColorCreateSRGB(red: f64, green: f64, blue: f64, alpha: f64) -> *mut c_void;
    fn CGColorRelease(color: *mut c_void);
}

/// By default Core Animation stretches the last frame to fit a resized window, which moves even the
/// edges that aren't changing. T1 measured this at 12 of 221 frames aligned, and 230 of 230 like this.
fn pin_layer(view: *mut c_void) {
    let view = view.cast::<AnyObject>();
    // SAFETY: `view` is the window's live content view, these are the documented selectors with their
    // documented types, and this runs on the main thread during window setup.
    unsafe {
        let root: *mut AnyObject = msg_send![view, layer];
        let sublayers: *mut NSArray<AnyObject> = msg_send![root, sublayers];
        let Some(sublayers) = sublayers.as_ref() else {
            return;
        };
        for layer in sublayers.iter() {
            let is_metal: bool = msg_send![&*layer, isKindOfClass: class!(CAMetalLayer)];
            if is_metal {
                let _: () = msg_send![&*layer, setContentsGravity: kCAGravityTopLeft];
                let color = CGColorCreateSRGB(WELL.r, WELL.g, WELL.b, 1.0);
                let _: () = msg_send![&*layer, setBackgroundColor: color.cast::<AnyObject>()];
                CGColorRelease(color);
            }
        }
    }
}

/// Decodes whatever the render thread currently wants, so a seek can't hold up drawing.
fn decode_thread() {
    let mut decoder: Option<Decoder> = None;
    let mut generation = u64::MAX;
    let mut last_wanted = i64::MIN;
    loop {
        let current = SHARED.generation.load(Ordering::Relaxed);
        if current != generation {
            generation = current;
            last_wanted = i64::MIN;
            decoder = SHARED
                .scene()
                .and_then(|scene| Decoder::open(&scene.video_path).ok());
        }
        let wanted = SHARED.wanted.load(Ordering::Relaxed);
        let Some(active) = decoder.as_mut().filter(|_| wanted != last_wanted) else {
            std::thread::sleep(Duration::from_millis(2));
            continue;
        };
        last_wanted = wanted;
        let Ok(Some(frame)) = active.frame_at(SourceTime(wanted)) else {
            continue;
        };
        let mut slot = SHARED
            .latest_frame
            .lock()
            .expect("no thread panics while holding this");
        let changed = slot.as_ref().is_none_or(|existing| {
            existing.frame.pts != frame.pts || existing.generation != generation
        });
        if changed {
            *slot = Some(FrameSlot {
                frame: Arc::new(Frame {
                    pts: frame.pts,
                    buffer: frame.buffer.clone(),
                }),
                generation,
            });
            SHARED.mark_dirty();
        }
    }
}

/// The largest area of the hole with the stage's shape, centred.
fn fit(hole: PixelRect, stage: (u32, u32)) -> PixelRect {
    let scale = (f64::from(hole.width) / f64::from(stage.0))
        .min(f64::from(hole.height) / f64::from(stage.1));
    let width = ((f64::from(stage.0) * scale) as u32).max(1);
    let height = ((f64::from(stage.1) * scale) as u32).max(1);
    PixelRect {
        x: hole.x + (hole.width - width.min(hole.width)) / 2,
        y: hole.y + (hole.height - height.min(hole.height)) / 2,
        width,
        height,
    }
}

fn render_thread(instance: &wgpu::Instance, surface: &wgpu::Surface<'_>) {
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        force_fallback_adapter: false,
        compatible_surface: Some(surface),
        apply_limit_buckets: false,
    }))
    .expect("no GPU was found");
    let (device, queue) =
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
            .expect("the GPU device couldn't be created");

    let format = wgpu::TextureFormat::Bgra8Unorm;
    let capabilities = surface.get_capabilities(&adapter);
    let size = SHARED.surface_size();
    let mut config = wgpu::SurfaceConfiguration {
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_DST,
        format,
        color_space: wgpu::SurfaceColorSpace::Auto,
        width: size.0.max(1),
        height: size.1.max(1),
        present_mode: wgpu::PresentMode::Fifo,
        // T1 found this costs no frame rate, and keeps a resize close to the window's edge.
        desired_maximum_frame_latency: 1,
        alpha_mode: capabilities.alpha_modes[0],
        view_formats: vec![],
    };
    surface.configure(&device, &config);

    let mut renderer = Renderer::new(device.clone(), queue.clone(), format);
    let cache = TextureCache::new(&device);
    let mut offscreen: Option<(wgpu::Texture, (u32, u32))> = None;
    // Wrapped frames stay alive until the GPU has surely finished with them.
    let mut in_flight: VecDeque<(WrappedTexture, WrappedTexture)> = VecDeque::new();
    let mut folder: Option<PathBuf> = None;

    loop {
        let wanted_size = SHARED.surface_size();
        if wanted_size.0 > 0 && wanted_size.1 > 0 && wanted_size != (config.width, config.height) {
            config.width = wanted_size.0;
            config.height = wanted_size.1;
            surface.configure(&device, &config);
            SHARED.mark_dirty();
        }

        let scene = SHARED.scene();
        let clock = SHARED.clock();
        let playing = clock.as_ref().is_some_and(Clock::is_playing);
        let time = clock.as_ref().map_or(TimelineTime::ZERO, Clock::now);

        if let Some(scene) = &scene
            && let Some(source) = scene.project.timeline.to_source(time)
        {
            SHARED.wanted.store(source.0, Ordering::Relaxed);
        }

        // A paused picture only needs drawing when something changes.
        if !playing && !SHARED.dirty.swap(false, Ordering::Relaxed) {
            std::thread::sleep(Duration::from_millis(4));
            continue;
        }

        let target = match surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(texture)
            | wgpu::CurrentSurfaceTexture::Suboptimal(texture) => texture,
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                SHARED.mark_dirty();
                std::thread::sleep(Duration::from_millis(8));
                continue;
            }
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                surface.configure(&device, &config);
                continue;
            }
            wgpu::CurrentSurfaceTexture::Validation => {
                panic!("wgpu reported a validation error acquiring a frame")
            }
        };

        let hole = *SHARED
            .hole
            .lock()
            .expect("no thread panics while holding this");
        let generation = SHARED.generation.load(Ordering::Relaxed);
        let slot = SHARED
            .latest_frame
            .lock()
            .expect("no thread panics while holding this");
        let frame = slot
            .as_ref()
            .filter(|slot| slot.generation == generation)
            .map(|slot| slot.frame.clone());
        drop(slot);

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        let surface_view = target
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        {
            // Clearing everything, then copying the picture in.
            let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("well"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &surface_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(WELL),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
        }

        if let (Some(scene), Some(hole), Some(frame)) = (&scene, hole, &frame)
            && hole.width > 1
            && hole.height > 1
        {
            if folder.as_deref() != scene.folder.as_deref() {
                folder.clone_from(&scene.folder);
                renderer.set_project_folder(folder.as_deref());
            }
            let placed = fit(hole, scene.stage);
            let stage = (placed.width, placed.height);
            if offscreen.as_ref().is_none_or(|(_, size)| *size != stage) {
                offscreen = Some((
                    device.create_texture(&wgpu::TextureDescriptor {
                        label: Some("picture"),
                        size: wgpu::Extent3d {
                            width: stage.0,
                            height: stage.1,
                            depth_or_array_layers: 1,
                        },
                        mip_level_count: 1,
                        sample_count: 1,
                        dimension: wgpu::TextureDimension::D2,
                        format,
                        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                            | wgpu::TextureUsages::COPY_SRC,
                        view_formats: &[],
                    }),
                    stage,
                ));
            }
            let (picture, _) = offscreen.as_ref().expect("it was just made");

            let video_size = (scene.info.width, scene.info.height);
            let (luma, chroma) = cache.wrap_nv12(&frame.buffer, video_size);
            let luma_view = luma
                .texture
                .create_view(&wgpu::TextureViewDescriptor::default());
            let chroma_view = chroma
                .texture
                .create_view(&wgpu::TextureViewDescriptor::default());
            let picture_view = picture.create_view(&wgpu::TextureViewDescriptor::default());

            renderer.prepare_background(&scene.project.look, stage);
            renderer.encode(
                &mut encoder,
                &picture_view,
                stage,
                &FrameInput {
                    look: &scene.project.look,
                    video: VideoSource::Nv12 {
                        luma: &luma_view,
                        chroma: &chroma_view,
                    },
                    video_size,
                    camera: scene.layers.camera.sample(time),
                    layers: Some(scene.layers.at(time, None)),
                },
            );
            // The window may just have shrunk past where the DOM says the hole is.
            let copy_width = stage.0.min(config.width.saturating_sub(placed.x));
            let copy_height = stage.1.min(config.height.saturating_sub(placed.y));
            if copy_width > 0 && copy_height > 0 {
                encoder.copy_texture_to_texture(
                    picture.as_image_copy(),
                    wgpu::TexelCopyTextureInfo {
                        texture: &target.texture,
                        mip_level: 0,
                        origin: wgpu::Origin3d {
                            x: placed.x,
                            y: placed.y,
                            z: 0,
                        },
                        aspect: wgpu::TextureAspect::All,
                    },
                    wgpu::Extent3d {
                        width: copy_width,
                        height: copy_height,
                        depth_or_array_layers: 1,
                    },
                );
            }
            in_flight.push_back((luma, chroma));
            while in_flight.len() > 3 {
                in_flight.pop_front();
            }
            #[cfg(feature = "self-test")]
            SHARED
                .stats
                .lock()
                .expect("no thread panics while holding this")
                .record(scene, time, frame, playing);
        }

        queue.submit(Some(encoder.finish()));
        queue.present(target);
    }
}

#[cfg(feature = "self-test")]
#[derive(Default)]
pub struct Stats {
    pub present_ms: Vec<f32>,
    /// How far the picture is from what the clock says should be showing, in milliseconds.
    pub sync_ms: Vec<f32>,
    pub distinct_frames: u64,
    last_pts: Option<SourceTime>,
    last_present: Option<std::time::Instant>,
}

#[cfg(feature = "self-test")]
impl Stats {
    fn record(&mut self, scene: &Scene, time: TimelineTime, frame: &Frame, playing: bool) {
        let now = std::time::Instant::now();
        if playing {
            if let Some(last) = self.last_present {
                self.present_ms
                    .push(now.duration_since(last).as_secs_f32() * 1000.0);
            }
            if let Some(source) = scene.project.timeline.to_source(time) {
                self.sync_ms
                    .push(((source - frame.pts).0 as f32 / 1000.0).abs());
            }
            if self.last_pts != Some(frame.pts) {
                self.distinct_frames += 1;
                self.last_pts = Some(frame.pts);
            }
        }
        self.last_present = Some(now);
    }

    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

/// How long a project runs.
pub fn duration() -> Micros {
    SHARED
        .scene()
        .map_or(Micros::ZERO, |scene| scene.project.timeline.duration())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_picture_fits_inside_the_hole_without_changing_shape() {
        let wide = PixelRect {
            x: 10,
            y: 20,
            width: 1000,
            height: 400,
        };
        assert_eq!(
            fit(wide, (1920, 1080)),
            PixelRect {
                x: 154,
                y: 20,
                width: 711,
                height: 400
            }
        );
        let tall = PixelRect {
            x: 0,
            y: 0,
            width: 400,
            height: 1000,
        };
        assert_eq!(
            fit(tall, (1920, 1080)),
            PixelRect {
                x: 0,
                y: 387,
                width: 400,
                height: 225
            }
        );
    }
}
