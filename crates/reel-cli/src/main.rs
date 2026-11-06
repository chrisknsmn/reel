//! Headless render and test harness.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use reel_core::format;
use reel_core::recordly::{self, ImportOptions};
use reel_core::store;
use reel_core::time::Micros;

const USAGE: &str = "\
Usage:
  reel-cli convert <project.recordly> [output.reel] [--force] [--source-duration-ms <ms>]

Converts a Recordly project into a Reel project, without changing the original.

  --force                  Overwrite the output if it exists.
  --source-duration-ms     How long the recording is. Only needed for old projects that have trims but no clips.";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("convert") => convert(&args[1..]),
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
