//! The wgpu side: a thread that uploads video frames and draws them into the webview's clear area.
//!
//! The surface covers the whole window, under the webview. Video only appears inside the rectangle
//! the DOM reports for `#hole`, which is how the real editor will line up its preview.
//!
//! A one-pixel magenta ring marks the rectangle's edge. The DOM draws a lime outline just outside
//! the same rectangle, so a captured frame shows lime touching magenta if and only if the two
//! layers agree. Those colours are only there for the alignment check.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

use dioxus::desktop::tao::window::Window;

use crate::video::{self, Decoder, FPS, Frame, HEIGHT, WIDTH};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

/// One entry each time the DOM reports where `#hole` is.
#[derive(Clone, Copy, Debug)]
pub struct HoleReport {
    pub at: Instant,
    pub inner_w: u32,
    pub inner_h: u32,
}

#[derive(Default)]
pub struct PhaseStats {
    pub name: String,
    pub started: Option<Instant>,
    pub ended: Option<Instant>,
    pub present_ms: Vec<f32>,
    pub upload_ms: Vec<f32>,
    pub shown: u64,
    pub dropped: u64,
    pub underruns: u64,
    pub occluded: u64,
    pub reconfigures: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Hud {
    pub present_fps: f32,
    pub worst_ms: f32,
    pub shown: u64,
    pub dropped: u64,
    pub surface: (u32, u32),
    pub hole: Rect,
}

#[derive(Default)]
pub struct Shared {
    /// Physical size from the window's Resized events, as `width << 32 | height`.
    pub surface_size: AtomicU64,
    /// The latest DOM rectangle in physical pixels, and when each report arrived.
    pub hole: Mutex<Option<Rect>>,
    pub hole_reports: Mutex<Vec<HoleReport>>,
    /// The rectangle and size the last presented frame actually used.
    pub drawn: Mutex<(Rect, (u32, u32))>,
    pub phases: Mutex<Vec<PhaseStats>>,
    pub hud: Mutex<Hud>,
    pub playhead_on: AtomicBool,
    pub adapter: Mutex<String>,
}

pub static SHARED: LazyLock<Shared> = LazyLock::new(Shared::default);

impl Shared {
    pub fn set_surface_size(&self, width: u32, height: u32) {
        self.surface_size.store(
            u64::from(width) << 32 | u64::from(height),
            Ordering::Relaxed,
        );
    }

    fn surface_size(&self) -> (u32, u32) {
        let packed = self.surface_size.load(Ordering::Relaxed);
        ((packed >> 32) as u32, packed as u32)
    }

    pub fn begin_phase(&self, name: &str) {
        let mut phases = self.phases.lock().unwrap();
        let now = Instant::now();
        if let Some(last) = phases.last_mut() {
            last.ended = Some(now);
        }
        phases.push(PhaseStats {
            name: name.to_string(),
            started: Some(now),
            ..PhaseStats::default()
        });
    }
}

const SHADER: &str = r#"
struct Params {
    hole: vec4f,  // x, y, width, height in surface pixels
    video: vec4f, // the same, for the letterboxed video
    ring: vec4f,  // x holds the ring width
};
@group(0) @binding(0) var<uniform> p: Params;
@group(0) @binding(1) var frame: texture_2d<f32>;
@group(0) @binding(2) var linear: sampler;

@vertex
fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4f {
    let x = f32(i32(i & 1u) * 4 - 1);
    let y = f32(i32(i >> 1u) * 4 - 1);
    return vec4f(x, y, 0.0, 1.0);
}

@fragment
fn fs(@builtin(position) pos: vec4f) -> @location(0) vec4f {
    let in_hole = pos.xy - p.hole.xy;
    let r = p.ring.x;
    if (in_hole.x < r || in_hole.y < r || in_hole.x >= p.hole.z - r || in_hole.y >= p.hole.w - r) {
        return vec4f(1.0, 0.0, 1.0, 1.0);
    }
    let uv = (pos.xy - p.video.xy) / p.video.zw;
    if (uv.x < 0.0 || uv.y < 0.0 || uv.x > 1.0 || uv.y > 1.0) {
        return vec4f(0.02, 0.02, 0.03, 1.0);
    }
    return textureSampleLevel(frame, linear, uv, 0.0);
}
"#;

/// Creates the surface here, on the main thread and before the webview exists, because that puts
/// the Metal layer underneath the webview's. The rest happens on the render thread.
pub fn start(window: Arc<Window>, ffmpeg: String, video_path: String) {
    let size = window.inner_size();
    SHARED.set_surface_size(size.width, size.height);

    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    #[cfg(target_os = "macos")]
    let content_view = crate::macos::content_view(&window);
    let surface = instance
        .create_surface(window)
        .expect("couldn't create a wgpu surface for the window");
    #[cfg(target_os = "macos")]
    crate::macos::configure_metal_layer(content_view);

    std::thread::Builder::new()
        .name("render".into())
        .spawn(move || {
            let decoder = video::start(&ffmpeg, &video_path);
            run(&instance, &surface, decoder);
        })
        .expect("couldn't start the render thread");
}

fn run(instance: &wgpu::Instance, surface: &wgpu::Surface<'_>, decoder: Decoder) {
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        force_fallback_adapter: false,
        compatible_surface: Some(surface),
        apply_limit_buckets: false,
    }))
    .expect("no GPU adapter");
    *SHARED.adapter.lock().unwrap() = format!("{:?}", adapter.get_info());

    let (device, queue) =
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
            .expect("couldn't create the GPU device");

    let caps = surface.get_capabilities(&adapter);
    let format = caps
        .formats
        .iter()
        .copied()
        .find(wgpu::TextureFormat::is_srgb)
        .unwrap_or(caps.formats[0]);
    let (width, height) = SHARED.surface_size();
    let mut config = wgpu::SurfaceConfiguration {
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        format,
        color_space: wgpu::SurfaceColorSpace::Auto,
        width: width.max(1),
        height: height.max(1),
        present_mode: wgpu::PresentMode::Fifo,
        desired_maximum_frame_latency: frame_latency(),
        alpha_mode: caps.alpha_modes[0],
        view_formats: vec![],
    };
    surface.configure(&device, &config);

    let extent = wgpu::Extent3d {
        width: WIDTH,
        height: HEIGHT,
        depth_or_array_layers: 1,
    };
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("video"),
        size: extent,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let params = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("params"),
        size: 48,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    });
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("video"),
        source: wgpu::ShaderSource::Wgsl(SHADER.into()),
    });
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("video"),
        layout: None,
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs"),
            compilation_options: Default::default(),
            targets: &[Some(format.into())],
        }),
        multiview_mask: None,
        cache: None,
    });
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("video"),
        layout: &pipeline.get_bind_group_layout(0),
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: params.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(
                    &texture.create_view(&wgpu::TextureViewDescriptor::default()),
                ),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::Sampler(&sampler),
            },
        ],
    });

    let Decoder { frames, recycle } = decoder;
    let mut queued: Option<Frame> = None;
    let mut clock: Option<Instant> = None;
    let mut last_shown = 0;
    let mut last_present = Instant::now();
    let mut hud_window = HudWindow::new();

    loop {
        let wanted = SHARED.surface_size();
        let mut reconfigured = false;
        if wanted.0 > 0 && wanted.1 > 0 && wanted != (config.width, config.height) {
            config.width = wanted.0;
            config.height = wanted.1;
            surface.configure(&device, &config);
            reconfigured = true;
        }

        let target = match surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(texture)
            | wgpu::CurrentSurfaceTexture::Suboptimal(texture) => texture,
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                record(|phase| phase.occluded += 1);
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

        // Take every frame that's due, keeping only the newest.
        let due = clock.map_or(0, |start: Instant| {
            (start.elapsed().as_secs_f64() * FPS) as u64
        });
        let mut newest: Option<Frame> = None;
        let mut dropped = 0;
        loop {
            if queued.is_none() {
                queued = frames.try_recv().ok();
            }
            match queued.take() {
                Some(frame) if frame.index <= due => {
                    if let Some(stale) = newest.replace(frame) {
                        dropped += 1;
                        let _ = recycle.send(stale.rgba);
                    }
                }
                other => {
                    queued = other;
                    break;
                }
            }
        }
        let underrun = newest.is_none() && queued.is_none() && due > last_shown;

        let mut upload_ms = None;
        let shown_new = newest.is_some();
        if let Some(frame) = newest {
            let started = Instant::now();
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &frame.rgba,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(WIDTH * 4),
                    rows_per_image: Some(HEIGHT),
                },
                extent,
            );
            upload_ms = Some(started.elapsed().as_secs_f32() * 1000.0);
            last_shown = frame.index;
            clock.get_or_insert_with(Instant::now);
            let _ = recycle.send(frame.rgba);
        }

        let hole = *SHARED.hole.lock().unwrap();
        let view = target
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("frame"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.05,
                            g: 0.05,
                            b: 0.06,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            // The DOM's rectangle can be out of date while resizing, and wgpu rejects a scissor
            // that doesn't fit inside the surface.
            if let Some(hole) = hole.and_then(|hole| clamp_to_surface(hole, &config)) {
                queue.write_buffer(&params, 0, &params_bytes(hole));
                pass.set_pipeline(&pipeline);
                pass.set_bind_group(0, &bind_group, &[]);
                pass.set_scissor_rect(hole.x, hole.y, hole.w, hole.h);
                pass.draw(0..3, 0..1);
            }
        }
        queue.submit(Some(encoder.finish()));
        queue.present(target);

        let now = Instant::now();
        let present_ms = now.duration_since(last_present).as_secs_f32() * 1000.0;
        last_present = now;

        record(|phase| {
            phase.present_ms.push(present_ms);
            phase.shown += u64::from(shown_new);
            phase.dropped += dropped;
            phase.underruns += u64::from(underrun);
            phase.reconfigures += u64::from(reconfigured);
            if let Some(ms) = upload_ms {
                phase.upload_ms.push(ms);
            }
        });
        *SHARED.drawn.lock().unwrap() = (hole.unwrap_or_default(), (config.width, config.height));
        hud_window.tick(
            present_ms,
            shown_new,
            dropped,
            (config.width, config.height),
            hole,
        );
    }
}

/// How many frames can queue up. On macOS, 1 cost no frame rate and made the moving edge line up in
/// about 60% of captures during a live resize, against 11% with 2. T1_LATENCY overrides it.
pub fn frame_latency() -> u32 {
    std::env::var("T1_LATENCY")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(1)
}

fn record(update: impl FnOnce(&mut PhaseStats)) {
    if let Some(phase) = SHARED.phases.lock().unwrap().last_mut() {
        update(phase);
    }
}

fn clamp_to_surface(hole: Rect, config: &wgpu::SurfaceConfiguration) -> Option<Rect> {
    if hole.x >= config.width || hole.y >= config.height {
        return None;
    }
    let clamped = Rect {
        x: hole.x,
        y: hole.y,
        w: hole.w.min(config.width - hole.x),
        h: hole.h.min(config.height - hole.y),
    };
    (clamped.w > 0 && clamped.h > 0).then_some(clamped)
}

/// The hole's rectangle, then the largest 16:9 rectangle centred inside it, then the ring width.
fn params_bytes(hole: Rect) -> Vec<u8> {
    let (x, y, w, h) = (hole.x as f32, hole.y as f32, hole.w as f32, hole.h as f32);
    let aspect = WIDTH as f32 / HEIGHT as f32;
    let (video_w, video_h) = if w / h > aspect {
        (h * aspect, h)
    } else {
        (w, w / aspect)
    };
    let values = [
        x,
        y,
        w,
        h,
        x + (w - video_w) / 2.0,
        y + (h - video_h) / 2.0,
        video_w,
        video_h,
        1.0,
        0.0,
        0.0,
        0.0,
    ];
    values
        .iter()
        .flat_map(|value| value.to_ne_bytes())
        .collect()
}

/// A rolling summary for the on-screen readout, which is how a person checks this by eye.
struct HudWindow {
    started: Instant,
    frames: u32,
    worst_ms: f32,
    shown: u64,
    dropped: u64,
}

impl HudWindow {
    fn new() -> Self {
        Self {
            started: Instant::now(),
            frames: 0,
            worst_ms: 0.0,
            shown: 0,
            dropped: 0,
        }
    }

    fn tick(
        &mut self,
        present_ms: f32,
        shown_new: bool,
        dropped: u64,
        surface: (u32, u32),
        hole: Option<Rect>,
    ) {
        self.frames += 1;
        self.worst_ms = self.worst_ms.max(present_ms);
        self.shown += u64::from(shown_new);
        self.dropped += dropped;
        let elapsed = self.started.elapsed().as_secs_f32();
        if elapsed >= 0.5 {
            *SHARED.hud.lock().unwrap() = Hud {
                present_fps: self.frames as f32 / elapsed,
                worst_ms: self.worst_ms,
                shown: self.shown,
                dropped: self.dropped,
                surface,
                hole: hole.unwrap_or_default(),
            };
            self.started = Instant::now();
            self.frames = 0;
            self.worst_ms = 0.0;
        }
    }
}
