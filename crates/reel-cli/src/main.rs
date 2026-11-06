//! Headless render and test harness.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use reel_core::format;
use reel_core::project::Project;
use reel_core::recordly::{self, ImportOptions};
use reel_core::store;
use reel_core::time::{Micros, TimelineTime};

const USAGE: &str = "\
Usage:
  reel-cli convert <project.recordly> [output.reel] [--force] [--source-duration-ms <ms>]
      Converts a Recordly project into a Reel project, without changing the original.

      --force                  Overwrite the output if it exists.
      --source-duration-ms     How long the recording is. Only needed for old projects with trims but no clips.

  reel-cli render <project> <output.mp4> [--fps <n>]
      Exports a project to an MP4. The project can be .reel or .recordly.

  reel-cli frame <project> --at-ms <ms> <output.png>
      Renders one frame of a project, exactly as the preview shows it.";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("convert") => convert(&args[1..]),
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

fn convert(args: &[String]) -> Result<(), String> {
    let mut paths = Vec::new();
    let mut force = false;
    let mut source_duration = None;
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--force" => force = true,
            "--source-duration-ms" => {
                let value = iter.next().ok_or_else(|| USAGE.to_string())?;
                let ms: f64 = value
                    .parse()
                    .map_err(|_| format!("Not a number of milliseconds: {value}"))?;
                source_duration = Some(Micros::from_ms(ms));
            }
            other if other.starts_with("--") => return Err(USAGE.to_string()),
            other => paths.push(PathBuf::from(other)),
        }
    }
    let (input, output) = match paths.as_slice() {
        [input] => (input.clone(), input.with_extension(format::EXTENSION)),
        [input, output] => (input.clone(), output.clone()),
        _ => return Err(USAGE.to_string()),
    };
    if output.exists() && !force {
        return Err(format!(
            "{} already exists. Use --force to overwrite it.",
            output.display()
        ));
    }

    let text = std::fs::read_to_string(&input)
        .map_err(|error| format!("Couldn't read {}: {error}", input.display()))?;
    let options = ImportOptions {
        source_duration,
        find_sidecars: true,
    };
    let imported = recordly::import(&text, options)
        .map_err(|error| format!("{}: {error}", input.display()))?;
    for warning in &imported.warnings {
        eprintln!("Warning: {warning}");
    }

    let mut project = imported.project;
    if let Some(folder) = absolute_folder(&output) {
        project.rebase_media(&folder);
    }
    store::atomic_write(&output, format::to_string(&project).as_bytes(), false)
        .map_err(|error| format!("Couldn't write {}: {error}", output.display()))?;
    println!("Wrote {}", output.display());
    Ok(())
}

fn absolute_folder(output: &Path) -> Option<PathBuf> {
    let folder = output
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    std::fs::canonicalize(folder).ok()
}

/// Reads a project of either kind, along with the folder its relative paths start from.
#[allow(dead_code)]
fn load_project(path: &Path) -> Result<(Project, Option<PathBuf>), String> {
    let text = std::fs::read_to_string(path)
        .map_err(|error| format!("Couldn't read {}: {error}", path.display()))?;
    let is_recordly = path
        .extension()
        .is_some_and(|ext| ext == "recordly" || ext == "openscreen");
    let folder = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .map(Path::to_path_buf);
    if is_recordly {
        let imported = recordly::import(
            &text,
            ImportOptions {
                source_duration: None,
                find_sidecars: true,
            },
        )
        .map_err(|error| format!("{}: {error}", path.display()))?;
        for warning in &imported.warnings {
            eprintln!("Warning: {warning}");
        }
        Ok((imported.project, folder))
    } else {
        let project =
            format::from_str(&text).map_err(|error| format!("{}: {error}", path.display()))?;
        Ok((project, folder))
    }
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
