//! Everything drawn over the video, measured from the pixels that come out. Text is checked by where
//! it lands rather than against an image, because fonts differ between machines.
//!
//! On a machine with no GPU, these print a note and pass.

use reel_core::project::{
    CaptionAnimation, ClickEffect, Look, Padding, Project, Webcam, WebcamPosition,
};
use reel_core::time::{Micros, SourceTime, TimelineTime};
use reel_core::timeline::{
    Annotation, AnnotationContent, ArrowDirection, Caption, Point, TextStyle, Timeline,
};
use reel_motion::camera::Camera;
use reel_motion::cursor::{CursorSample, CursorShape, CursorTrack, Interaction};
use reel_motion::layout::{self, Rect};
use reel_render::layers::SceneLayers;
use reel_render::{
    FrameInput, Headless, Renderer, VideoSource, WORKING_FORMAT, read_pixels, readable_target,
    texture_from_rgba,
};

const STAGE: (u32, u32) = (1280, 720);
const VIDEO: (u32, u32) = (64, 36);
const GREY: u8 = 90;

fn plain_look() -> Look {
    Look {
        shadow: 0.0,
        corner_radius: 0.0,
        padding: Padding::uniform(0.0),
        ..Look::default()
    }
}

fn project() -> Project {
    let mut project = Project {
        timeline: Timeline::for_recording(Micros::from_secs(10)),
        look: plain_look(),
        ..Project::default()
    };
    project.cursor.motion_blur = 0.0;
    project.zoom_motion.motion_blur = 0.0;
    project
}

fn full_mask() -> Rect {
    Rect {
        x: 0.0,
        y: 0.0,
        width: 1.0,
        height: 1.0,
    }
}

fn at(ms: f64) -> TimelineTime {
    TimelineTime::from_ms(ms)
}

fn solid(size: (u32, u32), color: [u8; 4]) -> Vec<u8> {
    color
        .iter()
        .copied()
        .cycle()
        .take((size.0 * size.1 * 4) as usize)
        .collect()
}

/// One frame's pixels. Keeps its renderer, so that caching across frames gets tested too.
struct Bench {
    gpu: Headless,
    renderer: Renderer,
    format: wgpu::TextureFormat,
}

impl Bench {
    fn new() -> Option<Self> {
        Self::with_format(WORKING_FORMAT)
    }

    fn with_format(format: wgpu::TextureFormat) -> Option<Self> {
        let Some(gpu) = Headless::new() else {
            println!("No GPU on this machine, so this test is skipped.");
            return None;
        };
        let renderer = Renderer::new(gpu.device.clone(), gpu.queue.clone(), format);
        Some(Self {
            gpu,
            renderer,
            format,
        })
    }

    fn draw(
        &mut self,
        project: &Project,
        layers: Option<&SceneLayers>,
        time: TimelineTime,
        camera: Camera,
        video: &[u8],
        webcam: Option<(&[u8], (u32, u32))>,
    ) -> Picture {
        let texture = texture_from_rgba(&self.gpu.device, &self.gpu.queue, VIDEO, video);
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let webcam_texture = webcam.map(|(pixels, size)| {
            let texture = texture_from_rgba(&self.gpu.device, &self.gpu.queue, size, pixels);
            (
                texture.create_view(&wgpu::TextureViewDescriptor::default()),
                size,
            )
        });
        let webcam_source = webcam_texture
            .as_ref()
            .map(|(view, size)| (VideoSource::Rgba(view), *size));
        let target = readable_target(&self.gpu.device, STAGE, self.format);
        self.renderer.render(
            &target,
            &FrameInput {
                look: &project.look,
                video: VideoSource::Rgba(&view),
                video_size: VIDEO,
                camera,
                layers: layers.map(|layers| layers.at(time, webcam_source)),
            },
        );
        Picture {
            pixels: read_pixels(&self.gpu.device, &self.gpu.queue, &target),
        }
    }

    /// A frame with a flat grey video and a still camera.
    fn plain(&mut self, project: &Project, layers: &SceneLayers, time: TimelineTime) -> Picture {
        let video = solid(VIDEO, [GREY, GREY, GREY, 255]);
        self.draw(project, Some(layers), time, Camera::IDENTITY, &video, None)
    }
}

struct Picture {
    pixels: Vec<u8>,
}

impl Picture {
    fn at(&self, x: u32, y: u32) -> [u8; 4] {
        let index = ((y * STAGE.0 + x) * 4) as usize;
        self.pixels[index..index + 4].try_into().unwrap()
    }

    fn is_grey(&self, x: u32, y: u32) -> bool {
        let [r, g, b, _] = self.at(x, y);
        [r, g, b].iter().all(|channel| channel.abs_diff(GREY) <= 2)
    }

    /// The box around everything that isn't the flat grey video.
    fn drawn_bounds(&self) -> Option<(u32, u32, u32, u32)> {
        let mut bounds: Option<(u32, u32, u32, u32)> = None;
        for y in 0..STAGE.1 {
            for x in 0..STAGE.0 {
                if !self.is_grey(x, y) {
                    bounds = Some(match bounds {
                        None => (x, y, x, y),
                        Some((left, top, right, bottom)) => {
                            (left.min(x), top.min(y), right.max(x), bottom.max(y))
                        }
                    });
                }
            }
        }
        bounds
    }

    /// How much neighbouring pixels differ across an area, which blur brings down.
    fn roughness(&self, area: (u32, u32, u32, u32)) -> f64 {
        let mut total = 0.0;
        let mut count = 0.0;
        for y in area.1..area.3 {
            for x in area.0..area.2 {
                total += f64::from(self.at(x, y)[0].abs_diff(self.at(x + 1, y)[0]));
                count += 1.0;
            }
        }
        total / count
    }

    fn mean(&self, area: (u32, u32, u32, u32)) -> [f64; 3] {
        let mut total = [0.0; 3];
        let mut count = 0.0;
        for y in area.1..area.3 {
            for x in area.0..area.2 {
                let pixel = self.at(x, y);
                for channel in 0..3 {
                    total[channel] += f64::from(pixel[channel]);
                }
                count += 1.0;
            }
        }
        total.map(|sum| sum / count)
    }
}

fn sample(ms: f64, x: f64, y: f64, interaction: Interaction) -> CursorSample {
    CursorSample {
        time: SourceTime::from_ms(ms),
        x,
        y,
        interaction,
        shape: CursorShape::Arrow,
    }
}

fn still_cursor(x: f64, y: f64) -> CursorTrack {
    CursorTrack {
        samples: vec![
            sample(0.0, x, y, Interaction::Move),
            sample(10_000.0, x, y, Interaction::Move),
        ],
    }
}

fn annotation(content: AnnotationContent, area: (f64, f64, f64, f64), layer: i32) -> Annotation {
    Annotation {
        id: format!("a{layer}").as_str().into(),
        start: at(0.0),
        end: at(5000.0),
        content,
        position: Point {
            x: area.0,
            y: area.1,
        },
        width: area.2,
        height: area.3,
        track: 0,
        layer,
    }
}

fn arrow(direction: ArrowDirection, color: &str) -> AnnotationContent {
    AnnotationContent::Arrow {
        direction,
        color: color.to_string(),
        stroke_width: 6.0,
    }
}

/// A small PNG as a data URL.
fn png_data_url(size: (u32, u32), color: [u8; 4]) -> String {
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, size.0, size.1);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&solid(size, color))
            .unwrap();
    }
    const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut text = String::from("data:image/png;base64,");
    for chunk in bytes.chunks(3) {
        let value = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        for index in 0..4 {
            if index <= chunk.len() {
                text.push(TABLE[((value >> (18 - index * 6)) & 63) as usize] as char);
            } else {
                text.push('=');
            }
        }
    }
    text
}

#[test]
fn with_no_overlays_the_video_is_untouched() {
    let Some(mut bench) = Bench::new() else {
        return;
    };
    let project = project();
    let layers = SceneLayers::build(&project, &full_mask(), None);
    assert_eq!(
        bench.plain(&project, &layers, at(100.0)).drawn_bounds(),
        None
    );
}

#[test]
fn the_cursors_tip_lands_where_the_recording_says() {
    let Some(mut bench) = Bench::new() else {
        return;
    };
    let project = project();
    let layers = SceneLayers::build(&project, &full_mask(), Some(&still_cursor(0.5, 0.5)));
    let picture = bench.plain(&project, &layers, at(100.0));
    let (left, top, right, bottom) = picture.drawn_bounds().expect("a cursor");
    // The arrow's tip is its top-left, and it should be right at the middle of the stage. The sprite
    // has a soft shadow around it, so allow a few pixels.
    assert!(
        (f64::from(left) - 640.0).abs() < 8.0,
        "the left edge is at {left}"
    );
    assert!(
        (f64::from(top) - 360.0).abs() < 8.0,
        "the top edge is at {top}"
    );
    // At the default size, on a 1280 stage: 28 × 2.5 × (1280/1920) ≈ 47 pixels tall.
    let height = f64::from(bottom - top);
    assert!((30.0..70.0).contains(&height), "it's {height} tall");
    assert!(right > left);
}

#[test]
fn turning_the_cursor_off_hides_it() {
    let Some(mut bench) = Bench::new() else {
        return;
    };
    let mut project = project();
    project.cursor.show = false;
    let layers = SceneLayers::build(&project, &full_mask(), Some(&still_cursor(0.5, 0.5)));
    assert_eq!(
        bench.plain(&project, &layers, at(100.0)).drawn_bounds(),
        None
    );
}

#[test]
fn the_cursor_moves_with_the_camera() {
    let Some(mut bench) = Bench::new() else {
        return;
    };
    let project = project();
    let layers = SceneLayers::build(&project, &full_mask(), Some(&still_cursor(0.25, 0.25)));
    let video = solid(VIDEO, [GREY, GREY, GREY, 255]);
    // Zoomed 2× on the top-left corner, a point a quarter of the way in lands in the middle.
    let camera = Camera {
        scale: 2.0,
        x: 0.0,
        y: 0.0,
    };
    let picture = bench.draw(&project, Some(&layers), at(100.0), camera, &video, None);
    let (left, top, _, bottom) = picture.drawn_bounds().expect("a cursor");
    assert!(
        (f64::from(left) - 640.0).abs() < 12.0,
        "the left edge is at {left}"
    );
    assert!(
        (f64::from(top) - 360.0).abs() < 12.0,
        "the top edge is at {top}"
    );
    assert!(
        bottom - top > 70,
        "and it's scaled up too, to {}",
        bottom - top
    );
}

#[test]
fn every_cursor_style_draws() {
    let Some(mut bench) = Bench::new() else {
        return;
    };
    for style in ["tahoe", "tahoe-inverted", "mono", "dot", "minimal", "figma"] {
        let mut project = project();
        project.cursor.style = style.to_string();
        let layers = SceneLayers::build(&project, &full_mask(), Some(&still_cursor(0.5, 0.5)));
        assert!(
            bench
                .plain(&project, &layers, at(100.0))
                .drawn_bounds()
                .is_some(),
            "{style} drew nothing"
        );
    }
}

#[test]
fn a_fast_cursor_smears_and_a_still_one_stays_sharp() {
    let Some(mut bench) = Bench::new() else {
        return;
    };
    let track = CursorTrack {
        samples: vec![
            sample(0.0, 0.1, 0.5, Interaction::Move),
            sample(200.0, 0.9, 0.5, Interaction::Move),
        ],
    };
    let mut sharp_project = project();
    sharp_project.cursor.smoothing = 0.0;
    let mut blurred_project = sharp_project.clone();
    blurred_project.cursor.motion_blur = 1.0;

    let sharp = SceneLayers::build(&sharp_project, &full_mask(), Some(&track));
    let blurred = SceneLayers::build(&blurred_project, &full_mask(), Some(&track));
    let width = |picture: Picture| {
        let (left, _, right, _) = picture.drawn_bounds().expect("a cursor");
        right - left
    };
    let sharp_width = width(bench.plain(&sharp_project, &sharp, at(100.0)));
    let blurred_width = width(bench.plain(&blurred_project, &blurred, at(100.0)));
    assert!(
        blurred_width > sharp_width + 20,
        "{blurred_width} isn't much wider than {sharp_width}"
    );

    // Standing still, the same setting makes no difference.
    let still = SceneLayers::build(
        &blurred_project,
        &full_mask(),
        Some(&still_cursor(0.5, 0.5)),
    );
    let still_plain =
        SceneLayers::build(&sharp_project, &full_mask(), Some(&still_cursor(0.5, 0.5)));
    assert_eq!(
        bench.plain(&blurred_project, &still, at(100.0)).pixels,
        bench.plain(&sharp_project, &still_plain, at(100.0)).pixels,
    );
}

#[test]
fn a_click_ripples_then_fades_away() {
    let Some(mut bench) = Bench::new() else {
        return;
    };
    let mut project = project();
    project.cursor.click_effect = ClickEffect::Ripple;
    project.cursor.click_effect_color = "#FF0000".to_string();
    project.cursor.click_effect_ms = 400.0;
    project.cursor.style = "dot".to_string();
    let track = CursorTrack {
        samples: vec![
            sample(0.0, 0.5, 0.5, Interaction::Move),
            sample(1000.0, 0.5, 0.5, Interaction::Click),
            sample(5000.0, 0.5, 0.5, Interaction::Move),
        ],
    };
    let layers = SceneLayers::build(&project, &full_mask(), Some(&track));
    let reddish = |picture: &Picture| {
        (0..STAGE.1)
            .flat_map(|y| (0..STAGE.0).map(move |x| (x, y)))
            .filter(|(x, y)| {
                let [r, g, _, _] = picture.at(*x, *y);
                i32::from(r) - i32::from(g) > 25
            })
            .count()
    };
    assert_eq!(
        reddish(&bench.plain(&project, &layers, at(900.0))),
        0,
        "nothing before the click"
    );
    // Effects start half a bounce after the click, which is 175 ms at the default.
    assert_eq!(
        reddish(&bench.plain(&project, &layers, at(1100.0))),
        0,
        "nor right as it lands"
    );
    let during = reddish(&bench.plain(&project, &layers, at(1200.0)));
    assert!(during > 50, "only {during} red pixels during the ripple");
    assert_eq!(
        reddish(&bench.plain(&project, &layers, at(2000.0))),
        0,
        "gone after it"
    );
}

#[test]
fn an_arrow_points_the_right_way_in_the_right_place() {
    let Some(mut bench) = Bench::new() else {
        return;
    };
    let mut project = project();
    project.timeline.annotations = vec![annotation(
        arrow(ArrowDirection::Right, "#00FF00"),
        (25.0, 25.0, 50.0, 50.0),
        0,
    )];
    let layers = SceneLayers::build(&project, &full_mask(), None);
    let picture = bench.plain(&project, &layers, at(100.0));
    // The box is 640×360 at (320,180). Paths are in a centred 360-pixel square starting at x = 460,
    // so the shaft runs from 20 to 80 across it: x from 532 to 748, at y = 360.
    assert!(picture.at(640, 360)[1] > 200, "the shaft is at the middle");
    assert!(picture.at(545, 360)[1] > 200);
    assert!(picture.at(735, 360)[1] > 200);
    assert!(picture.is_grey(640, 250), "nothing above it");
    assert!(picture.is_grey(500, 360), "nothing beyond its tail");
    assert!(picture.is_grey(790, 360), "nothing beyond its tip");
    // The head flares back from the tip, to (65,35) and (65,65) in the square.
    assert!(picture.at(721, 333)[1] > 200, "the head's upper barb");
    assert!(picture.at(721, 387)[1] > 200, "the head's lower barb");
    assert!(picture.is_grey(559, 333), "and no barbs at the tail");
}

#[test]
fn every_arrow_direction_draws_inside_its_box() {
    let Some(mut bench) = Bench::new() else {
        return;
    };
    use ArrowDirection::{Down, DownLeft, DownRight, Left, Right, Up, UpLeft, UpRight};
    for direction in [Up, Down, Left, Right, UpLeft, UpRight, DownLeft, DownRight] {
        let mut project = project();
        project.timeline.annotations = vec![annotation(
            arrow(direction, "#00FF00"),
            (25.0, 25.0, 50.0, 50.0),
            0,
        )];
        let layers = SceneLayers::build(&project, &full_mask(), None);
        let (left, top, right, bottom) = bench
            .plain(&project, &layers, at(100.0))
            .drawn_bounds()
            .expect("an arrow");
        assert!(
            left >= 320 && top >= 180 && right <= 960 && bottom <= 540,
            "{direction:?} left its box"
        );
    }
}

#[test]
fn annotations_only_show_during_their_time() {
    let Some(mut bench) = Bench::new() else {
        return;
    };
    let mut project = project();
    let mut item = annotation(
        arrow(ArrowDirection::Up, "#00FF00"),
        (25.0, 25.0, 50.0, 50.0),
        0,
    );
    item.start = at(1000.0);
    item.end = at(2000.0);
    project.timeline.annotations = vec![item];
    let layers = SceneLayers::build(&project, &full_mask(), None);
    assert!(
        bench
            .plain(&project, &layers, at(500.0))
            .drawn_bounds()
            .is_none()
    );
    assert!(
        bench
            .plain(&project, &layers, at(1500.0))
            .drawn_bounds()
            .is_some()
    );
    assert!(
        bench
            .plain(&project, &layers, at(2500.0))
            .drawn_bounds()
            .is_none()
    );
}

#[test]
fn annotations_sit_inside_the_frame_and_follow_the_camera() {
    let Some(mut bench) = Bench::new() else {
        return;
    };
    let mut project = project();
    let image = AnnotationContent::Image {
        data_url: png_data_url((16, 9), [255, 0, 0, 255]),
    };
    project.timeline.annotations = vec![annotation(image, (50.0, 50.0, 25.0, 25.0), 0)];
    project.look.padding = Padding::uniform(0.0);
    let layers = SceneLayers::build(&project, &full_mask(), None);
    let video = solid(VIDEO, [GREY, GREY, GREY, 255]);
    let still = bench
        .plain(&project, &layers, at(100.0))
        .drawn_bounds()
        .expect("an image");
    assert_eq!(
        (still.0, still.1),
        (640, 360),
        "position is its top-left corner, as a percentage"
    );
    assert!(
        still.2.abs_diff(960) <= 1 && still.3.abs_diff(540) <= 1,
        "{still:?}"
    );

    let zoomed = bench.draw(
        &project,
        Some(&layers),
        at(100.0),
        Camera {
            scale: 2.0,
            x: -0.5,
            y: -0.5,
        },
        &video,
        None,
    );
    let zoomed = zoomed.drawn_bounds().expect("an image");
    assert_eq!(
        (zoomed.0, zoomed.1),
        (640, 360),
        "the corner at the middle stays there through a 2× zoom"
    );
    assert!(
        zoomed.2.abs_diff(1279) <= 1 && zoomed.3.abs_diff(719) <= 1,
        "and it doubles in size: {zoomed:?}"
    );
}

#[test]
fn padding_moves_annotations_and_the_cursor_with_the_frame() {
    let Some(mut bench) = Bench::new() else {
        return;
    };
    let mut project = project();
    project.look.padding = Padding::uniform(50.0);
    let image = AnnotationContent::Image {
        data_url: png_data_url((16, 9), [255, 0, 0, 255]),
    };
    project.timeline.annotations = vec![annotation(image, (0.0, 0.0, 10.0, 10.0), 0)];
    let layers = SceneLayers::build(&project, &full_mask(), None);
    let video = solid(VIDEO, [GREY, GREY, GREY, 255]);
    let picture = bench.draw(
        &project,
        Some(&layers),
        at(100.0),
        Camera::IDENTITY,
        &video,
        None,
    );
    let stage = (f64::from(STAGE.0), f64::from(STAGE.1));
    let video_size = (f64::from(VIDEO.0), f64::from(VIDEO.1));
    let frame = layout::compute(stage, &project.look.padding, &project.look.crop, video_size).frame;
    assert!(
        frame.x > 10.0,
        "padding should move the frame in, but it's at {}",
        frame.x
    );
    let (x, y) = (frame.x.round() as u32, frame.y.round() as u32);
    assert!(
        picture.at(x + 4, y + 4)[0] > 200,
        "the image starts at the frame's corner"
    );
    let before = picture.at(x - 4, y + 4);
    assert!(before[0] < 200, "and not before it: {before:?}");
}

#[test]
fn later_layers_go_on_top_whatever_the_order_they_were_added() {
    let Some(mut bench) = Bench::new() else {
        return;
    };
    let mut project = project();
    let red = AnnotationContent::Image {
        data_url: png_data_url((16, 9), [255, 0, 0, 255]),
    };
    let green = AnnotationContent::Image {
        data_url: png_data_url((16, 9), [0, 255, 0, 255]),
    };
    project.timeline.annotations = vec![
        annotation(red, (25.0, 25.0, 25.0, 25.0), 2),
        annotation(green, (25.0, 25.0, 25.0, 25.0), 1),
    ];
    let layers = SceneLayers::build(&project, &full_mask(), None);
    let [r, g, _, _] = bench.plain(&project, &layers, at(100.0)).at(400, 240);
    assert!(r > 200 && g < 50, "layer 2 should win, but got {r},{g}");
}

#[test]
fn an_image_keeps_its_shape_inside_its_box() {
    let Some(mut bench) = Bench::new() else {
        return;
    };
    let mut project = project();
    let square = AnnotationContent::Image {
        data_url: png_data_url((10, 10), [255, 0, 0, 255]),
    };
    project.timeline.annotations = vec![annotation(square, (0.0, 0.0, 50.0, 50.0), 0)];
    let layers = SceneLayers::build(&project, &full_mask(), None);
    let (left, top, right, bottom) = bench
        .plain(&project, &layers, at(100.0))
        .drawn_bounds()
        .expect("an image");
    let (width, height) = (right - left + 1, bottom - top + 1);
    assert!(width.abs_diff(height) <= 2, "{width}×{height} isn't square");
    assert!(height.abs_diff(360) <= 2);
}

#[test]
fn a_broken_image_draws_nothing_rather_than_failing() {
    let Some(mut bench) = Bench::new() else {
        return;
    };
    let mut project = project();
    let broken = AnnotationContent::Image {
        data_url: "data:image/png;base64,AAAA".to_string(),
    };
    project.timeline.annotations = vec![annotation(broken, (0.0, 0.0, 50.0, 50.0), 0)];
    let layers = SceneLayers::build(&project, &full_mask(), None);
    assert_eq!(
        bench.plain(&project, &layers, at(100.0)).drawn_bounds(),
        None
    );
}

/// Vertical stripes one video pixel wide, so 20 stage pixels wide.
fn stripes() -> Vec<u8> {
    let mut pixels = Vec::new();
    for _ in 0..VIDEO.1 {
        for x in 0..VIDEO.0 {
            let value = if x % 2 == 0 { 30 } else { 220 };
            pixels.extend_from_slice(&[value, value, value, 255]);
        }
    }
    pixels
}

#[test]
fn a_blur_annotation_blurs_only_its_own_area() {
    let Some(mut bench) = Bench::new() else {
        return;
    };
    let mut project = project();
    project.timeline.annotations = vec![annotation(
        AnnotationContent::Blur {
            strength: 30.0,
            fill: None,
        },
        (25.0, 25.0, 50.0, 50.0),
        0,
    )];
    let layers = SceneLayers::build(&project, &full_mask(), None);
    let video = stripes();
    let picture = bench.draw(
        &project,
        Some(&layers),
        at(100.0),
        Camera::IDENTITY,
        &video,
        None,
    );
    let plain = project_without_annotations(&project);
    let plain_layers = SceneLayers::build(&plain, &full_mask(), None);
    let sharp = bench.draw(
        &plain,
        Some(&plain_layers),
        at(100.0),
        Camera::IDENTITY,
        &video,
        None,
    );

    let inside = (420, 260, 860, 460);
    let outside = (20, 20, 300, 160);
    assert!(
        picture.roughness(inside) < sharp.roughness(inside) * 0.4,
        "inside went from {} to {}",
        sharp.roughness(inside),
        picture.roughness(inside)
    );
    assert_eq!(
        picture.roughness(outside),
        sharp.roughness(outside),
        "outside isn't touched"
    );
    // And the blur's edges are clean: right next to it, the picture's unchanged.
    assert_eq!(picture.at(310, 300), sharp.at(310, 300));
}

fn project_without_annotations(project: &Project) -> Project {
    let mut plain = project.clone();
    plain.timeline.annotations.clear();
    plain
}

#[test]
fn a_blur_can_have_a_solid_fill_over_it() {
    let Some(mut bench) = Bench::new() else {
        return;
    };
    let mut project = project();
    project.timeline.annotations = vec![annotation(
        AnnotationContent::Blur {
            strength: 20.0,
            fill: Some("#000000".to_string()),
        },
        (25.0, 25.0, 50.0, 50.0),
        0,
    )];
    let layers = SceneLayers::build(&project, &full_mask(), None);
    let picture = bench.draw(
        &project,
        Some(&layers),
        at(100.0),
        Camera::IDENTITY,
        &stripes(),
        None,
    );
    assert_eq!(
        picture.mean((340, 200, 940, 520)),
        [0.0, 0.0, 0.0],
        "the whole box is covered"
    );
    assert!(
        picture.mean((20, 20, 300, 160))[0] > 50.0,
        "and nothing outside it"
    );
}

#[test]
fn blur_keeps_colours_right_on_the_encoders_pixel_format() {
    let Some(mut bench) = Bench::with_format(wgpu::TextureFormat::Bgra8Unorm) else {
        return;
    };
    let mut project = project();
    project.timeline.annotations = vec![annotation(
        AnnotationContent::Blur {
            strength: 20.0,
            fill: None,
        },
        (25.0, 25.0, 50.0, 50.0),
        0,
    )];
    let layers = SceneLayers::build(&project, &full_mask(), None);
    let red = solid(VIDEO, [250, 10, 10, 255]);
    let picture = bench.draw(
        &project,
        Some(&layers),
        at(100.0),
        Camera::IDENTITY,
        &red,
        None,
    );
    // The read-back is in the target's order, which is blue first. Red has to stay red, inside and out.
    for (x, y) in [(640, 360), (20, 20)] {
        let [blue, _, red_channel, _] = picture.at(x, y);
        assert!(
            red_channel > 230 && blue < 30,
            "swapped at {x},{y}: {:?}",
            picture.at(x, y)
        );
    }
}

#[test]
fn text_lands_inside_its_box() {
    let Some(mut bench) = Bench::new() else {
        return;
    };
    let mut project = project();
    let text = AnnotationContent::Text {
        text: "Hello".to_string(),
        style: TextStyle {
            color: "#FFFF00".to_string(),
            ..TextStyle::default()
        },
    };
    project.timeline.annotations = vec![annotation(text, (25.0, 25.0, 50.0, 50.0), 0)];
    let layers = SceneLayers::build(&project, &full_mask(), None);
    let (left, top, right, bottom) = bench
        .plain(&project, &layers, at(100.0))
        .drawn_bounds()
        .expect("text");
    assert!(
        left >= 320 && top >= 180 && right <= 960 && bottom <= 540,
        "{left},{top} to {right},{bottom}"
    );
}

#[test]
fn text_stays_the_same_shape_while_zooming() {
    let Some(mut bench) = Bench::new() else {
        return;
    };
    let mut project = project();
    let text = AnnotationContent::Text {
        text: "Hello".to_string(),
        style: TextStyle::default(),
    };
    project.timeline.annotations = vec![annotation(text, (25.0, 25.0, 50.0, 50.0), 0)];
    let layers = SceneLayers::build(&project, &full_mask(), None);
    let video = solid(VIDEO, [GREY, GREY, GREY, 255]);
    let bounds = |bench: &mut Bench, scale: f64| {
        let camera = Camera {
            scale,
            x: (1.0 - scale) / 2.0,
            y: (1.0 - scale) / 2.0,
        };
        let (left, top, right, bottom) = bench
            .draw(&project, Some(&layers), at(100.0), camera, &video, None)
            .drawn_bounds()
            .expect("text");
        (f64::from(right - left), f64::from(bottom - top))
    };
    let base = bounds(&mut bench, 1.0);
    // Between the sizes it's drawn ahead at, it's scaled down, so its size has to track the zoom.
    for scale in [1.1, 1.4, 2.0] {
        let found = bounds(&mut bench, scale);
        assert!(
            (found.0 / base.0 - scale).abs() < 0.1,
            "at {scale}× the width is {found:?}, from {base:?}"
        );
        assert!(
            (found.1 / base.1 - scale).abs() < 0.12,
            "at {scale}× the height is {found:?}, from {base:?}"
        );
    }
}

fn webcam_card() -> Vec<u8> {
    // Left half red, right half blue, to show mirroring.
    let mut pixels = Vec::new();
    for _ in 0..90 {
        for x in 0..160 {
            pixels.extend_from_slice(if x < 80 {
                &[255, 0, 0, 255]
            } else {
                &[0, 0, 255, 255]
            });
        }
    }
    pixels
}

fn webcam_project(mirror: bool) -> Project {
    let mut project = project();
    project.webcam = Webcam {
        enabled: true,
        mirror,
        width: 40.0,
        height: 40.0,
        position: WebcamPosition::BottomRight,
        shadow: 0.0,
        react_to_zoom: false,
        ..Webcam::default()
    };
    project
}

#[test]
fn the_webcam_sits_in_its_corner() {
    let Some(mut bench) = Bench::new() else {
        return;
    };
    let project = webcam_project(false);
    let layers = SceneLayers::build(&project, &full_mask(), None);
    let video = solid(VIDEO, [GREY, GREY, GREY, 255]);
    let card = webcam_card();
    let picture = bench.draw(
        &project,
        Some(&layers),
        at(100.0),
        Camera::IDENTITY,
        &video,
        Some((&card, (160, 90))),
    );
    let (left, top, right, bottom) = picture.drawn_bounds().expect("a webcam");
    assert!(
        left > 640 && top > 360,
        "it's in the bottom-right, from {left},{top}"
    );
    assert!(
        right < 1280 && bottom < 720,
        "with a margin, to {right},{bottom}"
    );
}

#[test]
fn the_webcam_mirrors_when_set() {
    let Some(mut bench) = Bench::new() else {
        return;
    };
    let video = solid(VIDEO, [GREY, GREY, GREY, 255]);
    let card = webcam_card();
    let mut left_of_bubble_is_red = Vec::new();
    for mirror in [false, true] {
        let project = webcam_project(mirror);
        let layers = SceneLayers::build(&project, &full_mask(), None);
        let picture = bench.draw(
            &project,
            Some(&layers),
            at(100.0),
            Camera::IDENTITY,
            &video,
            Some((&card, (160, 90))),
        );
        let (left, top, right, bottom) = picture.drawn_bounds().expect("a webcam");
        let [r, _, b, _] = picture.at(left + (right - left) / 8, (top + bottom) / 2);
        left_of_bubble_is_red.push(r > b);
    }
    assert_eq!(left_of_bubble_is_red, [true, false]);
}

#[test]
fn the_webcam_stays_put_through_a_zoom_but_can_shrink_with_it() {
    let Some(mut bench) = Bench::new() else {
        return;
    };
    let video = solid(VIDEO, [GREY, GREY, GREY, 255]);
    let card = webcam_card();
    let camera = Camera {
        scale: 2.0,
        x: -0.5,
        y: -0.5,
    };

    let fixed = webcam_project(false);
    let layers = SceneLayers::build(&fixed, &full_mask(), None);
    let still = bench
        .draw(
            &fixed,
            Some(&layers),
            at(100.0),
            Camera::IDENTITY,
            &video,
            Some((&card, (160, 90))),
        )
        .drawn_bounds();
    // `layers.zoom` comes from the project's own camera path, which here has no zooms, so a camera
    // passed in by hand moves the video and leaves the bubble alone.
    let zoomed = bench
        .draw(
            &fixed,
            Some(&layers),
            at(100.0),
            camera,
            &video,
            Some((&card, (160, 90))),
        )
        .drawn_bounds();
    assert_eq!(still, zoomed, "the camera doesn't carry it along");

    let mut reacting = fixed.clone();
    reacting.webcam.react_to_zoom = true;
    reacting.timeline.zooms = vec![reel_core::timeline::Zoom {
        id: "z".into(),
        start: at(0.0),
        end: at(5000.0),
        depth: 2,
        focus: Point { x: 0.5, y: 0.5 },
        mode: reel_core::timeline::ZoomMode::default(),
    }];
    let reacting_layers = SceneLayers::build(&reacting, &full_mask(), None);
    let settled = at(3000.0);
    let picture = bench.draw(
        &reacting,
        Some(&reacting_layers),
        settled,
        Camera::IDENTITY,
        &video,
        Some((&card, (160, 90))),
    );
    let (left, _, right, _) = picture.drawn_bounds().expect("a webcam");
    let (fixed_left, _, fixed_right, _) = still.expect("a webcam");
    assert!(
        right - left < (fixed_right - fixed_left) * 3 / 4,
        "{} should have shrunk from {}",
        right - left,
        fixed_right - fixed_left
    );
}

fn caption_project(animation: CaptionAnimation) -> Project {
    let mut project = project();
    project.caption_style.show = true;
    project.caption_style.animation = animation;
    project.caption_style.background_opacity = 0.8;
    project.timeline.captions = vec![Caption {
        id: "c".into(),
        start: at(1000.0),
        end: at(3000.0),
        text: "Hello there".to_string(),
        words: vec![],
    }];
    project
}

#[test]
fn captions_show_centred_at_the_bottom_during_their_time() {
    let Some(mut bench) = Bench::new() else {
        return;
    };
    let project = caption_project(CaptionAnimation::None);
    let layers = SceneLayers::build(&project, &full_mask(), None);
    assert!(
        bench
            .plain(&project, &layers, at(500.0))
            .drawn_bounds()
            .is_none()
    );
    assert!(
        bench
            .plain(&project, &layers, at(3500.0))
            .drawn_bounds()
            .is_none()
    );
    let (left, top, right, bottom) = bench
        .plain(&project, &layers, at(2000.0))
        .drawn_bounds()
        .expect("a caption");
    assert!(
        (i64::from(left + right) - 1280).abs() <= 4,
        "centred: {left} to {right}"
    );
    assert!(top > 500, "towards the bottom, from {top}");
    // The default offset is 3% of the height.
    assert!(bottom.abs_diff(698) <= 3, "its bottom edge is at {bottom}");
}

#[test]
fn turning_captions_off_hides_them() {
    let Some(mut bench) = Bench::new() else {
        return;
    };
    let mut project = caption_project(CaptionAnimation::None);
    project.caption_style.show = false;
    let layers = SceneLayers::build(&project, &full_mask(), None);
    assert!(
        bench
            .plain(&project, &layers, at(2000.0))
            .drawn_bounds()
            .is_none()
    );
}

#[test]
fn captions_stay_put_through_a_zoom() {
    let Some(mut bench) = Bench::new() else {
        return;
    };
    let project = caption_project(CaptionAnimation::None);
    let layers = SceneLayers::build(&project, &full_mask(), None);
    let video = solid(VIDEO, [GREY, GREY, GREY, 255]);
    let still = bench
        .draw(
            &project,
            Some(&layers),
            at(2000.0),
            Camera::IDENTITY,
            &video,
            None,
        )
        .drawn_bounds();
    let zoomed = bench
        .draw(
            &project,
            Some(&layers),
            at(2000.0),
            Camera {
                scale: 2.0,
                x: -0.5,
                y: -0.5,
            },
            &video,
            None,
        )
        .drawn_bounds();
    assert_eq!(still, zoomed);
}

#[test]
fn a_caption_fades_in() {
    let Some(mut bench) = Bench::new() else {
        return;
    };
    let project = caption_project(CaptionAnimation::Fade);
    let layers = SceneLayers::build(&project, &full_mask(), None);
    let settled = bench.plain(&project, &layers, at(2000.0));
    let (left, top, right, bottom) = settled.drawn_bounds().expect("a caption");
    // Inside the caption's box, away from the letters.
    let spot = (left + 6, (top + bottom) / 2);
    let darkness = |picture: &Picture| f64::from(GREY) - f64::from(picture.at(spot.0, spot.1)[0]);
    let early = bench.plain(&project, &layers, at(1010.0));
    assert!(
        darkness(&early) < darkness(&settled) * 0.6,
        "{} against {}",
        darkness(&early),
        darkness(&settled)
    );
    assert!(right > left);
}

#[test]
fn video_motion_blur_follows_the_camera_and_only_when_it_moves() {
    let Some(mut bench) = Bench::new() else {
        return;
    };
    let mut project = project();
    project.zoom_motion.motion_blur = 1.0;
    project.timeline.zooms = vec![reel_core::timeline::Zoom {
        id: "z".into(),
        start: at(1000.0),
        end: at(4000.0),
        depth: 5,
        focus: Point { x: 0.5, y: 0.5 },
        mode: reel_core::timeline::ZoomMode::default(),
    }];
    let mut steady = project.clone();
    steady.zoom_motion.motion_blur = 0.0;

    let blurred_layers = SceneLayers::build(&project, &full_mask(), None);
    let steady_layers = SceneLayers::build(&steady, &full_mask(), None);
    let video = stripes();
    let draw = |bench: &mut Bench, project: &Project, layers: &SceneLayers, ms: f64| {
        bench.draw(
            project,
            Some(layers),
            at(ms),
            layers.camera.sample(at(ms)),
            &video,
            None,
        )
    };

    // Before the zoom starts, nothing's moving, so the setting changes nothing.
    assert_eq!(
        draw(&mut bench, &project, &blurred_layers, 200.0).pixels,
        draw(&mut bench, &steady, &steady_layers, 200.0).pixels
    );

    // Find the moment the camera is moving fastest.
    let fastest = (0..200)
        .map(|index| 1000.0 + f64::from(index) * 10.0)
        .max_by(|a, b| {
            let speed = |ms: f64| {
                (blurred_layers.camera.sample(at(ms)).scale
                    - blurred_layers.camera.sample(at(ms - 10.0)).scale)
                    .abs()
            };
            speed(*a).total_cmp(&speed(*b))
        })
        .unwrap();
    // Away from the middle, where a zoom moves the picture the most.
    let area = (40, 100, 300, 620);
    let moving_blurred = draw(&mut bench, &project, &blurred_layers, fastest).roughness(area);
    let moving_sharp = draw(&mut bench, &steady, &steady_layers, fastest).roughness(area);
    assert!(
        moving_blurred < moving_sharp * 0.9,
        "{moving_blurred} against {moving_sharp} at {fastest} ms"
    );
}

/// Not a check: writes a frame with everything on it for a person to look at, if asked to.
#[test]
fn write_a_frame_for_looking_at() {
    let Some(path) = std::env::var_os("REEL_OVERLAY_SHOT") else {
        return;
    };
    let Some(mut bench) = Bench::new() else {
        return;
    };
    let mut project = project();
    project.look.padding = Padding::uniform(30.0);
    project.look.corner_radius = 40.0;
    project.look.shadow = 0.7;
    project.cursor.click_effect = ClickEffect::Echo;
    project.webcam = Webcam {
        enabled: true,
        ..Webcam::default()
    };
    project.caption_style.show = true;
    project.caption_style.background_opacity = 0.7;
    project.caption_style.animation = CaptionAnimation::None;
    project.timeline.captions = vec![Caption {
        id: "c".into(),
        start: at(0.0),
        end: at(5000.0),
        text: "Captions go down here, wrapped and centred".to_string(),
        words: vec![],
    }];
    project.timeline.annotations = vec![
        annotation(
            arrow(ArrowDirection::UpRight, "#34B27B"),
            (5.0, 5.0, 25.0, 30.0),
            1,
        ),
        annotation(
            arrow(ArrowDirection::Down, "#EF4444"),
            (32.0, 5.0, 12.0, 25.0),
            2,
        ),
        annotation(
            AnnotationContent::Text {
                text: "A text note".to_string(),
                style: TextStyle {
                    background: "#1F2937".to_string(),
                    ..TextStyle::default()
                },
            },
            (50.0, 6.0, 30.0, 14.0),
            3,
        ),
        annotation(
            AnnotationContent::Blur {
                strength: 18.0,
                fill: None,
            },
            (8.0, 55.0, 30.0, 30.0),
            4,
        ),
    ];
    let track = CursorTrack {
        samples: vec![
            sample(0.0, 0.62, 0.55, Interaction::Move),
            sample(100.0, 0.62, 0.55, Interaction::Click),
            sample(10_000.0, 0.62, 0.55, Interaction::Move),
        ],
    };
    let layers = SceneLayers::build(&project, &full_mask(), Some(&track));
    let mut video = Vec::new();
    for y in 0..VIDEO.1 {
        for x in 0..VIDEO.0 {
            let on_grid = x % 8 == 0 || y % 6 == 0;
            let (r, g, b) = if on_grid {
                (235, 235, 240)
            } else {
                ((x * 4) as u8, (y * 7) as u8, 170)
            };
            video.extend_from_slice(&[r, g, b, 255]);
        }
    }
    let mut card = Vec::new();
    for y in 0..90_u32 {
        for x in 0..160_u32 {
            let dx = f64::from(x) - 80.0;
            let dy = f64::from(y) - 45.0;
            let face = (dx * dx + dy * dy).sqrt() < 26.0;
            card.extend_from_slice(if face {
                &[238, 196, 160, 255]
            } else {
                &[60, 90, 120, 255]
            });
        }
    }
    let picture = bench.draw(
        &project,
        Some(&layers),
        at(400.0),
        Camera::IDENTITY,
        &video,
        Some((&card, (160, 90))),
    );
    let file = std::io::BufWriter::new(std::fs::File::create(path).unwrap());
    let mut encoder = png::Encoder::new(file, STAGE.0, STAGE.1);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .unwrap()
        .write_image_data(&picture.pixels)
        .unwrap();
}
