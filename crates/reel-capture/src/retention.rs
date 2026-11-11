//! Deleting old recordings: any older than 14 days, or beyond the newest 20. It's off unless the user
//! turns it on (D7).

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use crate::config::OutputPaths;

pub const KEEP_NEWEST: usize = 20;
pub const KEEP_FOR: Duration = Duration::from_secs(14 * 24 * 60 * 60);

/// A recording found on disk.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Recording {
    pub video: PathBuf,
    pub modified: SystemTime,
}

/// Which recordings the rules would delete. Anything a project uses, or that's in `spared`, stays.
#[must_use]
pub fn to_delete(
    recordings: &[Recording],
    spared: &HashSet<PathBuf>,
    now: SystemTime,
) -> Vec<PathBuf> {
    let mut newest_first: Vec<&Recording> = recordings.iter().collect();
    newest_first.sort_by(|a, b| b.modified.cmp(&a.modified));
    newest_first
        .iter()
        .enumerate()
        .filter(|(index, recording)| {
            let too_old = now
                .duration_since(recording.modified)
                .is_ok_and(|age| age > KEEP_FOR);
            (*index >= KEEP_NEWEST || too_old) && !spared.contains(&recording.video)
        })
        .map(|(_, recording)| recording.video.clone())
        .collect()
}

/// Everything that goes with a recording's video.
#[must_use]
pub fn files_of(video: &Path) -> Vec<PathBuf> {
    let paths = OutputPaths::for_video(video);
    vec![
        paths.video,
        paths.system_audio,
        paths.microphone,
        paths.webcam,
        paths.cursor_log,
        paths.cursor,
        paths.session,
    ]
}

/// The recordings in a folder: any `recording-*.mp4`, leaving out webcam files, which go with theirs.
#[must_use]
pub fn scan(folder: &Path) -> Vec<Recording> {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            let name = path.file_name()?.to_string_lossy().into_owned();
            let is_video = name.starts_with("recording-")
                && name.ends_with(".mp4")
                && !name.ends_with("-webcam.mp4");
            is_video.then_some(())?;
            Some(Recording {
                modified: entry.metadata().ok()?.modified().ok()?,
                video: path,
            })
        })
        .collect()
}

/// Applies the rules to a folder, and returns the videos it removed.
#[must_use]
pub fn prune(folder: &Path, spared: &HashSet<PathBuf>, now: SystemTime) -> Vec<PathBuf> {
    let doomed = to_delete(&scan(folder), spared, now);
    for video in &doomed {
        for file in files_of(video) {
            let _ = std::fs::remove_file(file);
        }
    }
    doomed
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: Duration = Duration::from_secs(24 * 60 * 60);

    fn now() -> SystemTime {
        SystemTime::UNIX_EPOCH + DAY * 1000
    }

    fn recording(name: &str, days_old: u32) -> Recording {
        Recording {
            video: PathBuf::from(name),
            modified: now() - DAY * days_old,
        }
    }

    #[test]
    fn recent_recordings_stay() {
        let list = [recording("a", 1), recording("b", 13)];
        assert!(to_delete(&list, &HashSet::new(), now()).is_empty());
    }

    #[test]
    fn old_recordings_go() {
        let list = [recording("old", 15), recording("new", 1)];
        assert_eq!(
            to_delete(&list, &HashSet::new(), now()),
            [PathBuf::from("old")]
        );
    }

    #[test]
    fn only_the_newest_twenty_stay() {
        let list: Vec<_> = (0..23)
            .map(|index| recording(&format!("r{index:02}"), 0))
            .collect();
        let mut list = list;
        for (index, item) in list.iter_mut().enumerate() {
            item.modified = now() - Duration::from_secs(index as u64);
        }
        let doomed = to_delete(&list, &HashSet::new(), now());
        assert_eq!(
            doomed,
            [
                PathBuf::from("r20"),
                PathBuf::from("r21"),
                PathBuf::from("r22")
            ]
        );
    }

    #[test]
    fn anything_in_use_is_spared_however_old() {
        let list = [recording("used", 400), recording("unused", 400)];
        let spared = HashSet::from([PathBuf::from("used")]);
        assert_eq!(to_delete(&list, &spared, now()), [PathBuf::from("unused")]);
    }

    #[test]
    fn pruning_removes_a_recordings_side_files_and_nothing_else() {
        let folder = tempfile::tempdir().unwrap();
        let old = folder.path().join("recording-1.mp4");
        let other = folder.path().join("my-video.mp4");
        let paths = OutputPaths::for_video(&old);
        for file in [
            &old,
            &paths.system_audio,
            &paths.microphone,
            &paths.webcam,
            &paths.cursor,
            &paths.session,
            &other,
        ] {
            std::fs::write(file, b"x").unwrap();
        }
        // Long enough from now that everything here counts as old.
        let later = SystemTime::now() + DAY * 30;
        let removed = prune(folder.path(), &HashSet::new(), later);
        assert_eq!(removed, std::slice::from_ref(&old));
        assert!(
            !old.exists()
                && !paths.microphone.exists()
                && !paths.webcam.exists()
                && !paths.session.exists()
        );
        assert!(
            other.exists(),
            "a file that isn't one of ours is left alone"
        );
    }

    #[test]
    fn a_webcam_file_isnt_mistaken_for_a_recording() {
        let folder = tempfile::tempdir().unwrap();
        std::fs::write(folder.path().join("recording-1-webcam.mp4"), b"x").unwrap();
        assert!(scan(folder.path()).is_empty());
    }
}
