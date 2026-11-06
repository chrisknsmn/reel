//! Reading video with AVFoundation, into hardware-backed pixel buffers.

use std::path::Path;

use objc2::rc::{Retained, autoreleasepool};
use objc2_av_foundation::{
    AVAssetReader, AVAssetReaderStatus, AVAssetReaderTrackOutput, AVMediaTypeAudio, AVMediaTypeVideo, AVURLAsset,
};
use objc2_core_foundation::CFRetained;
use objc2_core_media::{CMTime, CMTimeRange, kCMTimePositiveInfinity};
use objc2_core_video::{CVPixelBuffer, kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange};
use objc2_foundation::{NSNumber, NSString};
use reel_core::time::{Micros, SourceTime};

use super::{MediaError, dictionary, file_url};

/// What a video file holds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VideoInfo {
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub duration: Micros,
    pub has_audio: bool,
}

/// One decoded frame: 4:2:0 BT.709 in limited range, in an IOSurface the GPU can read directly.
pub struct Frame {
    pub pts: SourceTime,
    pub buffer: CFRetained<CVPixelBuffer>,
}

// SAFETY: a decoded CVPixelBuffer is reference counted, and nothing writes to it after decoding.
unsafe impl Send for Frame {}

fn open_asset(path: &Path) -> Retained<AVURLAsset> {
    // SAFETY: a documented constructor, given a valid URL and no options.
    unsafe { AVURLAsset::URLAssetWithURL_options(&file_url(path), None) }
}

fn seconds_to_micros(seconds: f64) -> i64 {
    (seconds * 1_000_000.0).round() as i64
}

/// Reads a video's size, rate and length.
///
/// # Errors
/// Returns an error if the file has no readable video.
pub fn probe(path: &Path) -> Result<VideoInfo, MediaError> {
    let asset = open_asset(path);
    // SAFETY: documented property reads on objects that were just created.
    unsafe {
        #[allow(deprecated)]
        let tracks = asset.tracksWithMediaType(AVMediaTypeVideo.expect("this constant always exists"));
        let track = tracks
            .firstObject()
            .ok_or_else(|| MediaError(format!("{} has no video that can be read.", path.display())))?;
        let size = track.naturalSize();
        #[allow(deprecated)]
        let audio = asset.tracksWithMediaType(AVMediaTypeAudio.expect("this constant always exists"));
        Ok(VideoInfo {
            width: size.width as u32,
            height: size.height as u32,
            fps: f64::from(track.nominalFrameRate()),
            duration: Micros(seconds_to_micros(asset.duration().seconds())),
            has_audio: audio.count() > 0,
        })
    }
}

/// Reads frames in order, jumping when asked for a time that isn't nearby.
pub struct Decoder {
    asset: Retained<AVURLAsset>,
    reader: Retained<AVAssetReader>,
    output: Retained<AVAssetReaderTrackOutput>,
    current: Option<Frame>,
    upcoming: Option<Frame>,
    frame_length: Micros,
    duration: Micros,
}

/// Frame times don't land on exact microseconds, so allow half a millisecond.
const TIME_SLACK: Micros = Micros(500);
/// A jump forward of more than this is quicker to seek than to decode through.
const LONGEST_WALK: Micros = Micros(500_000);

// SAFETY: the decoder is used from one thread at a time, and AVFoundation's readers allow that.
unsafe impl Send for Decoder {}

impl Decoder {
    /// Opens a video file.
    ///
    /// # Errors
    /// Returns an error if the file can't be read.
    pub fn open(path: &Path) -> Result<Self, MediaError> {
        let info = probe(path)?;
        let asset = open_asset(path);
        let (reader, output) = Self::start_reader(&asset, None)?;
        Ok(Self {
            asset,
            reader,
            output,
            current: None,
            upcoming: None,
            frame_length: Micros((1_000_000.0 / info.fps.max(1.0)) as i64),
            duration: info.duration,
        })
    }

    fn start_reader(
        asset: &AVURLAsset,
        start: Option<SourceTime>,
    ) -> Result<(Retained<AVAssetReader>, Retained<AVAssetReaderTrackOutput>), MediaError> {
        // SAFETY: documented AVFoundation calls in their documented order. The settings dictionary
        // holds the documented key and value types.
        unsafe {
            let reader = AVAssetReader::assetReaderWithAsset_error(asset)
                .map_err(|error| MediaError(format!("The video couldn't be opened: {error}")))?;
            #[allow(deprecated)]
            let tracks = asset.tracksWithMediaType(AVMediaTypeVideo.expect("this constant always exists"));
            let track = tracks.firstObject().ok_or_else(|| MediaError("The file has no video.".to_string()))?;
            let format = NSNumber::new_u32(kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange);
            let metal = NSNumber::new_bool(true);
            let settings = dictionary(&[
                (&NSString::from_str("PixelFormatType"), format.as_ref()),
                (&NSString::from_str("MetalCompatibility"), metal.as_ref()),
            ]);
            let output =
                AVAssetReaderTrackOutput::assetReaderTrackOutputWithTrack_outputSettings(&track, Some(&settings));
            output.setAlwaysCopiesSampleData(false);
            reader.addOutput(&output);
            if let Some(start) = start {
                reader.setTimeRange(CMTimeRange {
                    start: CMTime::with_seconds(start.secs().max(0.0), 600_000),
                    duration: kCMTimePositiveInfinity,
                });
            }
            if !reader.startReading() {
                return Err(MediaError("The video couldn't be read.".to_string()));
            }
            Ok((reader, output))
        }
    }

    fn read_frame(&mut self) -> Option<Frame> {
        autoreleasepool(|_| {
            // SAFETY: a documented call on a reader this owns, retaining the image buffer it returns.
            unsafe {
                if self.reader.status() != AVAssetReaderStatus::Reading {
                    return None;
                }
                let sample = self.output.copyNextSampleBuffer()?;
                let buffer = sample.image_buffer()?;
                let pts = SourceTime(seconds_to_micros(sample.output_presentation_time_stamp().seconds()));
                Some(Frame { pts, buffer })
            }
        })
    }

    fn jump_to(&mut self, time: SourceTime) -> Result<(), MediaError> {
        // SAFETY: cancels a reader this owns before replacing it.
        unsafe { self.reader.cancelReading() };
        // Starting a little early guarantees a frame at or before the wanted time.
        let start = SourceTime((time.0 - self.frame_length.0 * 2).max(0));
        let (reader, output) = Self::start_reader(&self.asset, Some(start))?;
        self.reader = reader;
        self.output = output;
        self.current = None;
        self.upcoming = None;
        Ok(())
    }

    /// The frame showing at a time: the latest one that starts at or before it. Near the end of the
    /// file, this holds on the last frame.
    ///
    /// # Errors
    /// Returns an error if a jump fails.
    pub fn frame_at(&mut self, time: SourceTime) -> Result<Option<&Frame>, MediaError> {
        // Nothing comes after the last frame, and a reader started past it would find nothing at all.
        let last_frame = SourceTime((self.duration.0 - self.frame_length.0).max(0));
        let time = time.min(last_frame);
        let wanted = time + TIME_SLACK;
        let is_behind = self.current.as_ref().is_some_and(|frame| frame.pts > wanted);
        let is_far_ahead = self
            .upcoming
            .as_ref()
            .or(self.current.as_ref())
            .is_some_and(|frame| wanted - frame.pts > LONGEST_WALK);
        if is_behind || is_far_ahead {
            self.jump_to(time)?;
        }
        if self.current.is_none() {
            self.current = self.read_frame();
        }
        loop {
            if self.upcoming.is_none() {
                self.upcoming = self.read_frame();
            }
            match &self.upcoming {
                Some(next) if next.pts <= wanted => {
                    self.current = self.upcoming.take();
                }
                _ => break,
            }
        }
        Ok(self.current.as_ref())
    }
}
