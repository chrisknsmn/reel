//! Draws the cursor, click effects, annotations, webcam and captions.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};

use reel_core::timeline::{Annotation, AnnotationContent, ArrowDirection, TextStyle};
use reel_motion::camera::Camera;
use reel_motion::captions;
use reel_motion::cursor::CursorShape;
use reel_motion::layout::Layout;
use reel_motion::webcam;
use tiny_skia::{Color, LineCap, LineJoin, Paint, PathBuilder, Pixmap, Stroke, Transform};

use crate::cursors::{self, SPRITE_PER_CURSOR, SPRITE_SIZE};
use crate::gradient::parse_color;
use crate::layers::FrameLayers;
use crate::text::{TextImage, TextPainter};
use crate::{
    FrameInput, Renderer, VideoSource, WORKING_FORMAT, begin_pass, make_texture, shaders,
    uniform_bytes,
};

/// Room for this many things in one frame. Beyond that, later ones are skipped rather than crash.
const SLOTS: u32 = 256;
const SLOT_SIZE: u32 = 256;
/// The size of the shader's `Quad`.
const QUAD_SIZE: u64 = 80;
/// Text and arrows are drawn ahead at these zooms and scaled down, so that a zoom doesn't redraw them
/// on every frame. They're never scaled up, so they stay sharp.
const RASTER_ZOOMS: [f64; 6] = [1.0, 1.25, 1.8, 2.5, 3.5, 5.0];

pub struct Overlay {
    pipeline: wgpu::RenderPipeline,
    texture_layout: wgpu::BindGroupLayout,
    uniform_bind: wgpu::BindGroup,
    uniforms: wgpu::Buffer,
    next_slot: u32,
    /// Images already uploaded, by what they show.
    textures: HashMap<String, CachedTexture>,
    text: Option<TextPainter>,
    /// Caption pages for a set of layers and a stage width.
    pages: Option<(u64, u32, Vec<captions::Page>)>,
}

#[derive(Clone)]
struct CachedTexture {
    view: wgpu::TextureView,
    size: (u32, u32),
}

/// One thing to draw.
enum Step {
    Quad {
        slot: u32,
        bind: wgpu::BindGroup,
    },
    /// A blurred window onto what's been drawn so far, then an optional solid fill over it.
    Blur {
        slot: u32,
        sigma: f32,
        fill: Option<u32>,
    },
    Webcam,
}

/// A frame's overlays, ready to go.
pub struct Plan {
    pub needs_snapshot: bool,
    steps: Vec<Step>,
    after_webcam: Vec<Step>,
    webcam_bind: Option<wgpu::BindGroup>,
    blank_bind: wgpu::BindGroup,
}

#[derive(Clone, Copy)]
struct QuadParams {
    center: (f64, f64),
    half: (f64, f64),
    rotation: f64,
    uv: [f32; 4],
    color: [f32; 4],
    kind: f32,
    radius: f32,
    stroke: f32,
}

impl Overlay {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let uniform_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: true,
                    min_binding_size: wgpu::BufferSize::new(QUAD_SIZE),
                },
                count: None,
            }],
        });
        let texture_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[Some(&uniform_layout), Some(&texture_layout)],
            immediate_size: 0,
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: None,
            source: wgpu::ShaderSource::Wgsl(shaders::QUAD.into()),
        });
        let premultiplied = wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::One,
            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
            operation: wgpu::BlendOperation::Add,
        };
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("overlay"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleStrip,
                ..wgpu::PrimitiveState::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState {
                        color: premultiplied,
                        alpha: premultiplied,
                    }),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("overlay"),
            size: u64::from(SLOTS * SLOT_SIZE),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let uniform_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &uniform_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &uniforms,
                    offset: 0,
                    size: wgpu::BufferSize::new(QUAD_SIZE),
                }),
            }],
        });
        Self {
            pipeline,
            texture_layout,
            uniform_bind,
            uniforms,
            next_slot: 0,
            textures: HashMap::new(),
            text: None,
            pages: None,
        }
    }
}

fn arrow_paths(direction: ArrowDirection) -> &'static [[(f32, f32); 2]; 3] {
    match direction {
        ArrowDirection::Up => &[
            [(50.0, 20.0), (50.0, 80.0)],
            [(50.0, 20.0), (35.0, 35.0)],
            [(50.0, 20.0), (65.0, 35.0)],
        ],
        ArrowDirection::Down => &[
            [(50.0, 20.0), (50.0, 80.0)],
            [(50.0, 80.0), (35.0, 65.0)],
            [(50.0, 80.0), (65.0, 65.0)],
        ],
        ArrowDirection::Left => &[
            [(80.0, 50.0), (20.0, 50.0)],
            [(20.0, 50.0), (35.0, 35.0)],
            [(20.0, 50.0), (35.0, 65.0)],
        ],
        ArrowDirection::Right => &[
            [(20.0, 50.0), (80.0, 50.0)],
            [(80.0, 50.0), (65.0, 35.0)],
            [(80.0, 50.0), (65.0, 65.0)],
        ],
        ArrowDirection::UpRight => &[
            [(25.0, 75.0), (75.0, 25.0)],
            [(75.0, 25.0), (60.0, 30.0)],
            [(75.0, 25.0), (70.0, 40.0)],
        ],
        ArrowDirection::UpLeft => &[
            [(75.0, 75.0), (25.0, 25.0)],
            [(25.0, 25.0), (40.0, 30.0)],
            [(25.0, 25.0), (30.0, 40.0)],
        ],
        ArrowDirection::DownRight => &[
            [(25.0, 25.0), (75.0, 75.0)],
            [(75.0, 75.0), (70.0, 60.0)],
            [(75.0, 75.0), (60.0, 70.0)],
        ],
        ArrowDirection::DownLeft => &[
            [(75.0, 25.0), (25.0, 75.0)],
            [(25.0, 75.0), (30.0, 60.0)],
            [(25.0, 75.0), (40.0, 70.0)],
        ],
    }
}

/// Draws an arrow into its own image, from 100×100 paths.
fn arrow_image(
    direction: ArrowDirection,
    color: [f32; 4],
    stroke_width: f64,
    size: (u32, u32),
) -> Vec<u8> {
    let (width, height) = (size.0.max(1), size.1.max(1));
    let mut pixmap = Pixmap::new(width, height).expect("the size is at least one pixel");
    let scale = (width.min(height) as f32) / 100.0;
    let offset = (
        (width as f32 - 100.0 * scale) / 2.0,
        (height as f32 - 100.0 * scale) / 2.0,
    );
    let mut paint = Paint {
        anti_alias: true,
        ..Paint::default()
    };
    paint.set_color(
        Color::from_rgba(color[0], color[1], color[2], color[3]).unwrap_or(Color::WHITE),
    );
    let stroke = Stroke {
        width: stroke_width as f32 * scale,
        line_cap: LineCap::Round,
        line_join: LineJoin::Round,
        ..Stroke::default()
    };
    for segment in arrow_paths(direction) {
        let mut builder = PathBuilder::new();
        builder.move_to(
            offset.0 + segment[0].0 * scale,
            offset.1 + segment[0].1 * scale,
        );
        builder.line_to(
            offset.0 + segment[1].0 * scale,
            offset.1 + segment[1].1 * scale,
        );
        if let Some(path) = builder.finish() {
            pixmap.stroke_path(&path, &paint, &stroke, Transform::identity(), None);
        }
    }
    pixmap.data().to_vec()
}

/// The smallest zoom to draw at that won't need scaling up.
fn raster_zoom(zoom: f64) -> f64 {
    RASTER_ZOOMS
        .iter()
        .copied()
        .find(|step| *step >= zoom)
        .unwrap_or(zoom)
}

fn hash_of(text: &str) -> u64 {
    let mut hasher = std::hash::DefaultHasher::new();
    text.hash(&mut hasher);
    hasher.finish()
}

fn shape_key(shape: CursorShape) -> u8 {
    shape as u8
}

fn premultiply(pixels: &mut [u8]) {
    for pixel in pixels.chunks_exact_mut(4) {
        let alpha = u32::from(pixel[3]);
        for channel in &mut pixel[..3] {
            *channel = (u32::from(*channel) * alpha / 255) as u8;
        }
    }
}

impl Renderer {
    fn upload(&mut self, key: String, size: (u32, u32), rgba: &[u8]) -> CachedTexture {
        let texture = make_texture(
            &self.device,
            size,
            WORKING_FORMAT,
            wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        );
        self.queue.write_texture(
            texture.as_image_copy(),
            rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(size.0 * 4),
                rows_per_image: Some(size.1),
            },
            texture.size(),
        );
        let cached = CachedTexture {
            view: texture.create_view(&wgpu::TextureViewDescriptor::default()),
            size,
        };
        // Plenty for any frame, with a stop so a long session can't grow without limit.
        if self.overlay.textures.len() > 128 {
            self.overlay.textures.clear();
        }
        self.overlay.textures.insert(key, cached.clone());
        cached
    }

    fn cached_or(
        &mut self,
        key: String,
        make: impl FnOnce(&mut Self) -> (Vec<u8>, (u32, u32)),
    ) -> CachedTexture {
        if let Some(found) = self.overlay.textures.get(&key) {
            return found.clone();
        }
        let (rgba, size) = make(self);
        self.upload(key, size, &rgba)
    }

    fn texture_bind(&self, view: &wgpu::TextureView) -> wgpu::BindGroup {
        self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &self.overlay.texture_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        })
    }

    fn take_slot(&mut self, stage: (u32, u32), params: QuadParams) -> Option<u32> {
        if self.overlay.next_slot >= SLOTS {
            return None;
        }
        let slot = self.overlay.next_slot;
        self.overlay.next_slot += 1;
        let (sin, cos) = params.rotation.sin_cos();
        let values = [
            params.center.0 as f32,
            params.center.1 as f32,
            params.half.0 as f32,
            params.half.1 as f32,
            cos as f32,
            sin as f32,
            stage.0 as f32,
            stage.1 as f32,
            params.uv[0],
            params.uv[1],
            params.uv[2],
            params.uv[3],
            params.color[0],
            params.color[1],
            params.color[2],
            params.color[3],
            params.kind,
            params.radius,
            params.stroke,
            0.0,
        ];
        self.queue.write_buffer(
            &self.overlay.uniforms,
            u64::from(slot * SLOT_SIZE),
            &uniform_bytes(&values),
        );
        Some(slot)
    }

    /// Prepares a frame's overlays, uploading anything new. Nothing is drawn yet.
    pub(crate) fn plan_overlays(
        &mut self,
        input: &FrameInput<'_>,
        layout: &Layout,
        stage: (u32, u32),
    ) -> Plan {
        self.overlay.next_slot = 0;
        let blank_bind = self.texture_bind(&self.placeholder.clone());
        let mut steps = Vec::new();
        let mut after_webcam = Vec::new();
        let mut webcam_bind = None;
        let mut needs_snapshot = false;

        if let Some(layers) = &input.layers {
            self.plan_pointer(layers, input.camera, layout, stage, &mut steps);
            let mut annotations = layers.annotations.clone();
            annotations.sort_by_key(|item| item.layer);
            for annotation in annotations {
                needs_snapshot |=
                    self.plan_annotation(annotation, input.camera, layout, stage, &mut steps);
            }
            // Outside the camera, so these stay put through a zoom.
            if let Some((video, size)) = layers.webcam {
                webcam_bind = Some(self.prepare_webcam(layers, video, size, stage));
                steps.push(Step::Webcam);
            }
            self.plan_caption(layers, stage, &mut after_webcam);
        }
        Plan {
            needs_snapshot,
            steps,
            after_webcam,
            webcam_bind,
            blank_bind,
        }
    }

    fn plan_pointer(
        &mut self,
        layers: &FrameLayers<'_>,
        camera: Camera,
        layout: &Layout,
        stage: (u32, u32),
        steps: &mut Vec<Step>,
    ) {
        let look = &layers.source.cursor;
        let frame = layout.frame;
        let height = 28.0 * look.size * (frame.width / 1920.0);
        let to_output = |x: f64, y: f64| -> (f64, f64) {
            let stage_x = layout.video.x + x * layout.video.width;
            let stage_y = layout.video.y + y * layout.video.height;
            (
                stage_x * camera.scale + camera.x * f64::from(stage.0),
                stage_y * camera.scale + camera.y * f64::from(stage.1),
            )
        };

        // Click effects go under the cursor.
        let color = parse_color(&look.click_effect_color).unwrap_or([0.145, 0.388, 0.922, 1.0]);
        for ring in layers.source.pointer.rings_at(layers.time, height) {
            let center = to_output(ring.x, ring.y);
            let radius = ring.radius * camera.scale;
            let stroke = ring.stroke * camera.scale;
            let reach = radius + stroke + 2.0;
            let params = QuadParams {
                center,
                half: (reach, reach),
                rotation: 0.0,
                uv: [0.0, 0.0, 1.0, 1.0],
                color: [color[0], color[1], color[2], ring.alpha as f32],
                kind: 1.0,
                radius: radius as f32,
                stroke: stroke as f32,
            };
            if let Some(slot) = self.take_slot(stage, params) {
                steps.push(Step::Quad {
                    slot,
                    bind: self.texture_bind(&self.placeholder.clone()),
                });
            }
        }

        if layers.pointer.is_empty() || !layers.pointer.iter().any(|pose| pose.visible) {
            return;
        }
        let style = look.style.clone();
        let copies = layers.pointer.len() as f64;
        // Enough per copy that together they build up to the cursor's usual 0.95.
        let alpha = 1.0 - (1.0 - 0.95_f64).powf(1.0 / copies);
        let shadow_alpha = 1.0 - (1.0 - 0.35_f64).powf(1.0 / copies);

        for pose in layers.pointer.clone() {
            if !pose.visible {
                continue;
            }
            let key = format!("cursor:{style}:{}", shape_key(pose.shape));
            let shadow_key = format!("{key}:shadow");
            if !self.overlay.textures.contains_key(&key) {
                let sprite = cursors::sprite(&style, pose.shape);
                let size = (SPRITE_SIZE, SPRITE_SIZE);
                self.upload(key.clone(), size, &sprite.rgba);
                self.upload(shadow_key.clone(), size, &sprite.shadow);
            }
            let hotspot = cursors::sprite_hotspot(&style, pose.shape);
            let sprite_size = height * SPRITE_PER_CURSOR * pose.scale * camera.scale;
            let half = sprite_size / 2.0;
            let tip = to_output(pose.x, pose.y);
            // The hotspot, not the middle of the image, sits on the true position, even when rotated.
            let (dx, dy) = (
                (0.5 - hotspot.0) * sprite_size,
                (0.5 - hotspot.1) * sprite_size,
            );
            let (sin, cos) = pose.rotation.sin_cos();
            let center = (tip.0 + dx * cos - dy * sin, tip.1 + dx * sin + dy * cos);

            if cursors::has_shadow(&style) {
                let shadow_view = self.overlay.textures[&shadow_key].view.clone();
                let params = QuadParams {
                    center: (
                        center.0,
                        center.1 + 2.0 * camera.scale * (height / 28.0).max(0.5),
                    ),
                    half: (half, half),
                    rotation: pose.rotation,
                    uv: [0.0, 0.0, 1.0, 1.0],
                    color: [1.0, 1.0, 1.0, shadow_alpha as f32],
                    kind: 0.0,
                    radius: 0.0,
                    stroke: 0.0,
                };
                if let Some(slot) = self.take_slot(stage, params) {
                    steps.push(Step::Quad {
                        slot,
                        bind: self.texture_bind(&shadow_view),
                    });
                }
            }
            let view = self.overlay.textures[&key].view.clone();
            let params = QuadParams {
                center,
                half: (half, half),
                rotation: pose.rotation,
                uv: [0.0, 0.0, 1.0, 1.0],
                color: [1.0, 1.0, 1.0, alpha as f32],
                kind: 0.0,
                radius: 0.0,
                stroke: 0.0,
            };
            if let Some(slot) = self.take_slot(stage, params) {
                steps.push(Step::Quad {
                    slot,
                    bind: self.texture_bind(&view),
                });
            }
        }
    }

    /// Adds an annotation's drawing, and returns whether it needs to read what's been drawn.
    fn plan_annotation(
        &mut self,
        item: &Annotation,
        camera: Camera,
        layout: &Layout,
        stage: (u32, u32),
        steps: &mut Vec<Step>,
    ) -> bool {
        let frame = layout.frame;
        // Positions and sizes are percentages of the recording area, from the top-left corner.
        let x = frame.x + item.position.x / 100.0 * frame.width;
        let y = frame.y + item.position.y / 100.0 * frame.height;
        let (width, height) = (
            item.width / 100.0 * frame.width,
            item.height / 100.0 * frame.height,
        );
        let scale = frame.width / 1920.0;
        let center = (
            (x + width / 2.0) * camera.scale + camera.x * f64::from(stage.0),
            (y + height / 2.0) * camera.scale + camera.y * f64::from(stage.1),
        );
        let half = (width / 2.0 * camera.scale, height / 2.0 * camera.scale);
        let raster = raster_zoom(camera.scale);
        let pixel_size = (
            (width * raster).ceil().max(1.0) as u32,
            (height * raster).ceil().max(1.0) as u32,
        );
        let texture_quad = QuadParams {
            center,
            half,
            rotation: 0.0,
            uv: [0.0, 0.0, 1.0, 1.0],
            color: [1.0, 1.0, 1.0, 1.0],
            kind: 0.0,
            radius: 0.0,
            stroke: 0.0,
        };

        match &item.content {
            AnnotationContent::Text { text, style } => {
                let key = format!("text:{}:{pixel_size:?}:{text}:{style:?}", item.id.0);
                let font_scale = (scale * raster) as f32;
                let (text, style) = (text.clone(), style.clone());
                let cached = self.cached_or(key, move |renderer| {
                    let image = renderer.paint_text(&text, &style, pixel_size, font_scale);
                    (image.rgba, (image.width, image.height))
                });
                if let Some(slot) = self.take_slot(stage, texture_quad) {
                    steps.push(Step::Quad {
                        slot,
                        bind: self.texture_bind(&cached.view),
                    });
                }
                false
            }
            AnnotationContent::Arrow {
                direction,
                color,
                stroke_width,
            } => {
                let parsed = parse_color(color).unwrap_or([0.145, 0.388, 0.922, 1.0]);
                let key = format!("arrow:{direction:?}:{color}:{stroke_width}:{pixel_size:?}");
                let (direction, stroke_width) = (*direction, *stroke_width);
                let cached = self.cached_or(key, move |_| {
                    (
                        arrow_image(direction, parsed, stroke_width, pixel_size),
                        pixel_size,
                    )
                });
                if let Some(slot) = self.take_slot(stage, texture_quad) {
                    steps.push(Step::Quad {
                        slot,
                        bind: self.texture_bind(&cached.view),
                    });
                }
                false
            }
            AnnotationContent::Image { data_url } => {
                let key = format!("image:{}", hash_of(data_url));
                let data = data_url.clone();
                let cached = self.cached_or(key, move |_| {
                    let decoded = crate::decode_data_url(&data)
                        .and_then(|bytes| image::load_from_memory(&bytes).ok());
                    match decoded {
                        Some(loaded) => {
                            let mut pixels = loaded.to_rgba8();
                            let size = pixels.dimensions();
                            premultiply(&mut pixels);
                            (pixels.into_raw(), size)
                        }
                        None => (vec![0, 0, 0, 0], (1, 1)),
                    }
                });
                // An image keeps its shape inside its box.
                let fit = (half.0 * 2.0 / f64::from(cached.size.0))
                    .min(half.1 * 2.0 / f64::from(cached.size.1));
                let quad = QuadParams {
                    half: (
                        f64::from(cached.size.0) * fit / 2.0,
                        f64::from(cached.size.1) * fit / 2.0,
                    ),
                    ..texture_quad
                };
                if let Some(slot) = self.take_slot(stage, quad) {
                    steps.push(Step::Quad {
                        slot,
                        bind: self.texture_bind(&cached.view),
                    });
                }
                false
            }
            AnnotationContent::Blur { strength, fill } => {
                let window = QuadParams {
                    kind: 2.0,
                    ..texture_quad
                };
                let Some(slot) = self.take_slot(stage, window) else {
                    return false;
                };
                let fill_slot = fill.as_deref().and_then(parse_color).and_then(|color| {
                    // A solid colour for censoring goes on top, as a filled ring big enough to cover.
                    let reach = half.0.hypot(half.1) + 2.0;
                    self.take_slot(
                        stage,
                        QuadParams {
                            half,
                            color,
                            kind: 1.0,
                            radius: reach as f32,
                            stroke: 0.0,
                            ..texture_quad
                        },
                    )
                });
                let sigma = (strength * scale * camera.scale).max(0.5) as f32;
                steps.push(Step::Blur {
                    slot,
                    sigma,
                    fill: fill_slot,
                });
                true
            }
        }
    }

    pub(crate) fn paint_text(
        &mut self,
        text: &str,
        style: &TextStyle,
        size: (u32, u32),
        font_scale: f32,
    ) -> TextImage {
        let painter = self.overlay.text.get_or_insert_with(TextPainter::new);
        painter.paint_box(text, style, size, font_scale)
    }

    fn prepare_webcam(
        &mut self,
        layers: &FrameLayers<'_>,
        video: VideoSource<'_>,
        size: (u32, u32),
        stage: (u32, u32),
    ) -> wgpu::BindGroup {
        let stage_f = (f64::from(stage.0), f64::from(stage.1));
        let scale = (stage_f.0 / 1920.0).min(stage_f.1 / 1080.0);
        let settings = &layers.source.webcam;
        let bubble = webcam::layout(
            settings,
            stage_f,
            (f64::from(size.0), f64::from(size.1)),
            layers.zoom,
            scale,
        );
        let kind = match video {
            VideoSource::Rgba(_) => 0.0,
            VideoSource::Nv12 { .. } => 1.0,
        };
        let rect = |r: &reel_motion::layout::Rect| {
            [r.x as f32, r.y as f32, r.width as f32, r.height as f32]
        };
        let (offset, blur, alpha) = bubble.shadow;
        let mut values: Vec<f32> = vec![1.0, 0.0, 0.0, 0.0];
        values.extend(rect(&bubble.frame));
        values.extend(rect(&bubble.video));
        values.extend([bubble.radius as f32, kind, scale as f32, 0.0]);
        values.extend([offset as f32, (blur / 2.0) as f32, alpha as f32, 0.0]);
        values.extend([0.0; 8]);
        values.extend([1.0, 0.0, 0.0, 0.0]);
        self.queue
            .write_buffer(&self.webcam_params, 0, &uniform_bytes(&values));
        self.frame_bind(&self.webcam_params, video)
    }

    fn plan_caption(&mut self, layers: &FrameLayers<'_>, stage: (u32, u32), steps: &mut Vec<Step>) {
        let style = &layers.source.caption_style;
        if !style.show || layers.source.captions.is_empty() {
            return;
        }
        let stage_f = (f64::from(stage.0), f64::from(stage.1));
        let metrics = captions::metrics(style, stage_f.0);
        let text_style = TextStyle {
            color: style.text_color.clone(),
            font_family: style.font_family.clone(),
            bold: false,
            ..TextStyle::default()
        };
        let revision = layers.source.revision;
        if self
            .overlay
            .pages
            .as_ref()
            .is_none_or(|(seen, width, _)| (*seen, *width) != (revision, stage.0))
        {
            let painter = self.overlay.text.get_or_insert_with(TextPainter::new);
            let font_px = metrics.font_px as f32;
            let inner = metrics.max_width - metrics.padding.0 * 2.0;
            let mut measure = |text: &str| f64::from(painter.measure(text, &text_style, font_px));
            let pages = captions::pages(&layers.source.captions, style, inner, &mut measure);
            self.overlay.pages = Some((revision, stage.0, pages));
        }
        let page = self
            .overlay
            .pages
            .as_ref()
            .and_then(|(_, _, pages)| captions::page_at(pages, layers.time))
            .cloned();
        let Some(page) = page else { return };

        let key = format!(
            "caption:{}:{:?}:{}:{}:{}",
            stage.0, page.lines, style.background_opacity, style.text_color, style.font_family,
        );
        let font = metrics.font_px as f32;
        let padding = (metrics.padding.0 as f32, metrics.padding.1 as f32);
        let radius = metrics.radius as f32;
        let alpha = style.background_opacity as f32;
        let lines = page.lines.clone();
        let paint_style = text_style.clone();
        let cached = self.cached_or(key, move |renderer| {
            let painter = renderer.overlay.text.get_or_insert_with(TextPainter::new);
            let image = painter.paint_caption(&lines, &paint_style, font, padding, radius, alpha);
            (image.rgba, (image.width, image.height))
        });

        let motion = captions::page_motion(&page, layers.time, style.animation);
        let scale = stage_f.1 / 1080.0;
        let (width, height) = (
            f64::from(cached.size.0) * motion.scale,
            f64::from(cached.size.1) * motion.scale,
        );
        let bottom = stage_f.1 * (1.0 - style.bottom_offset / 100.0);
        let center = (
            stage_f.0 / 2.0,
            bottom - height / 2.0 + motion.translate_y * scale,
        );
        let params = QuadParams {
            center,
            half: (width / 2.0, height / 2.0),
            rotation: 0.0,
            uv: [0.0, 0.0, 1.0, 1.0],
            color: [1.0, 1.0, 1.0, motion.opacity as f32],
            kind: 0.0,
            radius: 0.0,
            stroke: 0.0,
        };
        if let Some(slot) = self.take_slot(stage, params) {
            steps.push(Step::Quad {
                slot,
                bind: self.texture_bind(&cached.view),
            });
        }
    }

    pub(crate) fn scene_texture(&self, stage: (u32, u32)) -> wgpu::Texture {
        make_texture(
            &self.device,
            stage,
            self.target_format,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        )
    }

    /// A blurred copy of what's been drawn, recorded into the same commands so the order is right.
    pub(crate) fn blurred_copy(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        scene: &wgpu::Texture,
        stage: (u32, u32),
        sigma: f32,
    ) -> wgpu::TextureView {
        let usage = wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING;
        let blur_pass = |encoder: &mut wgpu::CommandEncoder,
                         source: &wgpu::TextureView,
                         axis: (f32, f32)| {
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
            let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: &self.blur_pipeline.get_bind_group_layout(0),
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: params.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(source),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                ],
            });
            let view = target.create_view(&wgpu::TextureViewDescriptor::default());
            {
                let mut pass = begin_pass(encoder, &view, wgpu::LoadOp::Clear(wgpu::Color::BLACK));
                pass.set_pipeline(&self.blur_pipeline);
                pass.set_bind_group(0, &bind, &[]);
                pass.draw(0..3, 0..1);
            }
            view
        };
        let scene_view = scene.create_view(&wgpu::TextureViewDescriptor::default());
        let horizontal = blur_pass(encoder, &scene_view, (1.0, 0.0));
        blur_pass(encoder, &horizontal, (0.0, 1.0))
    }

    /// Draws a frame's overlays.
    pub(crate) fn run_overlays(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        scene: Option<&wgpu::Texture>,
        stage: (u32, u32),
        plan: &Plan,
    ) {
        let all = plan.steps.iter().chain(&plan.after_webcam);
        let mut index = 0;
        let steps: Vec<&Step> = all.collect();
        while index < steps.len() {
            // Draw everything up to the next blur in one pass.
            {
                let mut pass = begin_pass(encoder, target, wgpu::LoadOp::Load);
                while index < steps.len() {
                    match steps[index] {
                        Step::Quad { slot, bind } => {
                            pass.set_pipeline(&self.overlay.pipeline);
                            pass.set_bind_group(0, &self.overlay.uniform_bind, &[slot * SLOT_SIZE]);
                            pass.set_bind_group(1, bind, &[]);
                            pass.draw(0..4, 0..1);
                        }
                        Step::Webcam => {
                            if let Some(bind) = &plan.webcam_bind {
                                pass.set_pipeline(&self.frame_pipeline);
                                pass.set_bind_group(0, bind, &[]);
                                pass.draw(0..3, 0..1);
                            }
                        }
                        Step::Blur { .. } => break,
                    }
                    index += 1;
                }
            }
            if let Some(Step::Blur { slot, sigma, fill }) = steps.get(index).copied() {
                if let Some(scene) = scene {
                    let blurred = self.blurred_copy(encoder, scene, stage, *sigma);
                    let bind = self.texture_bind(&blurred);
                    let mut pass = begin_pass(encoder, target, wgpu::LoadOp::Load);
                    pass.set_pipeline(&self.overlay.pipeline);
                    pass.set_bind_group(0, &self.overlay.uniform_bind, &[slot * SLOT_SIZE]);
                    pass.set_bind_group(1, &bind, &[]);
                    pass.draw(0..4, 0..1);
                    if let Some(fill) = fill {
                        pass.set_bind_group(0, &self.overlay.uniform_bind, &[fill * SLOT_SIZE]);
                        pass.set_bind_group(1, &plan.blank_bind, &[]);
                        pass.draw(0..4, 0..1);
                    }
                }
                index += 1;
            }
        }
    }
}
