//! A live level from a microphone, for the Audio popover (§27). It runs in the recorder's process, so
//! it's the same code path and the same permission as recording.

use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

pub struct Meter {
    child: Child,
    level: Arc<AtomicU64>,
    seen: u64,
}

impl Meter {
    /// Starts reading a device's level.
    ///
    /// # Errors
    /// Returns an error if the process can't be started.
    pub fn start(device: &str) -> std::io::Result<Self> {
        let exe = std::env::current_exe()?;
        let recorder = exe.parent().map_or_else(
            || "reel-recorder".into(),
            |folder| folder.join("reel-recorder"),
        );
        let mut child = Command::new(recorder)
            .args(["meter", device])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let level = Arc::new(AtomicU64::new(u64::MAX));
        if let Some(stdout) = child.stdout.take() {
            let shared = level.clone();
            std::thread::spawn(move || {
                for line in BufReader::new(stdout).lines() {
                    let Ok(line) = line else { break };
                    if let Ok(value) = line.trim().parse::<f64>() {
                        shared.store(value.to_bits(), Ordering::Relaxed);
                    }
                }
            });
        }
        Ok(Self {
            child,
            level,
            seen: u64::MAX,
        })
    }

    /// The newest level from 0 to 100, if there's been a new one.
    pub fn latest(&mut self) -> Option<f64> {
        let bits = self.level.load(Ordering::Relaxed);
        if bits == self.seen || bits == u64::MAX {
            return None;
        }
        self.seen = bits;
        Some(f64::from_bits(bits))
    }
}

impl Drop for Meter {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
