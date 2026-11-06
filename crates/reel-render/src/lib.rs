//! The one wgpu compositor. Preview draws with it to a window, and export draws with it offscreen, so
//! they can't disagree about how a project looks (BUILD_PLAN approach 4).
//!
//! The draw order is fixed: background, shadow, then video, cursor and annotations inside the camera,
//! then webcam and captions outside it. That settles the disagreement between Recordly's three paths
//! (§31).

pub mod gradient;
mod shaders;

#[cfg(target_os = "macos")]
pub mod apple;
#[cfg(target_os = "macos")]
pub mod export;

use reel_core::project::{Background, Look, MediaRef};
use reel_motion::camera::Camera;
use reel_motion::layout::{self, Layout, Rect};
use std::path::Path;

use gradient::{Gradient, Shape, parse_color};

/// Everything the renderer draws into has this format, holding encoded (not linear) colour.
pub const WORKING_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// The blur setting is multiplied by this at a stage 1080 pixels tall. Recordly's preview and export
/// disagree about this (§31), and this is what its export uses.
const BLUR_FACTOR: f64 = 3.0;

/// The current video frame, in whichever form it reached the GPU.
#[derive(Clone, Copy)]
pub enum VideoSource<'a> {
    Rgba(&'a wgpu::TextureView),
    /// BT.709 limited range, with a full-size luma plane and a half-size chroma plane.
    Nv12 { luma: &'a wgpu::TextureView, chroma: &'a wgpu::TextureView },
}

/// What to draw for one frame.
pub struct FrameInput<'a> {
    pub look: &'a Look,
    pub video: VideoSource<'a>,
    /// The video's size in pixels.
    pub video_size: (u32, u32),
    pub camera: Camera,
}

fn uniform_bytes(values: &[f32]) -> Vec<u8> {
    values.iter().flat_map(|value| value.to_ne_bytes()).collect()
}

struct Background_ {
    view: wgpu::TextureView,
    key: BackgroundKey,
}

#[derive(Clone, PartialEq)]
struct BackgroundKey {
    background: Background,
    blur: f64,
    stage: (u32, u32),
}

pub struct Renderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    target_format: wgpu::TextureFormat,
    sampler: wgpu::Sampler,
    background_pipeline: wgpu::RenderPipeline,
    blur_pipeline: wgpu::RenderPipeline,
    frame_pipeline: wgpu::RenderPipeline,
    copy_pipeline: wgpu::RenderPipeline,
    frame_params: wgpu::Buffer,
    placeholder: wgpu::TextureView,
    background: Option<Background_>,
    /// Bundled wallpapers aren't shipped yet (D8), so this says which fell back to a colour.
    pub warnings: Vec<String>,
    project_folder: Option<std::path::PathBuf>,
}

fn fullscreen_pipeline(
    device: &wgpu::Device,
    source: &str,
    format: wgpu::TextureFormat,
    blend: Option<wgpu::BlendState>,
) -> wgpu::RenderPipeline {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: None,
        source: wgpu::ShaderSource::Wgsl(source.into()),
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: None,
        layout: None,
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[],
        },
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    })
}

const COPY: &str = r#"
@group(0) @binding(0) var source: texture_2d<f32>;
@group(0) @binding(1) var linear_sampler: sampler;
struct Out { @builtin(position) position: vec4f, @location(0) uv: vec2f };
@vertex
fn vs(@builtin(vertex_index) i: u32) -> Out {
    let x = f32(i32(i & 1u) * 4 - 1);
    let y = f32(i32(i >> 1u) * 4 - 1);
    return Out(vec4f(x, y, 0.0, 1.0), vec2f((x + 1.0) / 2.0, (1.0 - y) / 2.0));
}
@fragment
fn fs(in: Out) -> @location(0) vec4f {
    return textureSampleLevel(source, linear_sampler, in.uv, 0.0);
}
"#;

fn make_texture(
    device: &wgpu::Device,
    size: (u32, u32),
    format: wgpu::TextureFormat,
    usage: wgpu::TextureUsages,
) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d { width: size.0.max(1), height: size.1.max(1), depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage,
        view_formats: &[],
    })
}

impl Renderer {
    /// Sets up a renderer that draws to textures of this format.
    #[must_use]
    pub fn new(device: wgpu::Device, queue: wgpu::Queue, target_format: wgpu::TextureFormat) -> Self {
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            ..wgpu::SamplerDescriptor::default()
        });
        let premultiplied = wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                operation: wgpu::BlendOperation::Add,
            },
        };
        let background_pipeline = fullscreen_pipeline(&device, shaders::BACKGROUND, WORKING_FORMAT, None);
        let blur_pipeline = fullscreen_pipeline(&device, shaders::BLUR, WORKING_FORMAT, None);
        let frame_pipeline = fullscreen_pipeline(&device, shaders::FRAME, target_format, Some(premultiplied));
        let copy_pipeline = fullscreen_pipeline(&device, COPY, target_format, None);
        let frame_params = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: 112,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let placeholder_texture = make_texture(
            &device,
            (1, 1),
            WORKING_FORMAT,
            wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        );
        queue.write_texture(
            placeholder_texture.as_image_copy(),
            &[0, 0, 0, 255],
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(4), rows_per_image: Some(1) },
            wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
        );
        let placeholder = placeholder_texture.create_view(&wgpu::TextureViewDescriptor::default());
        Self {
            device,
            queue,
            target_format,
            sampler,
            background_pipeline,
            blur_pipeline,
            frame_pipeline,
            copy_pipeline,
            frame_params,
            placeholder,
            background: None,
            warnings: Vec::new(),
            project_folder: None,
        }
    }

    #[must_use]
    pub fn device(&self) -> &wgpu::Device {
        &self.device
    }

    #[must_use]
    pub fn queue(&self) -> &wgpu::Queue {
        &self.queue
    }

    #[must_use]
    pub fn target_format(&self) -> wgpu::TextureFormat {
        self.target_format
    }

    /// Where relative media paths in the background start from.
    pub fn set_project_folder(&mut self, folder: Option<&Path>) {
        self.project_folder = folder.map(Path::to_path_buf);
    }

    /// Draws one frame into a texture of the renderer's format.
    pub fn render(&mut self, target: &wgpu::Texture, input: &FrameInput<'_>) {
        let size = target.size();
        let stage = (size.width, size.height);
        self.prepare_background(input.look, stage);
        let view = target.create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        self.encode(&mut encoder, &view, stage, input);
        self.queue.submit(Some(encoder.finish()));
    }

    /// Records one frame's drawing. This is for callers that manage their own submission.
    pub fn encode(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        stage: (u32, u32),
        input: &FrameInput<'_>,
    ) {
        let stage_f = (f64::from(stage.0), f64::from(stage.1));
        let video_f = (f64::from(input.video_size.0), f64::from(input.video_size.1));
        let look = input.look;
        let layout = layout::compute(stage_f, &look.padding, &look.crop, video_f);
        self.write_frame_params(input, &layout, stage_f);

        let background = self.background.as_ref().expect("prepare_background runs first");
        let copy_bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &self.copy_pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&background.view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.sampler) },
            ],
        });
        let (plane0, plane1) = match input.video {
            VideoSource::Rgba(view) => (view, &self.placeholder),
            VideoSource::Nv12 { luma, chroma } => (luma, chroma),
        };
        let frame_bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &self.frame_pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: self.frame_params.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(plane0) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(plane1) },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::Sampler(&self.sampler) },
            ],
        });

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("frame"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.copy_pipeline);
        pass.set_bind_group(0, &copy_bind, &[]);
        pass.draw(0..3, 0..1);
        pass.set_pipeline(&self.frame_pipeline);
        pass.set_bind_group(0, &frame_bind, &[]);
        pass.draw(0..3, 0..1);
    }

    fn write_frame_params(&self, input: &FrameInput<'_>, layout: &Layout, stage: (f64, f64)) {
        let camera = input.camera;
        let radius = layout::corner_radius_px(input.look.corner_radius, stage);
        // Shadows are measured at 1920×1080, and scale with the stage like the corner radius does.
        let shadow_scale = (stage.0 / 1920.0).min(stage.1 / 1080.0);
        let layers = layout::shadow_layers(input.look.shadow);
        let kind = match input.video {
            VideoSource::Rgba(_) => 0.0,
            VideoSource::Nv12 { .. } => 1.0,
        };
        let rect = |r: &Rect| [r.x as f32, r.y as f32, r.width as f32, r.height as f32];
        let mut values: Vec<f32> = vec![
            camera.scale as f32,
            (camera.x * stage.0) as f32,
            (camera.y * stage.1) as f32,
            0.0,
        ];
        values.extend(rect(&layout.frame));
        values.extend(rect(&layout.video));
        values.extend([radius as f32, kind, shadow_scale as f32, 0.0]);
        for layer in &layers {
            values.extend([
                (layer.offset_y * shadow_scale) as f32,
                // A CSS blur radius is two standard deviations.
                (layer.blur * shadow_scale / 2.0) as f32,
                layer.alpha as f32,
                0.0,
            ]);
        }
        self.queue.write_buffer(&self.frame_params, 0, &uniform_bytes(&values));
    }

    /// Makes sure the background for this look and size is ready. It's kept until either changes.
    pub fn prepare_background(&mut self, look: &Look, stage: (u32, u32)) {
        let key = BackgroundKey {
            background: look.background.clone(),
            blur: look.background_blur,
            stage,
        };
        if self.background.as_ref().is_some_and(|current| current.key == key) {
            return;
        }
        let texture = self.draw_background(&key);
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        self.background = Some(Background_ { view, key });
    }

    fn draw_background(&mut self, key: &BackgroundKey) -> wgpu::Texture {
        let usage = wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING;
        let target = make_texture(&self.device, key.stage, WORKING_FORMAT, usage);

        let (kind, angle, color, ramp, image, image_size) = self.background_inputs(&key.background);
        let values = [
            f32::from_bits(kind),
            angle,
            key.stage.0 as f32,
            key.stage.1 as f32,
            color[0],
            color[1],
            color[2],
            color[3],
            image_size.0,
            image_size.1,
            0.0,
            0.0,
        ];
        let params = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: 48,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.queue.write_buffer(&params, 0, &uniform_bytes(&values));
        let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &self.background_pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: params.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&ramp) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&image) },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::Sampler(&self.sampler) },
            ],
        });
        let target_view = target.create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("background"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.background_pipeline);
            pass.set_bind_group(0, &bind, &[]);
            pass.draw(0..3, 0..1);
        }
        self.queue.submit(Some(encoder.finish()));

        if key.blur <= 0.0 {
            return target;
        }
        // The same blur at every output size.
        let sigma = (key.blur * BLUR_FACTOR * f64::from(key.stage.1) / 1080.0).max(0.5) as f32;
        let horizontal = self.blur_pass(&target, key.stage, sigma, (1.0, 0.0));
        self.blur_pass(&horizontal, key.stage, sigma, (0.0, 1.0))
    }

    fn blur_pass(&self, source: &wgpu::Texture, stage: (u32, u32), sigma: f32, axis: (f32, f32)) -> wgpu::Texture {
        let usage = wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING;
        let target = make_texture(&self.device, stage, WORKING_FORMAT, usage);
        let values = [
            axis.0 / stage.0 as f32,
            axis.1 / stage.1 as f32,
            sigma,
            (sigma * 3.0).ceil().min(64.0),
        ];
        let params = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.queue.write_buffer(&params, 0, &uniform_bytes(&values));
        let source_view = source.create_view(&wgpu::TextureViewDescriptor::default());
        let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &self.blur_pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: params.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&source_view) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&self.sampler) },
            ],
        });
        let target_view = target.create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("blur"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.blur_pipeline);
            pass.set_bind_group(0, &bind, &[]);
            pass.draw(0..3, 0..1);
        }
        self.queue.submit(Some(encoder.finish()));
        target
    }

    fn resolve(&self, media: &MediaRef) -> std::path::PathBuf {
        media.resolve(self.project_folder.as_deref())
    }

    /// What the background shader needs: a kind, an angle, a colour, a ramp and an image.
    fn background_inputs(
        &mut self,
        background: &Background,
    ) -> (u32, f32, [f32; 4], wgpu::TextureView, wgpu::TextureView, (f32, f32)) {
        let solid = |color: [f32; 4], placeholder: &wgpu::TextureView| {
            (0_u32, 0.0, color, placeholder.clone(), placeholder.clone(), (1.0, 1.0))
        };
        let fallback = [0.125, 0.188, 0.247, 1.0];
        match background {
            Background::Color(text) => {
                let color = parse_color(text).unwrap_or_else(|| {
                    self.warnings.push(format!("Couldn't read the background colour {text}."));
                    fallback
                });
                solid(color, &self.placeholder)
            }
            Background::Gradient(text) => match Gradient::parse(text) {
                Some(parsed) => {
                    let ramp = self.gradient_ramp(&parsed);
                    let (kind, angle) = match parsed.shape {
                        Shape::Linear { angle } => (1, angle),
                        Shape::Radial => (2, 0.0),
                    };
                    (kind, angle, fallback, ramp, self.placeholder.clone(), (1.0, 1.0))
                }
                None => {
                    self.warnings.push(format!("Couldn't read the gradient {text}."));
                    solid(fallback, &self.placeholder)
                }
            },
            Background::Bundled(name) => {
                self.warnings.push(format!("The wallpaper {name} isn't available, so a plain colour is shown."));
                solid(fallback, &self.placeholder)
            }
            Background::Image(media) => {
                let path = self.resolve(media);
                match image::open(&path) {
                    Ok(loaded) => self.image_background(&loaded.to_rgba8()),
                    Err(error) => {
                        self.warnings.push(format!("Couldn't open the background {}: {error}.", path.display()));
                        solid(fallback, &self.placeholder)
                    }
                }
            }
            Background::DataUrl(text) => match decode_data_url(text).and_then(|bytes| image::load_from_memory(&bytes).ok()) {
                Some(loaded) => self.image_background(&loaded.to_rgba8()),
                None => {
                    self.warnings.push("Couldn't read the embedded background image.".to_string());
                    solid(fallback, &self.placeholder)
                }
            },
            Background::Video(media) => {
                // Video backgrounds arrive with step 7.1. Until then, say so rather than go quiet.
                self.warnings.push(format!(
                    "Video backgrounds aren't supported yet, so {} isn't shown.",
                    self.resolve(media).display()
                ));
                solid(fallback, &self.placeholder)
            }
        }
    }

    fn image_background(
        &self,
        pixels: &image::RgbaImage,
    ) -> (u32, f32, [f32; 4], wgpu::TextureView, wgpu::TextureView, (f32, f32)) {
        let (width, height) = pixels.dimensions();
        let texture = make_texture(
            &self.device,
            (width, height),
            WORKING_FORMAT,
            wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        );
        self.queue.write_texture(
            texture.as_image_copy(),
            pixels.as_raw(),
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(width * 4), rows_per_image: Some(height) },
            texture.size(),
        );
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        (3, 0.0, [0.0; 4], self.placeholder.clone(), view, (width as f32, height as f32))
    }

    /// A gradient's colours in a strip of texture, which the shader looks up.
    fn gradient_ramp(&self, gradient: &Gradient) -> wgpu::TextureView {
        const WIDTH: u32 = 1024;
        let bytes: Vec<u8> = (0..WIDTH)
            .flat_map(|x| {
                let color = gradient.sample(x as f32 / (WIDTH - 1) as f32);
                color.map(|channel| (channel.clamp(0.0, 1.0) * 255.0).round() as u8)
            })
            .collect();
        let texture = make_texture(
            &self.device,
            (WIDTH, 1),
            WORKING_FORMAT,
            wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        );
        self.queue.write_texture(
            texture.as_image_copy(),
            &bytes,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(WIDTH * 4), rows_per_image: Some(1) },
            texture.size(),
        );
        texture.create_view(&wgpu::TextureViewDescriptor::default())
    }
}

fn decode_data_url(text: &str) -> Option<Vec<u8>> {
    let encoded = text.split_once(";base64,")?.1;
    let mut bytes = Vec::with_capacity(encoded.len() * 3 / 4);
    let mut buffer = 0_u32;
    let mut bits = 0;
    for c in encoded.bytes() {
        let value = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' | b'\n' | b'\r' => continue,
            _ => return None,
        };
        buffer = (buffer << 6) | u32::from(value);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            bytes.push((buffer >> bits) as u8);
            buffer &= (1 << bits) - 1;
        }
    }
    Some(bytes)
}

/// A GPU with no window, for export and tests.
pub struct Headless {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
}

impl Headless {
    /// Finds a GPU, or `None` if this machine has none, as many CI machines don't.
    #[must_use]
    pub fn new() -> Option<Self> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            force_fallback_adapter: false,
            compatible_surface: None,
            apply_limit_buckets: false,
        }))
        .ok()?;
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).ok()?;
        Some(Self { device, queue })
    }
}

/// A texture to render into, whose pixels can be read back.
#[must_use]
pub fn readable_target(device: &wgpu::Device, size: (u32, u32), format: wgpu::TextureFormat) -> wgpu::Texture {
    make_texture(device, size, format, wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC)
}

/// Reads a texture's pixels, tightly packed.
#[must_use]
pub fn read_pixels(device: &wgpu::Device, queue: &wgpu::Queue, texture: &wgpu::Texture) -> Vec<u8> {
    let size = texture.size();
    let row = size.width * 4;
    let stride = row.next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: u64::from(stride * size.height),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(stride), rows_per_image: Some(size.height) },
        },
        size,
    );
    queue.submit(Some(encoder.finish()));
    buffer.slice(..).map_async(wgpu::MapMode::Read, |result| result.expect("reading a texture failed"));
    let _ = device.poll(wgpu::PollType::wait_indefinitely());
    let data = buffer.slice(..).get_mapped_range().expect("the buffer was just mapped");
    let mut pixels = Vec::with_capacity((row * size.height) as usize);
    for line in data.chunks_exact(stride as usize) {
        pixels.extend_from_slice(&line[..row as usize]);
    }
    drop(data);
    buffer.unmap();
    pixels
}

/// An RGBA image as a texture, for tests and stills.
#[must_use]
pub fn texture_from_rgba(device: &wgpu::Device, queue: &wgpu::Queue, size: (u32, u32), pixels: &[u8]) -> wgpu::Texture {
    let texture = make_texture(
        device,
        size,
        WORKING_FORMAT,
        wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
    );
    queue.write_texture(
        texture.as_image_copy(),
        pixels,
        wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(size.0 * 4), rows_per_image: Some(size.1) },
        texture.size(),
    );
    texture
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_urls_decode() {
        assert_eq!(decode_data_url("data:text/plain;base64,aGVsbG8=").unwrap(), b"hello");
        assert_eq!(decode_data_url("no marker"), None);
    }
}
