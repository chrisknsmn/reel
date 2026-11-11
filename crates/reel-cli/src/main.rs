//! Headless render and test harness.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use reel_core::format;
use reel_core::project::Project;
use reel_core::time::TimelineTime;

const USAGE: &str = "\
Usage:
  reel-cli render <project.reel> <output.mp4> [--fps <n>]
      Exports a project to an MP4.

  reel-cli frame <project.reel> --at-ms <ms> <output.png>
      Renders one frame of a project, exactly as the preview shows it.";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("render") => render(&args[1..]),
        Some("frame") => frame(&args[1..]),
        _ => Err(USAGE.to_string()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("{message}");
            ExitCode::FAILURE
        }
    }
}

/// Reads a project, along with the folder its relative paths start from.
#[allow(dead_code)]
fn load_project(path: &Path) -> Result<(Project, Option<PathBuf>), String> {
    let text = std::fs::read_to_string(path)
        .map_err(|error| format!("Couldn't read {}: {error}", path.display()))?;
    let project =
        format::from_str(&text).map_err(|error| format!("{}: {error}", path.display()))?;
    let folder = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .map(Path::to_path_buf);
    Ok((project, folder))
}

#[cfg(target_os = "macos")]
fn render(args: &[String]) -> Result<(), String> {
    use reel_render::export::{Scene, export_mp4};
    let (positional, fps) = match args {
        [a, b, flag, value] if flag == "--fps" => {
            let fps: u32 = value
                .parse()
                .map_err(|_| format!("Not a frame rate: {value}"))?;
            ([a, b], Some(fps))
        }
        [a, b] => ([a, b], None),
        _ => return Err(USAGE.to_string()),
    };
    let [input, output] = positional;
    let (mut project, folder) = load_project(Path::new(input))?;
    if let Some(fps) = fps {
        project.export.mp4_frame_rate = fps;
    }
    let scene = Scene::new(project, folder.as_deref()).map_err(|error| error.to_string())?;
    let mut last_percent = None;
    let report = export_mp4(&scene, Path::new(output), |done, total| {
        let percent = done * 100 / total.max(1);
        if last_percent != Some(percent) && percent % 10 == 0 {
            eprintln!("{percent}%");
            last_percent = Some(percent);
        }
        true
    })
    .map_err(|error| error.to_string())?;
    println!(
        "Wrote {output}: {}x{} at {} fps, {} frames, {:.1} Mbps, {:.1} s in {:.1} s ({:.2}x real time)",
        report.width,
        report.height,
        report.fps,
        report.frames,
        f64::from(report.bitrate) / 1e6,
        report.output_seconds,
        report.elapsed_seconds,
        report.realtime_factor(),
    );
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn render(_args: &[String]) -> Result<(), String> {
    Err(
        "Rendering isn't available on this OS yet. See Milestones 11 and 12 in BUILD_PLAN.md."
            .to_string(),
    )
}

#[cfg(target_os = "macos")]
fn frame(args: &[String]) -> Result<(), String> {
    use reel_render::export::{Scene, render_still};
    let [input, flag, ms, output] = args else {
        return Err(USAGE.to_string());
    };
    if flag != "--at-ms" {
        return Err(USAGE.to_string());
    }
    let ms: f64 = ms
        .parse()
        .map_err(|_| format!("Not a number of milliseconds: {ms}"))?;
    let (project, folder) = load_project(Path::new(input))?;
    let scene = Scene::new(project, folder.as_deref()).map_err(|error| error.to_string())?;
    let pixels =
        render_still(&scene, TimelineTime::from_ms(ms)).map_err(|error| error.to_string())?;
    let file = std::fs::File::create(output)
        .map_err(|error| format!("Couldn't write {output}: {error}"))?;
    let mut encoder =
        png::Encoder::new(std::io::BufWriter::new(file), scene.stage.0, scene.stage.1);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().map_err(|error| error.to_string())?;
    writer
        .write_image_data(&pixels)
        .map_err(|error| error.to_string())?;
    println!("Wrote {output}");
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn frame(_args: &[String]) -> Result<(), String> {
    Err(
        "Rendering isn't available on this OS yet. See Milestones 11 and 12 in BUILD_PLAN.md."
            .to_string(),
    )
}
