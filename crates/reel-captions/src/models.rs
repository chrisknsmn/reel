//! Whisper's model files: where they come from, and checking they arrived intact (BUILD_PLAN 9.1).
//! Without a checksum, a cut-off file would look fine until it failed.

use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use sha1_smol::Sha1;

/// A model that can be downloaded, with the checksum whisper.cpp publishes for it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Model {
    pub name: &'static str,
    pub file: &'static str,
    pub sha1: &'static str,
    pub megabytes: u64,
}

/// What Reel offers by default.
pub const SMALL: Model = Model {
    name: "Small",
    file: "ggml-small.bin",
    sha1: "55356645c2b361a969dfd0ef2c5a50d530afd8d5",
    megabytes: 466,
};

/// Much smaller, and good enough for tests.
pub const TINY_EN: Model = Model {
    name: "Tiny (English)",
    file: "ggml-tiny.en.bin",
    sha1: "c78c86eb1a8faa21b369bcd33207cc90d64ae9df",
    megabytes: 75,
};

impl Model {
    #[must_use]
    pub fn url(&self) -> String {
        format!(
            "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/{}",
            self.file
        )
    }

    /// Where it goes in a folder.
    #[must_use]
    pub fn path_in(&self, folder: &Path) -> PathBuf {
        folder.join(self.file)
    }
}

#[derive(Debug)]
pub enum ModelError {
    Network(String),
    Disk(std::io::Error),
    /// What arrived isn't what was expected, so it's been thrown away.
    Corrupt {
        expected: String,
        found: String,
    },
    Cancelled,
}

impl std::fmt::Display for ModelError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Network(reason) => write!(f, "The model couldn't be downloaded: {reason}"),
            Self::Disk(error) => write!(f, "The model couldn't be saved: {error}"),
            Self::Corrupt { .. } => f.write_str(
                "The downloaded model was damaged, so it's been removed. Please try again.",
            ),
            Self::Cancelled => f.write_str("The download was cancelled."),
        }
    }
}

impl std::error::Error for ModelError {}

impl From<std::io::Error> for ModelError {
    fn from(error: std::io::Error) -> Self {
        Self::Disk(error)
    }
}

/// A file's SHA-1, in hex.
///
/// # Errors
/// Returns an error if the file can't be read.
pub fn sha1_of(path: &Path) -> std::io::Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha1::new();
    let mut buffer = vec![0_u8; 1024 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher.digest().to_string())
}

/// Saves a stream to a file through a temporary name, checks it, and only then gives it its real name.
/// So the real name only ever holds a complete, correct model.
///
/// # Errors
/// Returns an error if writing fails, it's cancelled, or the checksum doesn't match. No partial file
/// is left behind.
pub fn save_checked(
    mut source: impl Read,
    total: Option<u64>,
    destination: &Path,
    expected_sha1: &str,
    mut progress: impl FnMut(u64, Option<u64>) -> bool,
) -> Result<(), ModelError> {
    if let Some(folder) = destination.parent() {
        std::fs::create_dir_all(folder)?;
    }
    let mut partial = destination.as_os_str().to_os_string();
    partial.push(".download");
    let partial = PathBuf::from(partial);
    let result = (|| {
        let mut file = File::create(&partial)?;
        let mut hasher = Sha1::new();
        let mut buffer = vec![0_u8; 256 * 1024];
        let mut received = 0_u64;
        loop {
            let read = source
                .read(&mut buffer)
                .map_err(|error| ModelError::Network(error.to_string()))?;
            if read == 0 {
                break;
            }
            file.write_all(&buffer[..read])?;
            hasher.update(&buffer[..read]);
            received += read as u64;
            if !progress(received, total) {
                return Err(ModelError::Cancelled);
            }
        }
        file.flush()?;
        let found = hasher.digest().to_string();
        if found != expected_sha1 {
            return Err(ModelError::Corrupt {
                expected: expected_sha1.to_string(),
                found,
            });
        }
        Ok(())
    })();
    match result {
        Ok(()) => {
            std::fs::rename(&partial, destination)?;
            Ok(())
        }
        Err(error) => {
            let _ = std::fs::remove_file(&partial);
            Err(error)
        }
    }
}

/// Downloads a model. Returning `false` from `progress` cancels.
///
/// # Errors
/// Returns why it didn't work. Nothing partial is left behind.
pub fn download(
    model: &Model,
    destination: &Path,
    progress: impl FnMut(u64, Option<u64>) -> bool,
) -> Result<(), ModelError> {
    let response = ureq::get(&model.url())
        .call()
        .map_err(|error| ModelError::Network(error.to_string()))?;
    let total = response
        .headers()
        .get("content-length")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse().ok());
    let reader = response.into_body().into_reader();
    save_checked(reader, total, destination, model.sha1, progress)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The SHA-1 of "hello".
    const HELLO: &str = "aaf4c61ddcc5e8a2dabede0f3b482cd9aea9434d";

    #[test]
    fn a_correct_file_gets_its_real_name() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("sub/model.bin");
        save_checked(&b"hello"[..], Some(5), &path, HELLO, |_, _| true).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"hello");
        assert_eq!(sha1_of(&path).unwrap(), HELLO);
        assert!(!folder.path().join("sub/model.bin.download").exists());
    }

    #[test]
    fn a_damaged_file_is_thrown_away() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("model.bin");
        let result = save_checked(&b"hellx"[..], None, &path, HELLO, |_, _| true);
        assert!(matches!(result, Err(ModelError::Corrupt { .. })));
        assert!(!path.exists());
        assert!(!folder.path().join("model.bin.download").exists());
    }

    #[test]
    fn cancelling_leaves_nothing() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("model.bin");
        let result = save_checked(&b"hello"[..], None, &path, HELLO, |_, _| false);
        assert!(matches!(result, Err(ModelError::Cancelled)));
        assert!(!path.exists());
        assert!(!folder.path().join("model.bin.download").exists());
    }

    #[test]
    fn progress_counts_up_to_the_total() {
        let folder = tempfile::tempdir().unwrap();
        let mut last = 0;
        save_checked(
            &b"hello"[..],
            Some(5),
            &folder.path().join("m"),
            HELLO,
            |got, total| {
                assert_eq!(total, Some(5));
                last = got;
                true
            },
        )
        .unwrap();
        assert_eq!(last, 5);
    }

    #[test]
    fn a_failing_stream_leaves_nothing() {
        struct Broken;
        impl Read for Broken {
            fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
                Err(std::io::Error::other("connection lost"))
            }
        }
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("model.bin");
        let result = save_checked(Broken, None, &path, HELLO, |_, _| true);
        assert!(matches!(result, Err(ModelError::Network(_))));
        assert!(!folder.path().join("model.bin.download").exists());
    }

    #[test]
    fn urls_point_at_the_right_files() {
        assert!(SMALL.url().ends_with("/ggml-small.bin"));
        assert!(TINY_EN.url().ends_with("/ggml-tiny.en.bin"));
    }
}
