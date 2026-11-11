//! Cursor data, streamed to disk as it's recorded. There's no limit on how long, and a crash loses at
//! most the last moment of it.

use std::fs::File;
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};

use reel_core::store::atomic_write;
use reel_core::time::Micros;
use reel_motion::cursor::{CursorShape, Interaction};
use serde::{Deserialize, Serialize};

/// Two clicks this close together in time and place are a double click.
const DOUBLE_CLICK: Micros = Micros(350_000);
const DOUBLE_CLICK_DISTANCE: f64 = 0.04;
/// How many samples may be unsaved. At 30 a second, this is about a third of a second.
const FLUSH_EVERY: u32 = 10;

/// One line of the log, and one sample of the finished file.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Sample {
    #[serde(rename = "timeMs")]
    pub time_ms: f64,
    pub cx: f64,
    pub cy: f64,
    #[serde(rename = "interactionType")]
    pub interaction: Interaction,
    #[serde(rename = "cursorType")]
    pub shape: CursorShape,
}

/// An area of the screen, in global points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bounds {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Bounds {
    /// A global position as fractions of this area. It's outside 0 to 1 when the cursor is outside.
    #[must_use]
    pub fn normalise(&self, x: f64, y: f64) -> (f64, f64) {
        (
            (x - self.x) / self.width.max(1.0),
            (y - self.y) / self.height.max(1.0),
        )
    }
}

/// A mouse button event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ButtonEvent {
    Down(Button),
    Up,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Button {
    Left,
    Right,
    Middle,
}

/// Works out what kind of click a press was.
#[derive(Clone, Copy, Debug, Default)]
pub struct ClickTracker {
    last: Option<(Micros, f64, f64)>,
}

impl ClickTracker {
    pub fn classify(&mut self, event: ButtonEvent, time: Micros, x: f64, y: f64) -> Interaction {
        match event {
            ButtonEvent::Up => Interaction::Mouseup,
            ButtonEvent::Down(Button::Right) => Interaction::RightClick,
            ButtonEvent::Down(Button::Middle) => Interaction::MiddleClick,
            ButtonEvent::Down(Button::Left) => {
                let double = self.last.is_some_and(|(at, last_x, last_y)| {
                    time - at <= DOUBLE_CLICK
                        && (x - last_x).hypot(y - last_y) <= DOUBLE_CLICK_DISTANCE
                });
                // A third quick click starts counting afresh, rather than being another double.
                self.last = if double { None } else { Some((time, x, y)) };
                if double {
                    Interaction::DoubleClick
                } else {
                    Interaction::Click
                }
            }
        }
    }
}

/// Appends samples to a log file.
pub struct CursorLog {
    file: BufWriter<File>,
    unsaved: u32,
    count: u64,
}

impl CursorLog {
    /// Starts a new log.
    ///
    /// # Errors
    /// Returns an error if the file can't be made.
    pub fn create(path: &Path) -> std::io::Result<Self> {
        Ok(Self {
            file: BufWriter::new(File::create(path)?),
            unsaved: 0,
            count: 0,
        })
    }

    /// Adds a sample.
    ///
    /// # Errors
    /// Returns an error if writing fails. A full disk is the likely reason.
    pub fn push(&mut self, sample: &Sample) -> std::io::Result<()> {
        serde_json::to_writer(&mut self.file, sample)?;
        self.file.write_all(b"\n")?;
        self.count += 1;
        self.unsaved += 1;
        if self.unsaved >= FLUSH_EVERY {
            self.flush()?;
        }
        Ok(())
    }

    /// Makes sure everything so far is in the file.
    ///
    /// # Errors
    /// Returns an error if writing fails.
    pub fn flush(&mut self) -> std::io::Result<()> {
        self.unsaved = 0;
        self.file.flush()
    }

    #[must_use]
    pub fn count(&self) -> u64 {
        self.count
    }
}

#[derive(Serialize)]
struct Finished<'a> {
    version: u32,
    provider: &'a str,
    samples: Vec<Sample>,
}

/// Turns a log into the finished cursor file, then removes the log. A process that died can leave half
/// a line at the end, which is skipped. Returns how many samples it holds.
///
/// # Errors
/// Returns an error if the log can't be read or the file can't be written.
pub fn finalise(log: &Path, output: &Path) -> std::io::Result<usize> {
    let reader = BufReader::new(File::open(log)?);
    let mut samples = Vec::new();
    for line in reader.lines() {
        let Ok(line) = line else { break };
        if let Ok(sample) = serde_json::from_str::<Sample>(&line) {
            samples.push(sample);
        }
    }
    samples.sort_by(|a, b| a.time_ms.total_cmp(&b.time_ms));
    let count = samples.len();
    let text = serde_json::to_string(&Finished {
        version: 2,
        provider: "reel",
        samples,
    })?;
    atomic_write(output, text.as_bytes(), false)?;
    std::fs::remove_file(log)?;
    Ok(count)
}

/// Where a video's cursor files go.
#[must_use]
pub fn log_path_for(video: &Path) -> PathBuf {
    let mut name = video.as_os_str().to_os_string();
    name.push(".cursor.jsonl");
    PathBuf::from(name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use reel_motion::cursor::CursorTrack;

    fn sample(ms: f64) -> Sample {
        Sample {
            time_ms: ms,
            cx: 0.25,
            cy: 0.75,
            interaction: Interaction::Move,
            shape: CursorShape::Arrow,
        }
    }

    #[test]
    fn positions_are_fractions_of_the_source() {
        let bounds = Bounds {
            x: 100.0,
            y: 50.0,
            width: 200.0,
            height: 100.0,
        };
        assert_eq!(bounds.normalise(200.0, 100.0), (0.5, 0.5));
        assert_eq!(bounds.normalise(100.0, 50.0), (0.0, 0.0));
        assert_eq!(
            bounds.normalise(400.0, 0.0),
            (1.5, -0.5),
            "outside the source stays outside"
        );
    }

    #[test]
    fn a_quick_second_click_in_the_same_place_is_a_double_click() {
        let mut clicks = ClickTracker::default();
        let press = ButtonEvent::Down(Button::Left);
        assert_eq!(
            clicks.classify(press, Micros::ZERO, 0.5, 0.5),
            Interaction::Click
        );
        assert_eq!(
            clicks.classify(press, Micros::from_ms(300.0), 0.51, 0.5),
            Interaction::DoubleClick
        );
        assert_eq!(
            clicks.classify(press, Micros::from_ms(500.0), 0.51, 0.5),
            Interaction::Click,
            "the third starts over"
        );
    }

    #[test]
    fn a_slow_or_distant_second_click_isnt() {
        let press = ButtonEvent::Down(Button::Left);
        let mut clicks = ClickTracker::default();
        clicks.classify(press, Micros::ZERO, 0.5, 0.5);
        assert_eq!(
            clicks.classify(press, Micros::from_ms(400.0), 0.5, 0.5),
            Interaction::Click
        );
        let mut clicks = ClickTracker::default();
        clicks.classify(press, Micros::ZERO, 0.5, 0.5);
        assert_eq!(
            clicks.classify(press, Micros::from_ms(100.0), 0.6, 0.5),
            Interaction::Click
        );
    }

    #[test]
    fn other_buttons_have_their_own_kinds() {
        let mut clicks = ClickTracker::default();
        assert_eq!(
            clicks.classify(ButtonEvent::Down(Button::Right), Micros::ZERO, 0.0, 0.0),
            Interaction::RightClick
        );
        assert_eq!(
            clicks.classify(ButtonEvent::Down(Button::Middle), Micros::ZERO, 0.0, 0.0),
            Interaction::MiddleClick
        );
        assert_eq!(
            clicks.classify(ButtonEvent::Up, Micros::ZERO, 0.0, 0.0),
            Interaction::Mouseup
        );
    }

    #[test]
    fn a_finished_log_is_a_file_the_editor_can_read() {
        let folder = tempfile::tempdir().unwrap();
        let (log_path, output) = (folder.path().join("a.jsonl"), folder.path().join("a.json"));
        let mut log = CursorLog::create(&log_path).unwrap();
        log.push(&sample(0.0)).unwrap();
        log.push(&Sample {
            interaction: Interaction::Click,
            shape: CursorShape::Pointer,
            ..sample(33.0)
        })
        .unwrap();
        log.flush().unwrap();
        drop(log);
        assert_eq!(finalise(&log_path, &output).unwrap(), 2);
        assert!(
            !log_path.exists(),
            "the log is gone once it's turned into the real file"
        );
        let track = CursorTrack::parse(&std::fs::read_to_string(&output).unwrap()).unwrap();
        assert_eq!(track.samples.len(), 2);
        assert_eq!(track.samples[1].interaction, Interaction::Click);
        assert_eq!(track.samples[1].shape, CursorShape::Pointer);
    }

    #[test]
    fn half_a_line_left_by_a_crash_is_skipped() {
        let folder = tempfile::tempdir().unwrap();
        let (log_path, output) = (folder.path().join("a.jsonl"), folder.path().join("a.json"));
        let mut log = CursorLog::create(&log_path).unwrap();
        for index in 0..5 {
            log.push(&sample(f64::from(index))).unwrap();
        }
        log.flush().unwrap();
        drop(log);
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&log_path)
            .unwrap();
        file.write_all(b"{\"timeMs\":5,\"cx\":0.").unwrap();
        drop(file);
        assert_eq!(finalise(&log_path, &output).unwrap(), 5);
    }

    #[test]
    fn samples_written_out_of_order_come_out_in_order() {
        let folder = tempfile::tempdir().unwrap();
        let (log_path, output) = (folder.path().join("a.jsonl"), folder.path().join("a.json"));
        let mut log = CursorLog::create(&log_path).unwrap();
        log.push(&sample(50.0)).unwrap();
        log.push(&sample(10.0)).unwrap();
        log.flush().unwrap();
        drop(log);
        finalise(&log_path, &output).unwrap();
        let track = CursorTrack::parse(&std::fs::read_to_string(&output).unwrap()).unwrap();
        assert!(track.samples[0].time < track.samples[1].time);
    }

    #[test]
    fn more_than_an_hour_is_kept() {
        let folder = tempfile::tempdir().unwrap();
        let (log_path, output) = (folder.path().join("a.jsonl"), folder.path().join("a.json"));
        let mut log = CursorLog::create(&log_path).unwrap();
        for index in 0..110_000 {
            log.push(&sample(f64::from(index) * 33.0)).unwrap();
        }
        log.flush().unwrap();
        drop(log);
        assert_eq!(finalise(&log_path, &output).unwrap(), 110_000);
    }
}
