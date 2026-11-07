//! A pretend capture source, for testing everything downstream of capture with no permissions. It
//! flashes the picture and beeps every second, all on one clock, so a flash and a beep that come apart
//! in the finished files show exactly how far the recording has drifted.

use std::ptr::{NonNull, null};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::Duration;

use objc2_core_foundation::CFRetained;
use objc2_core_video::{
    CVPixelBuffer, CVPixelBufferCreate, CVPixelBufferGetBaseAddress, CVPixelBufferGetBytesPerRow,
    CVPixelBufferGetHeight, CVPixelBufferLockBaseAddress, CVPixelBufferLockFlags,
    CVPixelBufferUnlockBaseAddress, kCVPixelFormatType_32BGRA,
};
use reel_core::time::Micros;
use reel_media::macos::record::{host_now, pcm_sample};

use super::recorder::{Recorder, Track};

/// A beep's length, and how long the flash stays lit.
const TICK: Micros = Micros(100_000);
const SECOND: i64 = 1_000_000;
/// How much sound goes in each buffer: 10 ms, like a real source.
const CHUNK_FRAMES: usize = 480;

struct Buffer(CFRetained<CVPixelBuffer>);

impl Buffer {
    fn get(&self) -> &CVPixelBuffer {
        &self.0
    }
}

// SAFETY: these buffers are filled before the threads start, and only read after that.
unsafe impl Send for Buffer {}

fn solid_frame(width: u32, height: u32, level: u8) -> Result<Buffer, String> {
    let mut raw: *mut CVPixelBuffer = std::ptr::null_mut();
    // SAFETY: the out pointer is valid and is checked. The base address is only written while locked,
    // and the writes stay inside the height and stride the buffer reports.
    unsafe {
        let status = CVPixelBufferCreate(
            None,
            width as usize,
            height as usize,
            kCVPixelFormatType_32BGRA,
            None,
            NonNull::from(&mut raw),
        );
        let buffer = NonNull::new(raw)
            .filter(|_| status == 0)
            .ok_or_else(|| format!("Couldn't make a test frame (error {status})."))?;
        let buffer: CFRetained<CVPixelBuffer> = CFRetained::from_raw(buffer);
        CVPixelBufferLockBaseAddress(&buffer, CVPixelBufferLockFlags::empty());
        let base = CVPixelBufferGetBaseAddress(&buffer).cast::<u8>();
        let (stride, rows) = (
            CVPixelBufferGetBytesPerRow(&buffer),
            CVPixelBufferGetHeight(&buffer),
        );
        if base != null::<u8>().cast_mut() {
            for y in 0..rows {
                let row = std::slice::from_raw_parts_mut(base.add(y * stride), width as usize * 4);
                for pixel in row.chunks_exact_mut(4) {
                    pixel.copy_from_slice(&[level, level, level, 255]);
                }
            }
        }
        CVPixelBufferUnlockBaseAddress(&buffer, CVPixelBufferLockFlags::empty());
        Ok(Buffer(buffer))
    }
}

/// Generates the pattern until told to stop.
pub struct Synthetic {
    stop: Arc<AtomicBool>,
    threads: Vec<JoinHandle<()>>,
}

impl Synthetic {
    /// Starts the threads. Sound goes to whichever tracks the recorder has open.
    ///
    /// # Errors
    /// Returns an error if the test frames can't be made.
    pub fn start(
        recorder: Arc<Recorder>,
        size: (u32, u32),
        fps: u32,
        mic_delay: Micros,
    ) -> Result<Self, String> {
        let stop = Arc::new(AtomicBool::new(false));
        let dark = solid_frame(size.0, size.1, 20)?;
        let bright = solid_frame(size.0, size.1, 235)?;
        let origin = host_now();
        let period = Duration::from_secs_f64(1.0 / f64::from(fps.max(1)));

        let mut threads = Vec::new();
        {
            let (stop, recorder) = (stop.clone(), recorder.clone());
            threads.push(std::thread::spawn(move || {
                let mut next = host_now();
                while !stop.load(Ordering::Relaxed) {
                    let now = host_now();
                    let lit = (now.0 - origin.0).rem_euclid(SECOND) < TICK.0;
                    recorder.on_frame(if lit { bright.get() } else { dark.get() }, now);
                    next = Micros(next.0 + period.as_micros() as i64);
                    let wait = next.0 - host_now().0;
                    if wait > 0 {
                        std::thread::sleep(Duration::from_micros(wait as u64));
                    }
                }
            }));
        }
        for (track, delay) in [
            (Track::System, Micros::ZERO),
            (Track::Microphone, mic_delay),
        ] {
            let (stop, recorder) = (stop.clone(), recorder.clone());
            threads.push(std::thread::spawn(move || {
                let chunk = Micros((CHUNK_FRAMES as f64 / 48_000.0 * 1_000_000.0) as i64);
                let start = host_now();
                let mut at = start;
                let mut samples = vec![0_i16; CHUNK_FRAMES * 2];
                while !stop.load(Ordering::Relaxed) {
                    // Real sources deliver a little after the sound happened, which this copies.
                    let due = at.0 + chunk.0;
                    let wait = due - host_now().0;
                    if wait > 0 {
                        std::thread::sleep(Duration::from_micros(wait as u64));
                    }
                    if at.0 - start.0 >= delay.0 {
                        fill(&mut samples, at.0 - origin.0);
                        if let Ok(sample) = pcm_sample(&samples, at) {
                            recorder.on_sound(track, &sample, at);
                        }
                    }
                    at = Micros(at.0 + chunk.0);
                }
            }));
        }
        Ok(Self { stop, threads })
    }

    pub fn stop(self) {
        self.stop.store(true, Ordering::Relaxed);
        for thread in self.threads {
            let _ = thread.join();
        }
    }
}

/// A 1 kHz beep for the first part of each second, on the same clock as the flash.
fn fill(samples: &mut [i16], start_us: i64) {
    for (index, frame) in samples.chunks_exact_mut(2).enumerate() {
        let time = start_us + (index as f64 / 48_000.0 * 1_000_000.0) as i64;
        let beeping = time.rem_euclid(SECOND) < TICK.0;
        let value = if beeping {
            ((time as f64 / 1_000_000.0 * 1000.0 * std::f64::consts::TAU).sin() * 20_000.0) as i16
        } else {
            0
        };
        frame[0] = value;
        frame[1] = value;
    }
}
