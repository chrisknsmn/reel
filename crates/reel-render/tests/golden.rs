//! Golden frames: renders known scenes with no window, and compares them with stored images.
//!
//! Run with `REEL_UPDATE_GOLDEN=1` to write new images after a deliberate change, and look at them
//! before committing. On a machine with no GPU, these print a note and pass.

use std::path::{Path, PathBuf};

use reel_core::project::{Background, Crop, Look, Padding};
use reel_core::timeline::Point;
use reel_motion::camera::{Camera, target_camera};
use reel_motion::layout::{self, Rect};
use reel_render::{
    FrameInput, Headless, Renderer, VideoSource, WORKING_FORMAT, read_pixels, readable_target,
    texture_from_rgba,
};

const STAGE: (u32, u32) = (640, 360);
const VIDEO: (u32, u32) = (320, 180);
/// The most any channel of any pixel may differ, to allow for differences between GPUs.
const TOLERANCE: u8 = 3;

/// A test card: a colour ramp with a grid, and a marked corner so flips and offsets show.
fn test_card() -> Vec<u8> {
    let mut pixels = Vec::with_capacity((VIDEO.0 * VIDEO.1 * 4) as usize);
    for y in 0..VIDEO.1 {
        for x in 0..VIDEO.0 {
            let on_grid = x % 40 == 0 || y % 40 == 0;
            let in_corner = x < 40 && y < 40;
            let (r, g, b) = if in_corner {
                (255, 40, 40)
            } else if on_grid {
                (255, 255, 255)
            } else {
                ((x * 255 / VIDEO.0) as u8, (y * 255 / VIDEO.1) as u8, 160)
            };
            pixels.extend_from_slice(&[r, g, b, 255]);
        }
    }
    pixels
}

fn golden_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(format!("{name}.png"))
}

fn write_png(path: &Path, size: (u32, u32), pixels: &[u8]) {
    let file = std::io::BufWriter::new(std::fs::File::create(path).unwrap());
    let mut encoder = png::Encoder::new(file, size.0, size.1);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .unwrap()
        .write_image_data(pixels)
        .unwrap();
}

fn read_png(path: &Path) -> (u32, u32, Vec<u8>) {
    let decoder = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(path).unwrap()));
    let mut reader = decoder.read_info().unwrap();
    let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut pixels).unwrap();
    pixels.truncate(info.buffer_size());
    (info.width, info.height, pixels)
}

struct Scene {
    gpu: Option<Headless>,
}

impl Scene {
    fn new() -> Self {
        let gpu = Headless::new();
        if gpu.is_none() {
            println!("No GPU on this machine, so this golden test is skipped.");
        }
        Self { gpu }
    }

    fn render(&self, look: &Look, camera: Camera) -> Option<Vec<u8>> {
        let gpu = self.gpu.as_ref()?;
        let mut renderer = Renderer::new(gpu.device.clone(), gpu.queue.clone(), WORKING_FORMAT);
        let video = texture_from_rgba(&gpu.device, &gpu.queue, VIDEO, &test_card());
        let view = video.create_view(&wgpu::TextureViewDescriptor::default());
        let target = readable_target(&gpu.device, STAGE, WORKING_FORMAT);
        renderer.render(
            &target,
            &FrameInput {
                look,
                video: VideoSource::Rgba(&view),
                video_size: VIDEO,
                camera,
                layers: None,
            },
        );
        assert!(renderer.warnings.is_empty(), "{:?}", renderer.warnings);
        Some(read_pixels(&gpu.device, &gpu.queue, &target))
    }

    fn check(&self, name: &str, look: &Look, camera: Camera) {
        let Some(pixels) = self.render(look, camera) else {
            return;
        };
        let path = golden_path(name);
        if std::env::var_os("REEL_UPDATE_GOLDEN").is_some() {
            write_png(&path, STAGE, &pixels);
            return;
        }
        let (width, height, stored) = read_png(&path);
        assert_eq!((width, height), STAGE, "{name} is the wrong size");
        let mut worst = 0;
        let mut different = 0;
        for (a, b) in pixels.iter().zip(&stored) {
            let difference = a.abs_diff(*b);
            worst = worst.max(difference);
            different += usize::from(difference > TOLERANCE);
        }
        if different > 0 {
            let actual = std::env::temp_dir().join(format!("{name}.actual.png"));
            write_png(&actual, STAGE, &pixels);
            panic!(
                "{name}: {different} channels differ by more than {TOLERANCE}, at worst by {worst}. See {}",
                actual.display()
            );
        }
    }
}

fn plain_look() -> Look {
    Look {
        background: Background::Color("#20303F".to_string()),
        background_blur: 0.0,
        shadow: 0.0,
        corner_radius: 0.0,
        padding: Padding::uniform(0.0),
        crop: Crop::default(),
        aspect_ratio: "16:9".to_string(),
    }
}

#[test]
fn the_video_fills_the_stage_with_no_padding() {
    Scene::new().check("plain", &plain_look(), Camera::IDENTITY);
}

#[test]
fn padding_corners_and_shadow() {
    let look = Look {
        shadow: 0.67,
        corner_radius: 12.5 * 4.0,
        padding: Padding::uniform(20.0),
        ..plain_look()
    };
    Scene::new().check("framed", &look, Camera::IDENTITY);
}

#[test]
fn a_crop_shows_part_of_the_video() {
    let look = Look {
        crop: Crop {
            x: 0.25,
            y: 0.25,
            width: 0.5,
            height: 0.5,
        },
        padding: Padding::uniform(20.0),
        ..plain_look()
    };
    Scene::new().check("cropped", &look, Camera::IDENTITY);
}

#[test]
fn a_zoom_goes_through_the_camera() {
    let look = Look {
        shadow: 0.67,
        padding: Padding::uniform(20.0),
        corner_radius: 50.0,
        ..plain_look()
    };
    let stage = (f64::from(STAGE.0), f64::from(STAGE.1));
    let frame = layout::compute(
        stage,
        &look.padding,
        &look.crop,
        (f64::from(VIDEO.0), f64::from(VIDEO.1)),
    )
    .frame;
    let mask = Rect {
        x: frame.x / stage.0,
        y: frame.y / stage.1,
        width: frame.width / stage.0,
        height: frame.height / stage.1,
    };
    let camera = target_camera(&mask, 2.2, 1.0, Point { x: 0.3, y: 0.7 });
    Scene::new().check("zoomed", &look, camera);
}

#[test]
fn a_linear_gradient_keeps_its_angle() {
    let look = Look {
        background: Background::Gradient("linear-gradient(90deg, #ff0000, #0000ff)".to_string()),
        padding: Padding::uniform(50.0),
        ..plain_look()
    };
    Scene::new().check("gradient-90deg", &look, Camera::IDENTITY);
}

#[test]
fn a_radial_gradient() {
    let look = Look {
        background: Background::Gradient("radial-gradient(circle, #ffffff, #000000)".to_string()),
        padding: Padding::uniform(50.0),
        ..plain_look()
    };
    Scene::new().check("gradient-radial", &look, Camera::IDENTITY);
}

#[test]
fn a_blurred_background_is_the_same_at_any_size() {
    // A hard-edged gradient, so the blur has something to soften.
    let look = Look {
        background: Background::Gradient(
            "linear-gradient(90deg, #ff0000 50%, #0000ff 50%)".to_string(),
        ),
        background_blur: 4.0,
        padding: Padding::uniform(50.0),
        ..plain_look()
    };
    Scene::new().check("blurred", &look, Camera::IDENTITY);
}

#[test]
fn nv12_video_converts_to_the_right_colours() {
    let scene = Scene::new();
    let Some(gpu) = &scene.gpu else { return };
    // Limited-range BT.709 for white, then for a mid red.
    let mut renderer = Renderer::new(gpu.device.clone(), gpu.queue.clone(), WORKING_FORMAT);
    let plane = |format, value: &[u8], bytes: u32| {
        let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        gpu.queue.write_texture(
            texture.as_image_copy(),
            value,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(bytes),
                rows_per_image: Some(1),
            },
            texture.size(),
        );
        texture.create_view(&wgpu::TextureViewDescriptor::default())
    };
    let look = plain_look();
    let mut read = |y: u8, u: u8, v: u8| {
        let (luma, chroma) = (
            plane(wgpu::TextureFormat::R8Unorm, &[y], 1),
            plane(wgpu::TextureFormat::Rg8Unorm, &[u, v], 2),
        );
        let target = readable_target(&gpu.device, (4, 4), WORKING_FORMAT);
        renderer.render(
            &target,
            &FrameInput {
                look: &look,
                video: VideoSource::Nv12 {
                    luma: &luma,
                    chroma: &chroma,
                },
                video_size: (4, 4),
                camera: Camera::IDENTITY,
                layers: None,
            },
        );
        let pixels = read_pixels(&gpu.device, &gpu.queue, &target);
        [pixels[0], pixels[1], pixels[2]]
    };
    let close = |a: [u8; 3], b: [u8; 3]| a.iter().zip(&b).all(|(x, y)| x.abs_diff(*y) <= 2);
    assert!(
        close(read(235, 128, 128), [255, 255, 255]),
        "limited-range white"
    );
    assert!(close(read(16, 128, 128), [0, 0, 0]), "limited-range black");
    let red = read(63, 102, 240);
    assert!(close(red, [255, 0, 0]), "BT.709 red, got {red:?}");
}

#[test]
fn the_shadow_falls_below_the_frame_and_fades_out() {
    let look = Look {
        background: Background::Color("#FFFFFF".to_string()),
        shadow: 1.0,
        padding: Padding::uniform(50.0),
        ..plain_look()
    };
    let scene = Scene::new();
    let Some(pixels) = scene.render(&look, Camera::IDENTITY) else {
        return;
    };
    let brightness = |x: u32, y: u32| pixels[((y * STAGE.0 + x) * 4) as usize];
    // The frame fills 10% to 90% of the stage at this padding.
    let middle = STAGE.0 / 2;
    let (top, bottom) = (STAGE.1 / 10, STAGE.1 * 9 / 10);
    let (above, below) = (brightness(middle, top - 2), brightness(middle, bottom + 2));
    assert!(
        below < 200,
        "there should be a shadow under the frame: {below}"
    );
    assert!(
        below < above,
        "the shadow is offset downward: above {above}, below {below}"
    );
    assert!(
        brightness(middle, bottom + 30) > below,
        "it fades with distance"
    );
    assert_eq!(
        brightness(2, 2),
        255,
        "the far corner of the stage is untouched"
    );
}

#[test]
fn no_shadow_setting_means_no_shadow() {
    let look = Look {
        background: Background::Color("#FFFFFF".to_string()),
        shadow: 0.0,
        padding: Padding::uniform(50.0),
        ..plain_look()
    };
    let scene = Scene::new();
    let Some(pixels) = scene.render(&look, Camera::IDENTITY) else {
        return;
    };
    let at = ((STAGE.1 * 9 / 10 + 2) * STAGE.0 + STAGE.0 / 2) as usize * 4;
    assert_eq!(pixels[at], 255);
}

#[test]
fn rendering_twice_gives_the_same_pixels() {
    let look = Look {
        shadow: 0.67,
        corner_radius: 50.0,
        padding: Padding::uniform(20.0),
        ..plain_look()
    };
    let scene = Scene::new();
    assert_eq!(
        scene.render(&look, Camera::IDENTITY),
        scene.render(&look, Camera::IDENTITY)
    );
}
