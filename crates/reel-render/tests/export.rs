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
        let view = target.texture.create_view(&wgpu::TextureViewDescriptor::default());
        let mut commands = gpu.device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        {
            let _pass = commands.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color { r: level, g: level, b: level, a: 1.0 }),
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
        decoder.frame_at(SourceTime::from_ms(ms)).unwrap().unwrap().pts
    };
    assert_eq!(pts_at(&mut decoder, 0.0), SourceTime::ZERO);
    assert_eq!(pts_at(&mut decoder, 450.0), SourceTime::from_ms(400.0), "holds the frame in between");
    assert_eq!(pts_at(&mut decoder, 500.0), SourceTime::from_ms(500.0));
    assert_eq!(pts_at(&mut decoder, 5000.0), SourceTime::from_ms(5000.0), "a long jump forward");
    assert_eq!(pts_at(&mut decoder, 1000.0), SourceTime::from_ms(1000.0), "a jump back");
    assert_eq!(pts_at(&mut decoder, 99_000.0), SourceTime::from_ms(7900.0), "past the end holds the last frame");
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
    assert_eq!(files_in(output_folder.path()), ["out.mp4"], "no temporary files left behind");

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
    assert!(files_in(output_folder.path()).is_empty(), "{:?}", files_in(output_folder.path()));
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
    project.timeline.apply(&Edit::SplitClip { at: at(2000.0) }).unwrap();
    project.timeline.apply(&Edit::SplitClip { at: at(4000.0) }).unwrap();
    let ids: Vec<_> = project.timeline.clips.iter().map(|clip| clip.id.clone()).collect();
    project.timeline.apply(&Edit::ReorderClip { id: ids[2].clone(), to_index: 0 }).unwrap();
    project.timeline.apply(&Edit::SetClipSpeed { id: ids[1].clone(), speed: 2.0 }).unwrap();
    // A gap, which the export should skip.
    let middle_start = project.timeline.clips.iter().find(|clip| clip.id == ids[0]).unwrap().start;
    project.timeline.apply(&Edit::MoveClip { id: ids[1].clone(), to: middle_start + Micros::from_secs(4) }).unwrap();

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
