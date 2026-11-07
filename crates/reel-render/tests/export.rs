//! The whole pipeline: decode, render, encode. These make their own test video, so they only need a
//! Mac with a GPU and a hardware encoder. CI's virtual Macs may have neither, so they're skipped there.

#![cfg(target_os = "macos")]

use std::path::Path;

use reel_core::edit::Edit;
use reel_core::project::{Background, MediaRef, Padding, Project};
use reel_core::time::{Micros, SourceTime, TimelineTime};
use reel_core::timeline::Timeline;
use reel_media::macos::{Encoder, EncoderSettings, probe};
use reel_render::Headless;
use reel_render::apple::TextureCache;
use reel_render::export::{Scene, export_mp4, render_still};

const SIZE: (u32, u32) = (320, 180);
const FPS: u32 = 10;
const SECONDS: i64 = 8;

fn gpu() -> Option<Headless> {
    if std::env::var_os("CI").is_some() {
        println!("Skipped on CI, which may have no GPU or hardware encoder.");
        return None;
    }
    let gpu = Headless::new();
    if gpu.is_none() {
        println!("No GPU on this machine, so this test is skipped.");
    }
    gpu
}

/// How bright a frame of the test video is: it steps up with each second, so a frame shows where it
/// came from. High contrast between steps survives compression.
fn level_at(second: i64) -> f64 {
    0.1 + 0.1 * second as f64
}

fn make_video(gpu: &Headless, path: &Path) {
    let cache = TextureCache::new(&gpu.device);
    let mut encoder = Encoder::create(
        path,
        EncoderSettings {
            width: SIZE.0,
            height: SIZE.1,
            fps: FPS,
            bitrate: 2_000_000,
            keyframe_interval: FPS,
            audio: false,
        },
    )
    .unwrap();
    for frame in 0..i64::from(FPS) * SECONDS {
        let level = level_at(frame / i64::from(FPS));
        let buffer = encoder.next_buffer().unwrap();
        let target = cache.wrap_target(&buffer, SIZE);
        let view = target
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut commands = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        {
            let _pass = commands.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: level,
                            g: level,
                            b: level,
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
        }
        gpu.queue.submit(Some(commands.finish()));
        let _ = gpu.device.poll(wgpu::PollType::wait_indefinitely());
        encoder.append_video(&buffer).unwrap();
    }
    encoder.finish().unwrap();
}

fn project_for(video: &Path) -> Project {
    let mut project = Project {
        timeline: Timeline::for_recording(Micros::from_secs(SECONDS)),
        ..Project::default()
    };
    project.media.video = MediaRef::absolute(video);
    project.look.background = Background::Color("#000000".to_string());
    project.look.padding = Padding::uniform(0.0);
    project.look.corner_radius = 0.0;
    project.look.shadow = 0.0;
    project.look.aspect_ratio = "16:9".to_string();
    project.export.mp4_frame_rate = FPS;
    project
}

/// The middle pixel's brightness at a time, from 0 to 1, read back through the real decoder.
fn brightness(video: &Path, time_ms: f64) -> f64 {
    let scene = Scene::new(project_for(video), None).unwrap();
    let pixels = render_still(&scene, TimelineTime::from_ms(time_ms)).unwrap();
    let at = ((scene.stage.1 / 2 * scene.stage.0 + scene.stage.0 / 2) * 4) as usize;
    f64::from(pixels[at]) / 255.0
}

fn assert_shows_second(video: &Path, time_ms: f64, second: i64) {
    let measured = brightness(video, time_ms);
    let expected = level_at(second);
    assert!(
        (measured - expected).abs() < 0.04,
        "at {time_ms} ms expected second {second} (level {expected:.2}), got level {measured:.3}"
    );
}

fn files_in(folder: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(folder)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

#[test]
fn the_decoder_finds_the_right_frame_going_forward_and_back() {
    let Some(gpu) = gpu() else { return };
    let dir = tempfile::tempdir().unwrap();
    let video = dir.path().join("source.mp4");
    make_video(&gpu, &video);
    let mut decoder = reel_media::macos::Decoder::open(&video).unwrap();
    let pts_at = |decoder: &mut reel_media::macos::Decoder, ms: f64| {
        decoder
            .frame_at(SourceTime::from_ms(ms))
            .unwrap()
            .unwrap()
            .pts
    };
    assert_eq!(pts_at(&mut decoder, 0.0), SourceTime::ZERO);
    assert_eq!(
        pts_at(&mut decoder, 450.0),
        SourceTime::from_ms(400.0),
        "holds the frame in between"
    );
    assert_eq!(pts_at(&mut decoder, 500.0), SourceTime::from_ms(500.0));
    assert_eq!(
        pts_at(&mut decoder, 5000.0),
        SourceTime::from_ms(5000.0),
        "a long jump forward"
    );
    assert_eq!(
        pts_at(&mut decoder, 1000.0),
        SourceTime::from_ms(1000.0),
        "a jump back"
    );
    assert_eq!(
        pts_at(&mut decoder, 99_000.0),
        SourceTime::from_ms(7900.0),
        "past the end holds the last frame"
    );
}

#[test]
fn an_export_is_the_right_length_and_leaves_one_file() {
    let Some(gpu) = gpu() else { return };
    let dir = tempfile::tempdir().unwrap();
    let video = dir.path().join("source.mp4");
    make_video(&gpu, &video);

    let output_folder = tempfile::tempdir().unwrap();
    let output = output_folder.path().join("out.mp4");
    let scene = Scene::new(project_for(&video), None).unwrap();
    let report = export_mp4(&scene, &output, |_, _| true).unwrap();
    assert_eq!(report.frames, 80);
    assert_eq!(
        files_in(output_folder.path()),
        ["out.mp4"],
        "no temporary files left behind"
    );

    let info = probe(&output).unwrap();
    assert_eq!((info.width, info.height), SIZE);
    assert_eq!(info.duration, Micros::from_secs(SECONDS));
    assert_shows_second(&output, 3500.0, 3);
}

#[test]
fn cancelling_leaves_nothing_behind() {
    let Some(gpu) = gpu() else { return };
    let dir = tempfile::tempdir().unwrap();
    let video = dir.path().join("source.mp4");
    make_video(&gpu, &video);

    let output_folder = tempfile::tempdir().unwrap();
    let output = output_folder.path().join("out.mp4");
    let scene = Scene::new(project_for(&video), None).unwrap();
    let result = export_mp4(&scene, &output, |done, _| done < 20);
    assert!(result.is_err());
    assert!(
        files_in(output_folder.path()).is_empty(),
        "{:?}",
        files_in(output_folder.path())
    );
}

#[test]
fn an_export_shows_the_right_footage_after_moves_cuts_and_speed_changes() {
    let Some(gpu) = gpu() else { return };
    let dir = tempfile::tempdir().unwrap();
    let video = dir.path().join("source.mp4");
    make_video(&gpu, &video);

    // Three clips of the 8 second video: 0-2 s, 2-4 s at 2×, and 4-8 s moved to the front.
    let mut project = project_for(&video);
    let at = TimelineTime::from_ms;
    project
        .timeline
        .apply(&Edit::SplitClip { at: at(2000.0) })
        .unwrap();
    project
        .timeline
        .apply(&Edit::SplitClip { at: at(4000.0) })
        .unwrap();
    let ids: Vec<_> = project
        .timeline
        .clips
        .iter()
        .map(|clip| clip.id.clone())
        .collect();
    project
        .timeline
        .apply(&Edit::ReorderClip {
            id: ids[2].clone(),
            to_index: 0,
        })
        .unwrap();
    project
        .timeline
        .apply(&Edit::SetClipSpeed {
            id: ids[1].clone(),
            speed: 2.0,
        })
        .unwrap();
    // A gap, which the export should skip.
    let middle_start = project
        .timeline
        .clips
        .iter()
        .find(|clip| clip.id == ids[0])
        .unwrap()
        .start;
    project
        .timeline
        .apply(&Edit::MoveClip {
            id: ids[1].clone(),
            to: middle_start + Micros::from_secs(4),
        })
        .unwrap();

    let output_folder = tempfile::tempdir().unwrap();
    let output = output_folder.path().join("out.mp4");
    let scene = Scene::new(project, None).unwrap();
    let report = export_mp4(&scene, &output, |_, _| true).unwrap();
    // 4 s + 2 s + 1 s, with the gap skipped.
    assert_eq!(report.frames, 70);
    assert_eq!(probe(&output).unwrap().duration, Micros::from_secs(7));

    // The clip moved to the front shows seconds 4 to 7 first.
    assert_shows_second(&output, 500.0, 4);
    assert_shows_second(&output, 3500.0, 7);
    // Then the original first clip, seconds 0 and 1.
    assert_shows_second(&output, 4500.0, 0);
    assert_shows_second(&output, 5500.0, 1);
    // Then the 2× clip, which covers seconds 2 and 3 in half the time.
    assert_shows_second(&output, 6250.0, 2);
    assert_shows_second(&output, 6750.0, 3);
}

#[test]
fn a_missing_video_says_so() {
    let project = project_for(Path::new("/nowhere/at/all.mp4"));
    let error = Scene::new(project, None).err().expect("should fail");
    assert!(error.0.contains("wasn't found"), "{}", error.0);
}

/// A folder with the test video in it, if this machine can make one.
fn with_video() -> Option<(tempfile::TempDir, std::path::PathBuf)> {
    let gpu = gpu()?;
    let folder = tempfile::tempdir().unwrap();
    let video = folder.path().join("video.mp4");
    make_video(&gpu, &video);
    Some((folder, video))
}

fn gif_project(video: &Path, gif_loop: bool) -> Project {
    let mut project = project_for(video);
    project.export.format = reel_core::project::ExportFormat::Gif;
    project.export.gif_frame_rate = 15;
    project.export.gif_size = reel_core::project::GifSize::Small;
    project.export.gif_loop = gif_loop;
    project
}

/// Counts the frames in a GIF, and whether it repeats, by reading it back.
fn read_gif(path: &Path) -> (u16, u16, usize, Vec<u16>, bool) {
    let bytes = std::fs::read(path).unwrap();
    let repeats = bytes.windows(11).any(|part| part == b"NETSCAPE2.0");
    let mut options = gif::DecodeOptions::new();
    options.set_color_output(gif::ColorOutput::Indexed);
    let mut reader = options.read_info(std::io::Cursor::new(bytes)).unwrap();
    let (width, height) = (reader.width(), reader.height());
    let mut delays = Vec::new();
    while let Some(frame) = reader.read_next_frame().unwrap() {
        delays.push(frame.delay);
    }
    (width, height, delays.len(), delays, repeats)
}

#[test]
fn a_gif_has_the_right_frames_and_timing() {
    let Some((folder, video)) = with_video() else {
        return;
    };
    let scene = Scene::new(gif_project(&video, true), None).unwrap();
    let output = folder.path().join("out.gif");
    let report = reel_render::export::export_gif(&scene, &output, |_, _| true).unwrap();
    let (width, height, frames, delays, repeats) = read_gif(&output);
    assert_eq!(u64::try_from(frames).unwrap(), report.frames);
    assert!(height <= 480 && width > 0, "{width}×{height}");
    assert!(repeats, "looping is on");
    // Averaged, the frame rate is what was asked for, and not what rounding each delay would give.
    let centiseconds: u32 = delays.iter().map(|delay| u32::from(*delay)).sum();
    let fps = frames as f64 / (f64::from(centiseconds) / 100.0);
    assert!((fps - 15.0).abs() < 0.2, "{fps} fps from {delays:?}");
}

#[test]
fn a_gif_with_loop_off_plays_once() {
    let Some((folder, video)) = with_video() else {
        return;
    };
    let scene = Scene::new(gif_project(&video, false), None).unwrap();
    let output = folder.path().join("out.gif");
    reel_render::export::export_gif(&scene, &output, |_, _| true).unwrap();
    let (.., repeats) = read_gif(&output);
    assert!(!repeats, "with no repeat block, viewers play it once");
}

#[test]
fn cancelling_a_gif_leaves_nothing() {
    let Some((folder, video)) = with_video() else {
        return;
    };
    let scene = Scene::new(gif_project(&video, true), None).unwrap();
    let output = folder.path().join("out.gif");
    let result = reel_render::export::export_gif(&scene, &output, |done, _| done < 3);
    assert!(result.is_err());
    assert!(!output.exists());
    let left: Vec<_> = std::fs::read_dir(folder.path())
        .unwrap()
        .flatten()
        .filter(|entry| entry.file_name().to_string_lossy().starts_with("out.gif"))
        .collect();
    assert!(left.is_empty(), "{left:?}");
}

#[test]
fn quality_scales_the_output_size() {
    let Some((folder, video)) = with_video() else {
        return;
    };
    let mut project = project_for(&video);
    project.export.quality = reel_core::project::ExportQuality::Low;
    let scene = Scene::new(project, None).unwrap();
    let output = folder.path().join("out.mp4");
    let report = export_mp4(&scene, &output, |_, _| true).unwrap();
    assert_eq!(
        (report.width, report.height),
        reel_core::project::ExportQuality::Low.scaled_size(scene.stage)
    );
    assert!(report.width < scene.stage.0);
    let info = reel_media::macos::probe(&output).unwrap();
    assert_eq!((info.width, info.height), (report.width, report.height));
}

#[test]
fn a_video_background_shows_and_moves() {
    let Some((_folder, video)) = with_video() else {
        return;
    };
    let mut project = project_for(&video);
    // The video itself makes a fine moving background for this test.
    project.look.background = Background::Video(project.media.video.clone());
    project.look.padding = Padding::uniform(50.0);
    let scene = Scene::new(project, None).unwrap();
    assert!(scene.background_video.is_some());
    let early = render_still(&scene, TimelineTime::from_ms(100.0)).unwrap();
    let later = render_still(&scene, TimelineTime::from_ms(2500.0)).unwrap();
    // The top-left corner is background, not video.
    let corner = |pixels: &[u8]| {
        let mut total = 0_u64;
        for y in 0..10_usize {
            for x in 0..10_usize {
                let at = (y * scene.stage.0 as usize + x) * 4;
                total +=
                    u64::from(pixels[at]) + u64::from(pixels[at + 1]) + u64::from(pixels[at + 2]);
            }
        }
        total
    };
    assert_ne!(
        corner(&early),
        corner(&later),
        "the background should move with time"
    );
}

#[test]
fn a_gif_size_keeps_the_shape_and_stays_even() {
    use reel_core::project::GifSize;
    use reel_render::export::gif_size;
    assert_eq!(gif_size((1920, 1080), GifSize::Medium), (1280, 720));
    assert_eq!(gif_size((1920, 1080), GifSize::Large), (1920, 1080));
    assert_eq!(gif_size((1920, 1080), GifSize::Original), (1920, 1080));
    assert_eq!(gif_size((3840, 2160), GifSize::Large), (1920, 1080));
    assert_eq!(gif_size((1000, 2000), GifSize::Medium), (360, 720));
    let (width, height) = gif_size((1921, 1081), GifSize::Medium);
    assert!(width % 2 == 0 && height % 2 == 0);
}

#[test]
fn gif_delays_average_out_to_the_frame_rate() {
    use reel_render::export::gif_delay;
    for fps in [15_u32, 20, 25, 30] {
        let total: u64 = (0..u64::from(fps) * 60)
            .map(|frame| u64::from(gif_delay(frame, fps)))
            .sum();
        assert_eq!(total, 6000, "{fps} fps over a minute");
    }
    assert_eq!(gif_delay(0, 20), 5);
    assert_eq!(gif_delay(0, 25), 4);
}

/// A busy 1080p video with sound, of the kind that makes the encoder take real time. Flat frames encode
/// too quickly to show the problem the next test is about. Returns `None` if FFmpeg isn't installed.
fn busy_video_with_sound(path: &Path) -> Option<()> {
    let status = std::process::Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-y"])
        .args([
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=1920x1080:rate=60:duration=5",
        ])
        .args(["-f", "lavfi", "-i", "sine=frequency=440:duration=5"])
        .args([
            "-c:v", "libx264", "-preset", "veryfast", "-crf", "20", "-pix_fmt", "yuv420p",
        ])
        .args(["-c:a", "aac", "-b:a", "160k"])
        .arg(path)
        .status()
        .ok()?;
    status.success().then_some(())
}

/// This used to hang for good. AVAssetWriter can refuse video until it has more sound, and the export
/// wouldn't write more sound until the video was taken. A blur annotation's extra work was enough to
/// bring it out, in a real project with a real 1080p video.
#[test]
fn an_export_with_sound_and_a_blur_finishes() {
    if gpu().is_none() {
        return;
    }
    let folder = tempfile::tempdir().unwrap();
    let video = folder.path().join("video.mp4");
    if busy_video_with_sound(&video).is_none() {
        println!("FFmpeg isn't installed, so this test is skipped.");
        return;
    }

    let mut project = project_for(&video);
    project.timeline = Timeline::for_recording(Micros::from_secs(5));
    project.export.mp4_frame_rate = 30;
    project.timeline.annotations = vec![reel_core::timeline::Annotation {
        id: "blur".into(),
        start: TimelineTime::ZERO,
        end: TimelineTime::from_ms(5000.0),
        content: reel_core::timeline::AnnotationContent::Blur {
            strength: 18.0,
            fill: None,
        },
        position: reel_core::timeline::Point { x: 40.0, y: 60.0 },
        width: 25.0,
        height: 25.0,
        track: 0,
        layer: 0,
    }];
    let output = folder.path().join("out.mp4");
    let (done, finished) = std::sync::mpsc::channel();
    let target = output.clone();
    std::thread::spawn(move || {
        let scene = Scene::new(project, None).unwrap();
        let report = export_mp4(&scene, &target, |_, _| true).map_err(|error| error.0);
        let _ = done.send(report);
    });
    let report = finished
        .recv_timeout(std::time::Duration::from_secs(40))
        .expect("the export hung")
        .unwrap();
    assert_eq!(report.frames, 150);
    let info = probe(&output).unwrap();
    assert!(info.has_audio);
    assert!(
        (info.duration.secs() - 5.0).abs() < 0.1,
        "{}",
        info.duration.secs()
    );
}
