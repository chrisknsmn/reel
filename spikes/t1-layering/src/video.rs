//! Decodes a video file to RGBA frames by piping them out of an `ffmpeg` process.
//!
//! This is deliberately the dumbest decoder that works. T1 is about layering, and choosing the real
//! decoder is T2's job (D5).

use std::io::Read;
use std::process::{Command, Stdio};
use std::sync::mpsc::{Receiver, Sender, SyncSender, channel, sync_channel};

pub const WIDTH: u32 = 1920;
pub const HEIGHT: u32 = 1080;
pub const FPS: f64 = 60.0;

pub struct Frame {
    pub index: u64,
    pub rgba: Vec<u8>,
}

pub struct Decoder {
    pub frames: Receiver<Frame>,
    pub recycle: Sender<Vec<u8>>,
}

pub fn start(ffmpeg: &str, path: &str) -> Decoder {
    let (frame_tx, frames): (SyncSender<Frame>, _) = sync_channel(4);
    let (recycle, recycled) = channel::<Vec<u8>>();

    let mut child = Command::new(ffmpeg)
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-stream_loop",
            "-1",
            "-i",
            path,
        ])
        .args(["-vf", &format!("scale={WIDTH}:{HEIGHT},fps={FPS}")])
        .args(["-f", "rawvideo", "-pix_fmt", "rgba", "-"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .spawn()
        .expect("couldn't start ffmpeg; set T1_FFMPEG to its path");
    let mut stdout = child
        .stdout
        .take()
        .expect("ffmpeg stdout was requested as a pipe");

    std::thread::Builder::new()
        .name("decode".into())
        .spawn(move || {
            let size = (WIDTH * HEIGHT * 4) as usize;
            let mut index = 0;
            loop {
                let mut rgba = recycled.try_recv().unwrap_or_else(|_| vec![0; size]);
                if stdout.read_exact(&mut rgba).is_err() {
                    eprintln!("ffmpeg stopped producing frames after {index}");
                    break;
                }
                if frame_tx.send(Frame { index, rgba }).is_err() {
                    break;
                }
                index += 1;
            }
            let _ = child.kill();
            let _ = child.wait();
        })
        .expect("couldn't start the decode thread");

    Decoder { frames, recycle }
}
