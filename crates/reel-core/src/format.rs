//! The `.reel` file: versioned JSON with explicit migrations (BUILD_PLAN 2.5).
//!
//! Every older version goes through a numbered migration, and a newer one is refused rather than
//! half-read and then overwritten.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::project::Project;

pub const EXTENSION: &str = "reel";

/// What's in the `format` field, to tell a `.reel` file from any other JSON.
const FORMAT: &str = "reel-project";

pub const CURRENT_VERSION: u64 = 1;

/// One step per version, turning version `n + 1` into version `n + 2`. There are none yet.
const MIGRATIONS: &[Migration] = &[];

type Migration = fn(&mut Value);

#[derive(Debug)]
pub enum LoadError {
    Malformed(String),
    NotAProject,
    /// Written by a newer version of Reel, which this one can't safely read.
    TooNew {
        version: u64,
    },
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Malformed(reason) => write!(f, "This project file is damaged: {reason}"),
            Self::NotAProject => write!(f, "This isn't a Reel project."),
            Self::TooNew { version } => write!(
                f,
                "This project was saved by a newer version of Reel (format {version}). Update Reel to open it."
            ),
        }
    }
}

impl std::error::Error for LoadError {}

#[derive(Serialize)]
struct FileOut<'a> {
    format: &'static str,
    version: u64,
    #[serde(flatten)]
    project: &'a Project,
}

#[derive(Deserialize)]
struct FileIn {
    #[serde(flatten)]
    project: Project,
}

/// Writes a project as pretty-printed JSON.
#[must_use]
pub fn to_string(project: &Project) -> String {
    let file = FileOut {
        format: FORMAT,
        version: CURRENT_VERSION,
        project,
    };
    // Nothing in a project can fail to serialize: it has no maps with non-string keys.
    let mut text = serde_json::to_string_pretty(&file).expect("a project always serializes");
    text.push('\n');
    text
}

/// Reads a project, bringing older versions up to date.
///
/// # Errors
/// Returns why the file can't be read, including if a newer version of Reel wrote it.
pub fn from_str(text: &str) -> Result<Project, LoadError> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut value: Value =
        serde_json::from_str(text).map_err(|error| LoadError::Malformed(error.to_string()))?;
    if value.get("format").and_then(Value::as_str) != Some(FORMAT) {
        return Err(LoadError::NotAProject);
    }
    let version = value
        .get("version")
        .and_then(Value::as_u64)
        .ok_or_else(|| LoadError::Malformed("it has no version number".to_string()))?;
    migrate(&mut value, version, CURRENT_VERSION, MIGRATIONS)?;
    let file: FileIn =
        serde_json::from_value(value).map_err(|error| LoadError::Malformed(error.to_string()))?;
    let mut project = file.project;
    project.normalize();
    Ok(project)
}

fn migrate(
    value: &mut Value,
    from: u64,
    current: u64,
    steps: &[Migration],
) -> Result<(), LoadError> {
    if from == 0 {
        return Err(LoadError::Malformed("its version number is 0".to_string()));
    }
    if from > current {
        return Err(LoadError::TooNew { version: from });
    }
    for step in &steps[(from - 1) as usize..(current - 1) as usize] {
        step(value);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::time::Micros;
    use crate::timeline::Timeline;

    fn sample() -> Project {
        Project {
            id: "abc".to_string(),
            timeline: Timeline::for_recording(Micros::from_secs(30)),
            ..Project::default()
        }
    }

    #[test]
    fn a_project_round_trips_exactly() {
        let project = sample();
        let text = to_string(&project);
        let loaded = from_str(&text).unwrap();
        assert_eq!(loaded, project);
        assert_eq!(to_string(&loaded), text, "saving again changes nothing");
    }

    #[test]
    fn a_byte_order_mark_is_tolerated() {
        let text = format!("\u{feff}{}", to_string(&sample()));
        assert!(from_str(&text).is_ok());
    }

    #[test]
    fn other_json_is_not_a_project() {
        assert!(matches!(
            from_str("{\"version\": 1}"),
            Err(LoadError::NotAProject)
        ));
    }

    #[test]
    fn broken_json_says_so() {
        assert!(matches!(from_str("{"), Err(LoadError::Malformed(_))));
    }

    #[test]
    fn a_newer_file_is_refused_rather_than_half_read() {
        let text = to_string(&sample()).replace("\"version\": 1", "\"version\": 999");
        assert!(matches!(
            from_str(&text),
            Err(LoadError::TooNew { version: 999 })
        ));
    }

    #[test]
    fn missing_sections_take_their_defaults() {
        let project = from_str(r#"{ "format": "reel-project", "version": 1, "id": "x" }"#).unwrap();
        assert_eq!(project.id, "x");
        assert_eq!(project.cursor.size, 2.5);
    }

    #[test]
    fn out_of_range_values_are_brought_back_in_on_load() {
        let text =
            to_string(&sample()).replace("\"background_blur\": 0.0", "\"background_blur\": 99.0");
        assert_eq!(from_str(&text).unwrap().look.background_blur, 8.0);
    }

    #[test]
    fn migrations_run_in_order_from_the_files_version() {
        fn one(value: &mut Value) {
            value["steps"] = Value::from("one");
        }
        fn two(value: &mut Value) {
            let so_far = value["steps"].as_str().unwrap_or("none").to_string();
            value["steps"] = Value::from(format!("{so_far},two"));
        }
        let steps: &[Migration] = &[one, two];

        let mut value = serde_json::json!({});
        migrate(&mut value, 1, 3, steps).unwrap();
        assert_eq!(value["steps"], "one,two");

        let mut value = serde_json::json!({});
        migrate(&mut value, 2, 3, steps).unwrap();
        assert_eq!(value["steps"], "none,two");

        let mut value = serde_json::json!({});
        migrate(&mut value, 3, 3, steps).unwrap();
        assert!(value.get("steps").is_none());

        assert!(matches!(
            migrate(&mut value, 4, 3, steps),
            Err(LoadError::TooNew { .. })
        ));
    }
}
