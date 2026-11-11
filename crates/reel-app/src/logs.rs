//! Diagnostics: what happened during recordings and exports, kept on this machine, to help when
//! something goes wrong. Nothing here is ever sent anywhere.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::recording::support_folder;

/// How many days of logs are kept.
const KEEP_FOR: Duration = Duration::from_secs(14 * 24 * 60 * 60);
const SECONDS_IN_A_DAY: u64 = 24 * 60 * 60;

fn folder() -> PathBuf {
    support_folder().join("logs")
}

/// Today's file for a kind of log. It's numbered by the day, so that old ones are easy to spot.
fn file_for(folder: &Path, kind: &str, now: SystemTime) -> PathBuf {
    let day = now
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_secs() / SECONDS_IN_A_DAY);
    folder.join(format!("{kind}-{day}.log"))
}

fn append_to(folder: &Path, kind: &str, message: &str, now: SystemTime) -> std::io::Result<()> {
    std::fs::create_dir_all(folder)?;
    let seconds = now
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_secs());
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(file_for(folder, kind, now))?;
    writeln!(file, "{seconds} {message}")
}

/// Deletes logs older than the limit, and returns how many went.
fn prune_in(folder: &Path, now: SystemTime) -> usize {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return 0;
    };
    let mut removed = 0;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_none_or(|extension| extension != "log") {
            continue;
        }
        let old = entry
            .metadata()
            .and_then(|info| info.modified())
            .ok()
            .and_then(|modified| now.duration_since(modified).ok())
            .is_some_and(|age| age > KEEP_FOR);
        if old && std::fs::remove_file(path).is_ok() {
            removed += 1;
        }
    }
    removed
}

/// Adds a line to a log. A log that can't be written doesn't get in the way of anything.
pub fn write(kind: &str, message: &str) {
    let _ = append_to(&folder(), kind, message, SystemTime::now());
}

/// Clears out old logs. It runs once at startup.
pub fn prune() {
    let _ = prune_in(&folder(), SystemTime::now());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_go_in_a_file_for_the_day() {
        let folder = tempfile::tempdir().unwrap();
        let now = SystemTime::now();
        append_to(folder.path(), "export", "first", now).unwrap();
        append_to(folder.path(), "export", "second", now).unwrap();
        append_to(folder.path(), "recording", "other", now).unwrap();
        let text = std::fs::read_to_string(file_for(folder.path(), "export", now)).unwrap();
        assert!(
            text.contains(" first\n") && text.contains(" second\n"),
            "{text}"
        );
        assert!(!text.contains("other"));
    }

    #[test]
    fn a_new_day_is_a_new_file() {
        let folder = Path::new("/logs");
        let today = SystemTime::now();
        assert_ne!(
            file_for(folder, "export", today),
            file_for(
                folder,
                "export",
                today + Duration::from_secs(SECONDS_IN_A_DAY)
            )
        );
    }

    #[test]
    fn only_old_logs_are_pruned() {
        let folder = tempfile::tempdir().unwrap();
        let log = folder.path().join("export-1.log");
        let other = folder.path().join("keep.txt");
        std::fs::write(&log, "x").unwrap();
        std::fs::write(&other, "x").unwrap();
        assert_eq!(prune_in(folder.path(), SystemTime::now()), 0, "it's new");
        assert!(log.exists());
        assert_eq!(prune_in(folder.path(), SystemTime::now() + KEEP_FOR * 2), 1);
        assert!(!log.exists());
        assert!(other.exists(), "only logs are touched");
    }
}
