//! Saving files so a crash can't lose them (§45, BUILD_PLAN 2.6).

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

fn temp_path(folder: &Path) -> PathBuf {
    let unique = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    folder.join(format!(".reel-{}-{unique}.tmp", std::process::id()))
}

/// Where a file's backup goes: `a.reel` becomes `a.reel.bak`.
#[must_use]
pub fn backup_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(".bak");
    PathBuf::from(name)
}

/// Writes a file in a way that's always either the old contents or the new, never half of either.
///
/// The bytes go to a temporary file in the same folder and are flushed to disk, then the temporary
/// file is renamed over the target. With `keep_backup`, the previous file becomes `<name>.bak` first.
///
/// # Errors
/// Returns the underlying error. The target is untouched if anything fails before the final rename.
pub fn atomic_write(path: &Path, contents: &[u8], keep_backup: bool) -> io::Result<()> {
    let folder = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(folder)?;

    let temp = temp_path(folder);
    let result = write_new(&temp, contents, path).and_then(|()| {
        if keep_backup && path.exists() {
            copy_to_backup(path, folder)?;
        }
        fs::rename(&temp, path)?;
        sync_folder(folder)
    });
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

/// Creates the temporary file, refusing one that's already there, with the old file's permissions.
fn write_new(temp: &Path, contents: &[u8], target: &Path) -> io::Result<()> {
    let mut file = OpenOptions::new().write(true).create_new(true).open(temp)?;
    file.write_all(contents)?;
    if let Ok(metadata) = fs::metadata(target) {
        file.set_permissions(metadata.permissions())?;
    }
    file.sync_all()
}

/// Copies through a temporary file too, so a crash can't leave a half-written backup.
fn copy_to_backup(path: &Path, folder: &Path) -> io::Result<()> {
    let temp = temp_path(folder);
    let result = fs::copy(path, &temp).and_then(|_| {
        File::open(&temp)?.sync_all()?;
        fs::rename(&temp, backup_path(path))
    });
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

#[cfg(unix)]
fn sync_folder(folder: &Path) -> io::Result<()> {
    File::open(folder)?.sync_all()
}

#[cfg(not(unix))]
fn sync_folder(_folder: &Path) -> io::Result<()> {
    Ok(())
}

/// Makes a name safe for a file on every OS.
#[must_use]
pub fn safe_file_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .filter(|c| {
            !matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') && !c.is_control()
        })
        .collect();
    let trimmed = cleaned.trim_end_matches(['.', ' ']).trim_start();
    if trimmed.is_empty() {
        "Untitled".to_string()
    } else {
        trimmed.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writing_creates_the_file_and_any_folders() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("deep/er/a.reel");
        atomic_write(&path, b"one", true).unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"one");
    }

    #[test]
    fn the_previous_save_becomes_the_backup() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.reel");
        atomic_write(&path, b"one", true).unwrap();
        assert!(
            !backup_path(&path).exists(),
            "the first save has nothing to back up"
        );
        atomic_write(&path, b"two", true).unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"two");
        assert_eq!(fs::read(backup_path(&path)).unwrap(), b"one");
    }

    #[test]
    fn no_temporary_files_are_left_behind() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.reel");
        atomic_write(&path, b"one", true).unwrap();
        atomic_write(&path, b"two", true).unwrap();
        let mut names: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(names, ["a.reel", "a.reel.bak"]);
    }

    #[cfg(unix)]
    #[test]
    fn saving_keeps_the_files_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.reel");
        atomic_write(&path, b"one", false).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        atomic_write(&path, b"two", false).unwrap();
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_failed_save_leaves_the_old_file_alone() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.reel");
        atomic_write(&path, b"one", false).unwrap();
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o500)).unwrap();
        let result = atomic_write(&path, b"two", false);
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o700)).unwrap();
        assert!(result.is_err());
        assert_eq!(fs::read(&path).unwrap(), b"one");
    }

    #[test]
    fn names_are_cleaned_for_every_os() {
        assert_eq!(safe_file_name("a/b:c*d?.reel"), "abcd.reel");
        assert_eq!(safe_file_name("trailing dots... "), "trailing dots");
        assert_eq!(safe_file_name("???"), "Untitled");
    }
}
