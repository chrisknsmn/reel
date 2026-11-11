//! Checks that there's room to record, so a recording can't fill the disk.

use std::path::Path;

/// Below this, a recording won't start.
pub const MIN_TO_START: u64 = 500 * 1024 * 1024;
/// Below this, a recording finishes itself, while there's still room to write the index.
pub const MIN_TO_CONTINUE: u64 = 150 * 1024 * 1024;

/// How much room is left on the disk holding this path, or `None` if that can't be found out.
#[must_use]
pub fn free_bytes(path: &Path) -> Option<u64> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        // Whatever part of the path already exists: the file itself won't yet.
        let existing = path.ancestors().find(|part| part.exists())?;
        let text = std::ffi::CString::new(existing.as_os_str().as_bytes()).ok()?;
        let mut info = std::mem::MaybeUninit::<libc::statvfs>::zeroed();
        // SAFETY: the path is a valid C string and the out pointer is valid. It's only read on success.
        unsafe {
            if libc::statvfs(text.as_ptr(), info.as_mut_ptr()) != 0 {
                return None;
            }
            let info = info.assume_init();
            #[allow(clippy::unnecessary_cast)]
            Some(info.f_bavail as u64 * info.f_frsize as u64)
        }
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        None
    }
}

/// Whether a recording may start with this much room.
#[must_use]
pub fn may_start(free: Option<u64>) -> bool {
    // If it can't be found out, don't stand in the way.
    free.is_none_or(|free| free >= MIN_TO_START)
}

/// How many seconds of recording fit, for telling the user when they're getting close.
#[must_use]
pub fn seconds_left(free: u64, bits_per_second: u64) -> u64 {
    free.saturating_sub(MIN_TO_CONTINUE) * 8 / bits_per_second.max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn this_disk_reports_some_room() {
        let folder = tempfile::tempdir().unwrap();
        assert!(free_bytes(folder.path()).unwrap() > 0);
        assert!(
            free_bytes(&folder.path().join("not/made/yet.mp4")).unwrap() > 0,
            "a path that doesn't exist yet works too"
        );
    }

    #[test]
    fn starting_needs_room_but_doesnt_insist_on_knowing() {
        assert!(may_start(Some(MIN_TO_START)));
        assert!(!may_start(Some(MIN_TO_START - 1)));
        assert!(may_start(None));
    }

    #[test]
    fn time_left_allows_for_finishing_up() {
        assert_eq!(seconds_left(MIN_TO_CONTINUE, 8_000_000), 0);
        assert_eq!(seconds_left(MIN_TO_CONTINUE + 10_000_000, 8_000_000), 10);
        assert_eq!(seconds_left(0, 8_000_000), 0);
    }
}
