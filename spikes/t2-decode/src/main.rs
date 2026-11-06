//! T2 from BUILD_PLAN.md: hardware-decode H.264 up to 4K60 into a wgpu texture, comparing FFmpeg's
//! library with native AVFoundation (D5). Runs headless, so it needs no window or permissions.
//!
//! Usage: t2-decode <video> [report.json]

use std::ptr::NonNull;
use std::sync::mpsc::{Receiver, TryRecvError, sync_channel};
use std::time::{Duration, Instant};

use ffmpeg_next as ffmpeg;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_av_foundation::{
    AVAssetReader, AVAssetReaderTrackOutput, AVMediaTypeVideo, AVURLAsset,
};
use objc2_core_foundation::CFRetained;
use objc2_core_media::{CMTime, CMTimeRange, kCMTimePositiveInfinity};
use objc2_core_video::{
    CVMetalTexture, CVMetalTextureCache, CVMetalTextureGetTexture, CVPixelBuffer,
    CVPixelBufferGetBaseAddressOfPlane, CVPixelBufferGetBytesPerRowOfPlane,
    CVPixelBufferLockBaseAddress,
    CVPixelBufferLockFlags, CVPixelBufferUnlockBaseAddress,
    kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange, kCVReturnSuccess,
};
use objc2_foundation::{NSDictionary, NSNumber, NSString, NSURL};
use objc2_metal::{MTLPixelFormat, MTLTextureType};
use serde_json::{Value, json};

type Metal = wgpu::hal::api::Metal;

// ---- Decoders ----

/// A decoded frame, always in the same hardware-backed form so both decoders can be compared fairly.
struct Frame {
    pts: f64,
    buffer: CFRetained<CVPixelBuffer>,
}

// SAFETY: a CVPixelBuffer is reference counted and safe to hand to another thread once decoded.
unsafe impl Send for Frame {}

trait Decoder {
    fn next(&mut self) -> Option<Frame>;
    /// Positions the decoder and returns the first frame at or after `seconds`.
    fn seek(&mut self, seconds: f64) -> Option<Frame>;
}

struct NativeDecoder {
    asset: Retained<AVURLAsset>,
    output: Retained<AVAssetReaderTrackOutput>,
    reader: Retained<AVAssetReader>,
}

fn native_settings() -> Retained<NSDictionary<NSString, AnyObject>> {
    let format = NSNumber::new_u32(kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange);
    let metal = NSNumber::new_bool(true);
    let format_key = NSString::from_str("PixelFormatType");
    let metal_key = NSString::from_str("MetalCompatibility");
    let objects: [&AnyObject; 2] = [format.as_ref(), metal.as_ref()];
    NSDictionary::from_slices(&[&*format_key, &*metal_key], &objects)
}

impl NativeDecoder {
    fn open(path: &str) -> Self {
        let url = NSURL::fileURLWithPath(&NSString::from_str(path));
        // SAFETY: documented constructor, with a valid URL and no options.
        let asset = unsafe { AVURLAsset::URLAssetWithURL_options(&url, None) };
        let (reader, output) = Self::reader(&asset, None);
        Self { asset, output, reader }
    }

    fn reader(
        asset: &AVURLAsset,
        start: Option<f64>,
    ) -> (Retained<AVAssetReader>, Retained<AVAssetReaderTrackOutput>) {
        // SAFETY: documented AVFoundation calls with valid objects. Output settings are a plain
        // dictionary of the documented key and value types.
        unsafe {
            let reader = AVAssetReader::assetReaderWithAsset_error(asset).expect("no asset reader");
            #[allow(deprecated)]
            let tracks = asset.tracksWithMediaType(AVMediaTypeVideo.unwrap());
            let track = tracks.firstObject().expect("no video track");
            let output = AVAssetReaderTrackOutput::assetReaderTrackOutputWithTrack_outputSettings(
                &track,
                Some(&native_settings()),
            );
            output.setAlwaysCopiesSampleData(false);
            reader.addOutput(&output);
            if let Some(start) = start {
                reader.setTimeRange(CMTimeRange {
                    start: CMTime::with_seconds(start, 600),
                    duration: kCMTimePositiveInfinity,
                });
            }
            assert!(reader.startReading(), "AVAssetReader wouldn't start");
            (reader, output)
        }
    }
}

impl Decoder for NativeDecoder {
    fn next(&mut self) -> Option<Frame> {
        objc2::rc::autoreleasepool(|_| {
            // SAFETY: documented calls on objects this decoder owns. The image buffer is retained.
            unsafe {
                let sample = self.output.copyNextSampleBuffer()?;
                let buffer = sample.image_buffer()?;
                let pts = sample.output_presentation_time_stamp().seconds();
                Some(Frame { pts, buffer })
            }
        })
    }

    fn seek(&mut self, seconds: f64) -> Option<Frame> {
        // SAFETY: cancels a reader this decoder owns before replacing it.
        unsafe { self.reader.cancelReading() };
        let (reader, output) = Self::reader(&self.asset, Some(seconds));
        self.reader = reader;
        self.output = output;
        loop {
            let frame = self.next()?;
            if frame.pts >= seconds - 0.001 {
                return Some(frame);
            }
        }
    }
}

struct FfmpegDecoder {
    input: ffmpeg::format::context::Input,
    decoder: ffmpeg::decoder::Video,
    stream: usize,
    time_base: f64,
    frame: ffmpeg::frame::Video,
    drained: bool,
}

unsafe extern "C" fn choose_videotoolbox(
    _context: *mut ffmpeg::ffi::AVCodecContext,
    _formats: *const ffmpeg::ffi::AVPixelFormat,
) -> ffmpeg::ffi::AVPixelFormat {
    ffmpeg::ffi::AVPixelFormat::AV_PIX_FMT_VIDEOTOOLBOX
}

impl FfmpegDecoder {
    fn open(path: &str, hardware: bool) -> Self {
        ffmpeg::init().expect("ffmpeg init");
        ffmpeg::log::set_level(ffmpeg::log::Level::Error);
        let input = ffmpeg::format::input(&path).expect("couldn't open the video");
        let stream = input.streams().best(ffmpeg::media::Type::Video).expect("no video stream");
        let index = stream.index();
        let time_base = f64::from(stream.time_base());
        let mut context = ffmpeg::codec::context::Context::from_parameters(stream.parameters())
            .expect("no codec context");
        if hardware {
            // SAFETY: standard libavcodec hwaccel setup. The context is valid and not yet opened.
            unsafe {
                let mut device = std::ptr::null_mut();
                let result = ffmpeg::ffi::av_hwdevice_ctx_create(
                    &mut device,
                    ffmpeg::ffi::AVHWDeviceType::AV_HWDEVICE_TYPE_VIDEOTOOLBOX,
                    std::ptr::null(),
                    std::ptr::null_mut(),
                    0,
                );
                assert_eq!(result, 0, "couldn't create the VideoToolbox device");
                let raw = context.as_mut_ptr();
                (*raw).hw_device_ctx = ffmpeg::ffi::av_buffer_ref(device);
                (*raw).get_format = Some(choose_videotoolbox);
                ffmpeg::ffi::av_buffer_unref(&mut device);
            }
        }
        let decoder = context.decoder().video().expect("couldn't open the decoder");
        Self {
            input,
            decoder,
            stream: index,
            time_base,
            frame: ffmpeg::frame::Video::empty(),
            drained: false,
        }
    }

    /// Decodes one frame into `self.frame`, returning its timestamp.
    fn advance(&mut self) -> Option<f64> {
        loop {
            if self.decoder.receive_frame(&mut self.frame).is_ok() {
                return Some(self.frame.pts().unwrap_or(0) as f64 * self.time_base);
            }
            if self.drained {
                return None;
            }
            let mut sent = false;
            for (stream, packet) in self.input.packets() {
                if stream.index() == self.stream {
                    self.decoder.send_packet(&packet).ok()?;
                    sent = true;
                    break;
                }
            }
            if !sent {
                self.decoder.send_eof().ok()?;
                self.drained = true;
            }
        }
    }

    fn to_frame(&self, pts: f64) -> Option<Frame> {
        // SAFETY: for AV_PIX_FMT_VIDEOTOOLBOX, FFmpeg documents data[3] as the CVPixelBufferRef.
        // `retain` keeps it alive after the AVFrame moves on.
        unsafe {
            let raw = (*self.frame.as_ptr()).data[3].cast::<CVPixelBuffer>();
            let buffer = CFRetained::retain(NonNull::new(raw)?);
            Some(Frame { pts, buffer })
        }
    }

    fn seek_to(&mut self, seconds: f64) -> Option<f64> {
        let timestamp = (seconds * f64::from(ffmpeg::ffi::AV_TIME_BASE)) as i64;
        self.input.seek(timestamp, ..timestamp).ok()?;
        self.decoder.flush();
        self.drained = false;
        loop {
            let pts = self.advance()?;
            if pts >= seconds - 0.001 {
                return Some(pts);
            }
        }
    }
}

impl Decoder for FfmpegDecoder {
    fn next(&mut self) -> Option<Frame> {
        let pts = self.advance()?;
        self.to_frame(pts)
    }

    fn seek(&mut self, seconds: f64) -> Option<Frame> {
        let pts = self.seek_to(seconds)?;
        self.to_frame(pts)
    }
}

// ---- GPU ----

const SHADER: &str = r#"
@group(0) @binding(0) var luma: texture_2d<f32>;
@group(0) @binding(1) var chroma: texture_2d<f32>;
@group(0) @binding(2) var linear: sampler;

struct Out { @builtin(position) position: vec4f, @location(0) uv: vec2f };

@vertex
fn vs(@builtin(vertex_index) i: u32) -> Out {
    let x = f32(i32(i & 1u) * 4 - 1);
    let y = f32(i32(i >> 1u) * 4 - 1);
    return Out(vec4f(x, y, 0.0, 1.0), vec2f((x + 1.0) / 2.0, (1.0 - y) / 2.0));
}

// BT.709, limited range.
@fragment
fn fs(in: Out) -> @location(0) vec4f {
    let y = (textureSampleLevel(luma, linear, in.uv, 0.0).r - 16.0 / 255.0) * 255.0 / 219.0;
    let uv = (textureSampleLevel(chroma, linear, in.uv, 0.0).rg - vec2f(128.0 / 255.0)) * 255.0 / 224.0;
    return vec4f(
        y + 1.5748 * uv.y,
        y - 0.1873 * uv.x - 0.4681 * uv.y,
        y + 1.8556 * uv.x,
        1.0,
    );
}
"#;

struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::RenderPipeline,
    sampler: wgpu::Sampler,
    target: wgpu::Texture,
    target_view: wgpu::TextureView,
    width: u32,
    height: u32,
    luma: wgpu::Texture,
    chroma: wgpu::Texture,
    cache: CFRetained<CVMetalTextureCache>,
}

impl Gpu {
    fn new(width: u32, height: u32) -> Self {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            force_fallback_adapter: false,
            compatible_surface: None,
            apply_limit_buckets: false,
        }))
        .expect("no GPU adapter");
        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
                .expect("no GPU device");
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: None,
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: None,
            layout: None,
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::TextureFormat::Rgba8Unorm.into())],
            }),
            multiview_mask: None,
            cache: None,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let make = |format, w, h, usage| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: None,
                size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage,
                view_formats: &[],
            })
        };
        let plane_usage = wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST;
        let luma = make(wgpu::TextureFormat::R8Unorm, width, height, plane_usage);
        let chroma = make(wgpu::TextureFormat::Rg8Unorm, width / 2, height / 2, plane_usage);
        let target = make(
            wgpu::TextureFormat::Rgba8Unorm,
            width,
            height,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        );
        let target_view = target.create_view(&Default::default());

        // SAFETY: the Metal backend is the only one enabled on this platform, and the out pointer
        // is valid for the duration of the call.
        let cache = unsafe {
            let hal = device.as_hal::<Metal>().expect("not the Metal backend");
            let mut raw: *mut CVMetalTextureCache = std::ptr::null_mut();
            let status = CVMetalTextureCache::create(
                None,
                None,
                hal.raw_device(),
                None,
                NonNull::from(&mut raw),
            );
            assert_eq!(status, kCVReturnSuccess);
            CFRetained::from_raw(NonNull::new(raw).expect("no texture cache"))
        };

        Self {
            device,
            queue,
            pipeline,
            sampler,
            target,
            target_view,
            width,
            height,
            luma,
            chroma,
            cache,
        }
    }

    /// Copies a frame's planes into textures this owns, the portable way to reach the GPU.
    fn upload(&self, frame: &Frame) {
        let buffer = &frame.buffer;
        let read_only = CVPixelBufferLockFlags::ReadOnly;
        // SAFETY: the planes stay locked until after both copies, and the sizes come from the
        // buffer's own row stride and the texture size this decoder's output is checked against.
        unsafe {
            assert_eq!(CVPixelBufferLockBaseAddress(buffer, read_only), kCVReturnSuccess);
            for (plane, texture, w, h) in [
                (0, &self.luma, self.width, self.height),
                (1, &self.chroma, self.width / 2, self.height / 2),
            ] {
                let stride = CVPixelBufferGetBytesPerRowOfPlane(buffer, plane);
                let base = CVPixelBufferGetBaseAddressOfPlane(buffer, plane).cast::<u8>();
                let data = std::slice::from_raw_parts(base, stride * h as usize);
                self.queue.write_texture(
                    wgpu::TexelCopyTextureInfo {
                        texture,
                        mip_level: 0,
                        origin: wgpu::Origin3d::ZERO,
                        aspect: wgpu::TextureAspect::All,
                    },
                    data,
                    wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(stride as u32),
                        rows_per_image: Some(h),
                    },
                    wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                );
            }
            CVPixelBufferUnlockBaseAddress(buffer, read_only);
        }
    }

    /// Wraps a frame's IOSurface planes as textures without copying any pixels.
    fn wrap(&self, frame: &Frame) -> (wgpu::Texture, wgpu::Texture, Vec<CFRetained<CVMetalTexture>>) {
        let mut keep = Vec::with_capacity(2);
        let mut plane = |index: usize, metal, format, w: u32, h: u32| {
            // SAFETY: the cache and pixel buffer are live, and the plane index, format and size
            // match this buffer's 420v layout. `keep` holds the CVMetalTexture until the GPU is
            // finished with it, which keeps the underlying IOSurface from being reused.
            unsafe {
                let mut raw: *mut CVMetalTexture = std::ptr::null_mut();
                let status = CVMetalTextureCache::create_texture_from_image(
                    None,
                    &self.cache,
                    &frame.buffer,
                    None,
                    metal,
                    w as usize,
                    h as usize,
                    index,
                    NonNull::from(&mut raw),
                );
                assert_eq!(status, kCVReturnSuccess);
                let cv = CFRetained::from_raw(NonNull::new(raw).expect("no CVMetalTexture"));
                let mtl = CVMetalTextureGetTexture(&cv).expect("no MTLTexture");
                keep.push(cv);
                let hal = wgpu::hal::metal::Device::texture_from_raw(
                    mtl,
                    format,
                    MTLTextureType::Type2D,
                    1,
                    1,
                    wgpu::hal::CopyExtent { width: w, height: h, depth: 1 },
                    None,
                );
                self.device.create_texture_from_hal::<Metal>(
                    hal,
                    &wgpu::TextureDescriptor {
                        label: None,
                        size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                        mip_level_count: 1,
                        sample_count: 1,
                        dimension: wgpu::TextureDimension::D2,
                        format,
                        usage: wgpu::TextureUsages::TEXTURE_BINDING,
                        view_formats: &[],
                    },
                    wgpu::TextureUses::RESOURCE,
                )
            }
        };
        let (w, h) = (self.width, self.height);
        let luma = plane(0, MTLPixelFormat::R8Unorm, wgpu::TextureFormat::R8Unorm, w, h);
        let chroma = plane(1, MTLPixelFormat::RG8Unorm, wgpu::TextureFormat::Rg8Unorm, w / 2, h / 2);
        (luma, chroma, keep)
    }

    /// Converts NV12 to RGBA into the target, as the compositor will on every frame.
    fn draw(&self, luma: &wgpu::Texture, chroma: &wgpu::Texture) {
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &self.pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&luma.create_view(&Default::default())),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&chroma.create_view(&Default::default())),
                },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&self.sampler) },
            ],
        });
        let mut encoder = self.device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.target_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            pass.draw(0..3, 0..1);
        }
        self.queue.submit(Some(encoder.finish()));
    }

    fn wait(&self) {
        let _ = self.device.poll(wgpu::PollType::wait_indefinitely());
    }

    fn read_target(&self) -> Vec<u8> {
        let stride = self.width * 4;
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: u64::from(stride * self.height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = self.device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            self.target.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(stride),
                    rows_per_image: Some(self.height),
                },
            },
            wgpu::Extent3d { width: self.width, height: self.height, depth_or_array_layers: 1 },
        );
        self.queue.submit(Some(encoder.finish()));
        buffer.slice(..).map_async(wgpu::MapMode::Read, |result| result.expect("map failed"));
        self.wait();
        let data = buffer.slice(..).get_mapped_range().expect("mapped").to_vec();
        buffer.unmap();
        data
    }
}

// ---- Measurements ----

#[derive(Clone, Copy, PartialEq)]
enum Path {
    DecodeOnly,
    Upload,
    ZeroCopy,
}

fn cpu_seconds() -> f64 {
    let mut usage = std::mem::MaybeUninit::<Rusage>::uninit();
    // SAFETY: getrusage fills the struct when it returns 0.
    unsafe {
        if getrusage(0, usage.as_mut_ptr()) != 0 {
            return 0.0;
        }
        let usage = usage.assume_init();
        usage.user.seconds as f64
            + f64::from(usage.user.micros) / 1e6
            + usage.system.seconds as f64
            + f64::from(usage.system.micros) / 1e6
    }
}

#[repr(C)]
struct TimeValue {
    seconds: i64,
    micros: i32,
}

#[repr(C)]
struct Rusage {
    user: TimeValue,
    system: TimeValue,
    rest: [i64; 14],
}

unsafe extern "C" {
    fn getrusage(who: i32, usage: *mut Rusage) -> i32;
}

fn show(gpu: &Gpu, path: Path, frame: &Frame) {
    match path {
        Path::DecodeOnly => {}
        Path::Upload => {
            gpu.upload(frame);
            gpu.draw(&gpu.luma, &gpu.chroma);
        }
        Path::ZeroCopy => {
            let (luma, chroma, keep) = gpu.wrap(frame);
            gpu.draw(&luma, &chroma);
            gpu.wait();
            drop(keep);
        }
    }
}

/// As fast as it'll go, to see how far above real time each path is.
fn throughput(decoder: &mut dyn Decoder, gpu: &Gpu, path: Path) -> Value {
    let started = Instant::now();
    let cpu_started = cpu_seconds();
    let mut frames = 0_u32;
    while let Some(frame) = decoder.next() {
        show(gpu, path, &frame);
        if path == Path::Upload && frames % 4 == 0 {
            gpu.wait();
        }
        frames += 1;
    }
    gpu.wait();
    let seconds = started.elapsed().as_secs_f64();
    json!({
        "frames": frames,
        "fps": f64::from(frames) / seconds,
        "cpu_cores": (cpu_seconds() - cpu_started) / seconds,
    })
}

/// Plays in real time on a 60 Hz clock, with the decoder on its own thread as the app will have it.
fn realtime(mut make: impl FnMut() -> Box<dyn Decoder> + Send + 'static, gpu: &Gpu, path: Path, fps: f64, seconds: f64) -> Value {
    let (tx, rx) = sync_channel::<Frame>(6);
    std::thread::spawn(move || {
        let mut decoder = make();
        while let Some(frame) = decoder.next() {
            if tx.send(frame).is_err() {
                return;
            }
        }
    });
    let rx: Receiver<Frame> = rx;

    let first = rx.recv().expect("no first frame");
    let period = Duration::from_secs_f64(1.0 / fps);
    let start = Instant::now();
    let cpu_started = cpu_seconds();
    let mut queued = Some(first);
    let mut ticks = 0_u32;
    let (mut shown, mut dropped, mut repeated, mut late) = (0_u32, 0_u32, 0_u32, 0_u32);
    let mut work_ms = Vec::new();
    let mut ended = false;
    while start.elapsed().as_secs_f64() < seconds && !ended {
        let deadline = start + period * ticks;
        if let Some(wait) = deadline.checked_duration_since(Instant::now()) {
            std::thread::sleep(wait);
        }
        let tick_started = Instant::now();
        let due = f64::from(ticks) / fps;
        let mut newest = None;
        loop {
            if queued.is_none() {
                match rx.try_recv() {
                    Ok(frame) => queued = Some(frame),
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        ended = true;
                        break;
                    }
                }
            }
            match queued.take() {
                Some(frame) if frame.pts <= due + period.as_secs_f64() / 2.0 => {
                    if newest.replace(frame).is_some() {
                        dropped += 1;
                    }
                }
                other => {
                    queued = other;
                    break;
                }
            }
        }
        match newest {
            Some(frame) => {
                show(gpu, path, &frame);
                gpu.wait();
                shown += 1;
            }
            None => repeated += 1,
        }
        let work = tick_started.elapsed();
        work_ms.push(work.as_secs_f32() * 1000.0);
        if work > period {
            late += 1;
        }
        ticks += 1;
    }
    let elapsed = start.elapsed().as_secs_f64();
    work_ms.sort_by(f32::total_cmp);
    json!({
        "seconds": elapsed,
        "shown": shown,
        "fps": f64::from(shown) / elapsed,
        "dropped": dropped,
        "repeated_ticks": repeated,
        "late_ticks": late,
        "work_ms_p50": work_ms[work_ms.len() / 2],
        "work_ms_p99": work_ms[(work_ms.len() - 1) * 99 / 100],
        "work_ms_max": work_ms[work_ms.len() - 1],
        "cpu_cores": (cpu_seconds() - cpu_started) / elapsed,
    })
}

/// Jumps to spread-out positions and times how long until that exact frame is ready.
fn seeks(decoder: &mut dyn Decoder, duration: f64, fps: f64) -> Value {
    let mut state = 12345_u64;
    let mut times = Vec::new();
    let mut wrong = 0;
    for _ in 0..40 {
        state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        let index = (state >> 33) % ((duration * fps) as u64 - 2);
        let target = index as f64 / fps;
        let started = Instant::now();
        match decoder.seek(target) {
            Some(frame) => {
                times.push(started.elapsed().as_secs_f32() * 1000.0);
                if (frame.pts - target).abs() > 0.5 / fps {
                    wrong += 1;
                }
            }
            None => wrong += 1,
        }
    }
    times.sort_by(f32::total_cmp);
    json!({
        "seeks": times.len(),
        "wrong_frame": wrong,
        "ms_p50": times[times.len() / 2],
        "ms_p95": times[(times.len() - 1) * 95 / 100],
        "ms_max": times[times.len() - 1],
    })
}

/// Compares what reached the GPU with a reference frame decoded by the ffmpeg CLI at the same time.
fn accuracy(decoder: &mut dyn Decoder, gpu: &Gpu, path: Path, video: &str, seconds: f64) -> Value {
    let frame = decoder.seek(seconds).expect("no frame at the check position");
    show(gpu, path, &frame);
    gpu.wait();
    let pixels = gpu.read_target();
    let (w, h) = (gpu.width as usize, gpu.height as usize);
    // The neighbouring frames show whether this really is the requested frame: only the right one
    // should score best, however well the colour conversion matches.
    let score = |at: f64, luma_only: bool| {
        let reference = std::process::Command::new("ffmpeg")
            .args(["-hide_banner", "-loglevel", "error", "-ss", &format!("{at:.4}"), "-i", video])
            .args(["-frames:v", "1", "-vf", "scale=in_color_matrix=bt709:in_range=tv,format=rgb24"])
            .args(["-f", "rawvideo", "-"])
            .output()
            .expect("couldn't run ffmpeg for the reference frame")
            .stdout;
        assert_eq!(reference.len(), w * h * 3, "unexpected reference frame size");
        let gray = |r: u8, g: u8, b: u8| 0.2126 * f64::from(r) + 0.7152 * f64::from(g) + 0.0722 * f64::from(b);
        let mut squared = 0.0_f64;
        for i in 0..w * h {
            let (p, q) = (&pixels[i * 4..i * 4 + 3], &reference[i * 3..i * 3 + 3]);
            if luma_only {
                let d = gray(p[0], p[1], p[2]) - gray(q[0], q[1], q[2]);
                squared += d * d;
            } else {
                for c in 0..3 {
                    let d = f64::from(p[c]) - f64::from(q[c]);
                    squared += d * d / 3.0;
                }
            }
        }
        let mse = squared / (w * h) as f64;
        if mse == 0.0 { 99.0 } else { 10.0 * (255.0 * 255.0 / mse).log10() }
    };
    let step = 1.0 / 60.0;
    json!({
        "at_seconds": seconds,
        "pts": frame.pts,
        "psnr_db": score(seconds, false),
        "psnr_db_luma": score(seconds, true),
        "psnr_db_previous_frame": score(seconds - step, false),
        "psnr_db_next_frame": score(seconds + step, false),
    })
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let video = args.get(1).expect("usage: t2-decode <video> [report.json]").clone();

    let mut probe = FfmpegDecoder::open(&video, false);
    let (width, height) = (probe.decoder.width(), probe.decoder.height());
    let rate = probe.input.stream(probe.stream).expect("stream").avg_frame_rate();
    let fps = f64::from(rate);
    let duration = probe.input.duration() as f64 / f64::from(ffmpeg::ffi::AV_TIME_BASE);
    println!("{video}: {width}x{height} at {fps} fps, {duration:.1} s");

    let gpu = Gpu::new(width, height);

    // Software decode, the fallback on any OS, measured for decoding alone.
    let software = {
        let started = Instant::now();
        let cpu_started = cpu_seconds();
        let mut frames = 0_u32;
        while probe.advance().is_some() {
            frames += 1;
        }
        let seconds = started.elapsed().as_secs_f64();
        let fps_measured = f64::from(frames) / seconds;
        println!("software decode: {fps_measured:.0} fps");
        json!({ "frames": frames, "fps": fps_measured, "cpu_cores": (cpu_seconds() - cpu_started) / seconds })
    };
    let software_seek = {
        let mut state = 12345_u64;
        let mut times = Vec::new();
        for _ in 0..20 {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            let target = ((state >> 33) % ((duration * fps) as u64 - 2)) as f64 / fps;
            let started = Instant::now();
            probe.seek_to(target);
            times.push(started.elapsed().as_secs_f32() * 1000.0);
        }
        times.sort_by(f32::total_cmp);
        json!({ "ms_p50": times[times.len() / 2], "ms_max": times[times.len() - 1] })
    };

    let mut backends = serde_json::Map::new();
    let video_for = video.clone();
    let open: [(&str, Box<dyn Fn() -> Box<dyn Decoder> + Send + Sync>); 2] = [
        ("avfoundation", {
            let v = video_for.clone();
            Box::new(move || Box::new(NativeDecoder::open(&v)) as Box<dyn Decoder>)
        }),
        ("ffmpeg_videotoolbox", {
            let v = video_for.clone();
            Box::new(move || Box::new(FfmpegDecoder::open(&v, true)) as Box<dyn Decoder>)
        }),
    ];
    let open: Vec<(&str, std::sync::Arc<dyn Fn() -> Box<dyn Decoder> + Send + Sync>)> = open
        .into_iter()
        .map(|(name, make)| (name, std::sync::Arc::from(make)))
        .collect();

    for (name, make) in &open {
        let mut result = serde_json::Map::new();
        for (label, path) in [("decode_only", Path::DecodeOnly), ("upload", Path::Upload), ("zero_copy", Path::ZeroCopy)] {
            let measured = throughput(make().as_mut(), &gpu, path);
            println!("{name} {label}: {:.0} fps, {:.2} CPU cores", measured["fps"].as_f64().unwrap_or(0.0), measured["cpu_cores"].as_f64().unwrap_or(0.0));
            result.insert(format!("throughput_{label}"), measured);
        }
        for (label, path) in [("upload", Path::Upload), ("zero_copy", Path::ZeroCopy)] {
            let make = make.clone();
            let measured = realtime(move || make(), &gpu, path, fps, duration.min(10.0));
            println!("{name} realtime {label}: {measured}");
            result.insert(format!("realtime_{label}"), measured);
        }
        let seek = seeks(make().as_mut(), duration, fps);
        println!("{name} seeks: {seek}");
        result.insert("seeks".to_string(), seek);
        for (label, path) in [("upload", Path::Upload), ("zero_copy", Path::ZeroCopy)] {
            let measured = accuracy(make().as_mut(), &gpu, path, &video, 5.0);
            println!("{name} accuracy {label}: {measured}");
            result.insert(format!("accuracy_{label}"), measured);
        }
        backends.insert((*name).to_string(), Value::Object(result));
    }

    let report = json!({
        "video": video,
        "width": width,
        "height": height,
        "fps": fps,
        "duration": duration,
        "software_decode": software,
        "software_seek": software_seek,
        "backends": backends,
    });
    if let Some(path) = args.get(2) {
        std::fs::write(path, serde_json::to_vec_pretty(&report).unwrap()).expect("couldn't write the report");
    }
}
