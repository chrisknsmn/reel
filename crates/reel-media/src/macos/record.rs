//! Writing a recording as it happens.
//!
//! Unlike an export, nothing here can wait: frames arrive in real time, so one that can't be taken is
//! dropped and counted. The file is written in fragments, so a crash leaves everything up to the last
//! fragment playable (§58, §59).
//!
//! Sound files come out continuous. A gap in what arrives is filled with silence and a small overlap
//! is pulled into place, which keeps every sample within `TOLERANCE` of its true time. Anything reading
//! these files can treat them as one unbroken stream, so tracks can't drift apart.

use std::path::{Path, PathBuf};
use std::ptr::{NonNull, null, null_mut};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use block2::RcBlock;
use objc2::rc::Retained;
use objc2_av_foundation::{
    AVAssetWriter, AVAssetWriterInput, AVAssetWriterInputPixelBufferAdaptor, AVAssetWriterStatus,
    AVFileTypeAppleM4A, AVFileTypeMPEG4,
};
use objc2_core_audio_types::{AudioStreamBasicDescription, kAudioFormatFlagIsNonInterleaved};
use objc2_core_foundation::CFRetained;
use objc2_core_media::{
    CMAudioFormatDescription, CMAudioFormatDescriptionGetStreamBasicDescription,
    CMAudioSampleBufferCreateReadyWithPacketDescriptions, CMBlockBuffer, CMFormatDescription,
    CMSampleBuffer, CMSampleTimingInfo, CMTime, kCMBlockBufferAssureMemoryNowFlag,
};
use objc2_core_video::CVPixelBuffer;
use reel_core::time::Micros;

use super::encode::{Encoder, EncoderSettings};
use super::{MediaError, file_url};

/// How often the file becomes safe against a crash. A crash loses at most about this much.
const FRAGMENT_SECONDS: i64 = 1;
/// How far sound may sit from its true time before it's corrected.
pub const TOLERANCE: Micros = Micros(20_000);
const MICROSECONDS: i32 = 1_000_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RecordSettings {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub bitrate: u32,
    pub keyframe_interval: u32,
}

fn time_of(value: Micros) -> CMTime {
    // SAFETY: makes a plain time value.
    unsafe { CMTime::new(value.0, MICROSECONDS) }
}

fn failure(writer: &AVAssetWriter, doing: &str) -> MediaError {
    // SAFETY: a documented property read.
    let reason = unsafe { writer.error() };
    MediaError(format!(
        "{doing} failed: {}",
        reason.map_or_else(
            || "no reason given".to_string(),
            |error| {
                // The top error is always the same "couldn't be completed". The cause is underneath.
                let details = error.userInfo();
                let key = objc2_foundation::NSString::from_str("NSUnderlyingError");
                let cause = details
                    .objectForKey(&key)
                    .and_then(|cause| cause.downcast::<objc2_foundation::NSError>().ok())
                    .map_or_else(String::new, |cause| {
                        format!(", because of {} {}", cause.domain(), cause.code())
                    });
                format!("{error} ({} {}{cause})", error.domain(), error.code())
            }
        )
    ))
}

fn start_writer(
    path: &Path,
    file_type: &objc2_foundation::NSString,
) -> Result<Retained<AVAssetWriter>, MediaError> {
    if path.exists() {
        std::fs::remove_file(path).map_err(|error| {
            MediaError(format!("{} can't be replaced: {error}", path.display()))
        })?;
    }
    // SAFETY: documented calls on a writer made here, before writing starts.
    unsafe {
        let writer = AVAssetWriter::assetWriterWithURL_fileType_error(&file_url(path), file_type)
            .map_err(|error| {
            MediaError(format!("{} can't be written: {error}", path.display()))
        })?;
        writer.setShouldOptimizeForNetworkUse(false);
        writer.setMovieFragmentInterval(CMTime::new(FRAGMENT_SECONDS, 1));
        Ok(writer)
    }
}

fn begin(writer: &AVAssetWriter) -> Result<(), MediaError> {
    // SAFETY: documented calls, in their documented order.
    unsafe {
        if !writer.startWriting() {
            return Err(failure(writer, "Starting the recording"));
        }
        writer.startSessionAtSourceTime(time_of(Micros::ZERO));
    }
    Ok(())
}

fn finish_writer(
    writer: &AVAssetWriter,
    input: &AVAssetWriterInput,
    end: Micros,
) -> Result<(), MediaError> {
    // SAFETY: documented calls, in order, on objects the caller owns. The block outlives the call.
    unsafe {
        input.markAsFinished();
        writer.endSessionAtSourceTime(time_of(end));
        let done = Arc::new(AtomicBool::new(false));
        let flag = done.clone();
        let handler = RcBlock::new(move || flag.store(true, Ordering::Release));
        writer.finishWritingWithCompletionHandler(&handler);
        while !done.load(Ordering::Acquire) {
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        if writer.status() != AVAssetWriterStatus::Completed {
            return Err(failure(writer, "Finishing the recording"));
        }
    }
    Ok(())
}

/// How a recording's video went.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct VideoStats {
    pub frames: u64,
    /// Frames that arrived while the encoder wasn't ready.
    pub dropped: u64,
}

/// A recording's picture, written as frames arrive.
pub struct VideoRecorder {
    writer: Retained<AVAssetWriter>,
    input: Retained<AVAssetWriterInput>,
    adaptor: Retained<AVAssetWriterInputPixelBufferAdaptor>,
    path: PathBuf,
    last: Option<Micros>,
    stats: VideoStats,
}

// SAFETY: this is used from one thread at a time, behind a lock, which AVAssetWriter allows.
unsafe impl Send for VideoRecorder {}

impl VideoRecorder {
    /// Starts a new file, replacing any that's there.
    ///
    /// # Errors
    /// Returns an error if the file can't be written, or the encoder won't take these settings.
    pub fn create(path: &Path, settings: RecordSettings) -> Result<Self, MediaError> {
        // SAFETY: this constant always exists.
        let file_type = unsafe { AVFileTypeMPEG4 }.expect("this constant always exists");
        let writer = start_writer(path, file_type)?;
        let input = Encoder::video_input(
            &EncoderSettings {
                width: settings.width,
                height: settings.height,
                fps: settings.fps,
                bitrate: settings.bitrate,
                keyframe_interval: settings.keyframe_interval,
                audio: false,
            },
            // Fragments and reordered frames don't mix: with them, AVAssetWriter fails with -16341 at a
            // fragment boundary. In testing that was 6 failures in 6 at 1080p60, and none in 36 without.
            false,
        );
        // SAFETY: documented calls on objects made here, before writing starts.
        let adaptor = unsafe {
            input.setExpectsMediaDataInRealTime(true);
            if !writer.canAddInput(&input) {
                return Err(MediaError(
                    "The encoder doesn't accept these video settings.".to_string(),
                ));
            }
            writer.addInput(&input);
            AVAssetWriterInputPixelBufferAdaptor::assetWriterInputPixelBufferAdaptorWithAssetWriterInput_sourcePixelBufferAttributes(&input, None)
        };
        begin(&writer)?;
        Ok(Self {
            writer,
            input,
            adaptor,
            path: path.to_path_buf(),
            last: None,
            stats: VideoStats::default(),
        })
    }

    /// Adds a frame at a time on the recording. Returns whether it was taken: a frame that can't be
    /// taken straight away is dropped, because the next is already on its way.
    ///
    /// # Errors
    /// Returns an error if the encoder has failed, which ends the recording.
    pub fn append(&mut self, buffer: &CVPixelBuffer, time: Micros) -> Result<bool, MediaError> {
        if self.last.is_some_and(|last| time <= last) {
            return Ok(false);
        }
        // SAFETY: documented calls on objects this owns, and the buffer is live for the call.
        unsafe {
            if self.writer.status() != AVAssetWriterStatus::Writing {
                return Err(failure(&self.writer, "Recording"));
            }
            if !self.input.isReadyForMoreMediaData() {
                self.stats.dropped += 1;
                return Ok(false);
            }
            if !self
                .adaptor
                .appendPixelBuffer_withPresentationTime(buffer, time_of(time))
            {
                return Err(failure(&self.writer, "Recording"));
            }
        }
        self.last = Some(time);
        self.stats.frames += 1;
        Ok(true)
    }

    #[must_use]
    pub fn stats(&self) -> VideoStats {
        self.stats
    }

    /// The time of the last frame taken.
    #[must_use]
    pub fn last_time(&self) -> Option<Micros> {
        self.last
    }

    /// Finishes the file, which then runs until `end`.
    ///
    /// # Errors
    /// Returns an error if the encoder failed at any point. What's been written so far is still there.
    pub fn finish(self, end: Micros) -> Result<VideoStats, MediaError> {
        finish_writer(&self.writer, &self.input, end)?;
        remove_leftovers(&self.path);
        Ok(self.stats)
    }

    /// Stops without finishing the file, and deletes it.
    pub fn cancel(self) {
        // SAFETY: a documented call on a writer this owns.
        unsafe { self.writer.cancelWriting() };
        let _ = std::fs::remove_file(&self.path);
        remove_leftovers(&self.path);
    }
}

fn remove_leftovers(path: &Path) {
    super::encode::remove_safe_save_leftovers(path);
}

/// What a sound buffer's audio is, as far as lining it up needs.
#[derive(Clone, Copy)]
struct Format {
    rate: f64,
    description: AudioStreamBasicDescription,
}

fn format_of(sample: &CMSampleBuffer) -> Option<(CFRetained<CMFormatDescription>, Format)> {
    // SAFETY: the description comes from a live sample buffer, and is retained for as long as it's used.
    unsafe {
        let description = sample.format_description()?;
        let audio: &CMFormatDescription = &description;
        let basic = CMAudioFormatDescriptionGetStreamBasicDescription(
            (audio as *const CMFormatDescription)
                .cast::<CMAudioFormatDescription>()
                .as_ref()?,
        );
        let basic = *basic.as_ref()?;
        Some((
            description,
            Format {
                rate: basic.mSampleRate,
                description: basic,
            },
        ))
    }
}

/// How a run of sound should be placed, given where the file has got to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Placement {
    /// Write it straight after what's there.
    Next,
    /// Write this many frames of silence first.
    AfterSilence(i64),
    /// All of it is earlier than what's already written.
    Drop,
}

/// Decides how to place sound of `frames` length at `time`, when `written` frames are in the file.
#[must_use]
pub fn place_sound(written: i64, frames: i64, time: Micros, rate: f64) -> Placement {
    let tolerance = (TOLERANCE.secs() * rate) as i64;
    let wanted = (time.secs() * rate).round() as i64;
    let ahead = wanted - written;
    if ahead > tolerance {
        Placement::AfterSilence(ahead)
    } else if -ahead > tolerance && -ahead >= frames {
        Placement::Drop
    } else {
        Placement::Next
    }
}

/// One track of a recording's sound, written as it arrives.
pub struct AudioRecorder {
    writer: Retained<AVAssetWriter>,
    input: Retained<AVAssetWriterInput>,
    path: PathBuf,
    /// How many frames are in the file, in the source's rate.
    written: i64,
    format: Option<(CFRetained<CMFormatDescription>, Format)>,
}

// SAFETY: this is used from one thread at a time, behind a lock, which AVAssetWriter allows.
unsafe impl Send for AudioRecorder {}

impl AudioRecorder {
    /// Starts a new AAC file at 48 kHz stereo, replacing any that's there.
    ///
    /// # Errors
    /// Returns an error if the file can't be written.
    pub fn create(path: &Path, bitrate: u32) -> Result<Self, MediaError> {
        // SAFETY: this constant always exists.
        let file_type = unsafe { AVFileTypeAppleM4A }.expect("this constant always exists");
        let writer = start_writer(path, file_type)?;
        let input = Encoder::audio_input(bitrate);
        // SAFETY: documented calls on objects made here, before writing starts.
        unsafe {
            input.setExpectsMediaDataInRealTime(true);
            if !writer.canAddInput(&input) {
                return Err(MediaError(
                    "The encoder doesn't accept these audio settings.".to_string(),
                ));
            }
            writer.addInput(&input);
        }
        begin(&writer)?;
        Ok(Self {
            writer,
            input,
            path: path.to_path_buf(),
            written: 0,
            format: None,
        })
    }

    /// Adds sound that belongs at `time` on the recording.
    ///
    /// # Errors
    /// Returns an error if the encoder has failed. Sound the encoder isn't ready for is only skipped,
    /// and the next buffer fills the hole with silence.
    pub fn append(&mut self, sample: &CMSampleBuffer, time: Micros) -> Result<(), MediaError> {
        if self.format.is_none() {
            self.format = format_of(sample);
        }
        let Some((_, format)) = self.format.as_ref() else {
            return Err(MediaError("A sound buffer had no format.".to_string()));
        };
        let format = *format;
        // SAFETY: a documented read on a live buffer.
        let frames = unsafe { sample.num_samples() } as i64;
        if frames <= 0 {
            return Ok(());
        }
        // SAFETY: documented property reads on objects this owns.
        unsafe {
            if self.writer.status() != AVAssetWriterStatus::Writing {
                return Err(failure(&self.writer, "Recording sound"));
            }
            if !self.input.isReadyForMoreMediaData() {
                return Ok(());
            }
        }
        match place_sound(self.written, frames, time, format.rate) {
            Placement::Drop => return Ok(()),
            Placement::AfterSilence(silence) => self.write_silence(silence, format)?,
            Placement::Next => {}
        }
        self.write(sample, frames, format)
    }

    fn pts(&self, format: Format) -> CMTime {
        // SAFETY: makes a plain time value.
        unsafe { CMTime::new(self.written, format.rate as i32) }
    }

    fn write(
        &mut self,
        sample: &CMSampleBuffer,
        frames: i64,
        format: Format,
    ) -> Result<(), MediaError> {
        // SAFETY: the timing entry lives through the call, which copies it, and the result is checked.
        unsafe {
            let timing = CMSampleTimingInfo {
                duration: CMTime::new(1, format.rate as i32),
                presentationTimeStamp: self.pts(format),
                decodeTimeStamp: objc2_core_media::kCMTimeInvalid,
            };
            let mut copy: *mut CMSampleBuffer = null_mut();
            let status = CMSampleBuffer::create_copy_with_new_timing(
                None,
                sample,
                1,
                &timing,
                NonNull::from(&mut copy),
            );
            let copy = NonNull::new(copy).filter(|_| status == 0).ok_or_else(|| {
                MediaError(format!("Couldn't retime the sound (error {status})."))
            })?;
            let copy: CFRetained<CMSampleBuffer> = CFRetained::from_raw(copy);
            if !self.input.appendSampleBuffer(&copy) {
                return Err(failure(&self.writer, "Recording sound"));
            }
        }
        self.written += frames;
        Ok(())
    }

    fn write_silence(&mut self, frames: i64, format: Format) -> Result<(), MediaError> {
        let basic = format.description;
        let planes = if basic.mFormatFlags & kAudioFormatFlagIsNonInterleaved != 0 {
            basic.mChannelsPerFrame as usize
        } else {
            1
        };
        let Some((description, _)) = self.format.as_ref() else {
            return Ok(());
        };
        // Silence goes in in steps of a tenth of a second at most, so as not to swamp the encoder.
        let step = (format.rate / 10.0).max(1.0) as i64;
        let mut remaining = frames;
        while remaining > 0 {
            let count = remaining.min(step);
            let bytes = count as usize * basic.mBytesPerFrame as usize * planes;
            // SAFETY: the block is made to hold `bytes` and zeroed. Both objects are checked and
            // retained by these wrappers, and the description matches what was already written.
            unsafe {
                let mut block: *mut CMBlockBuffer = null_mut();
                let status = CMBlockBuffer::create_with_memory_block(
                    None,
                    null_mut(),
                    bytes,
                    None,
                    null(),
                    0,
                    bytes,
                    kCMBlockBufferAssureMemoryNowFlag,
                    NonNull::from(&mut block),
                );
                let block = NonNull::new(block).filter(|_| status == 0).ok_or_else(|| {
                    MediaError(format!("Couldn't make silence (error {status})."))
                })?;
                let block: CFRetained<CMBlockBuffer> = CFRetained::from_raw(block);
                let status = CMBlockBuffer::fill_data_bytes(0, &block, 0, bytes);
                if status != 0 {
                    return Err(MediaError(format!(
                        "Couldn't clear silence (error {status})."
                    )));
                }
                let mut sample: *mut CMSampleBuffer = null_mut();
                let status = CMAudioSampleBufferCreateReadyWithPacketDescriptions(
                    None,
                    &block,
                    description,
                    count as isize,
                    self.pts(format),
                    null(),
                    NonNull::from(&mut sample),
                );
                let sample = NonNull::new(sample)
                    .filter(|_| status == 0)
                    .ok_or_else(|| {
                        MediaError(format!("Couldn't package silence (error {status})."))
                    })?;
                let sample: CFRetained<CMSampleBuffer> = CFRetained::from_raw(sample);
                while !self.input.isReadyForMoreMediaData() {
                    if self.writer.status() != AVAssetWriterStatus::Writing {
                        return Err(failure(&self.writer, "Recording sound"));
                    }
                    std::thread::sleep(std::time::Duration::from_micros(200));
                }
                if !self.input.appendSampleBuffer(&sample) {
                    return Err(failure(&self.writer, "Recording sound"));
                }
            }
            self.written += count;
            remaining -= count;
        }
        Ok(())
    }

    /// How much sound is in the file.
    #[must_use]
    pub fn duration(&self) -> Micros {
        let rate = self
            .format
            .as_ref()
            .map_or(48_000.0, |(_, format)| format.rate);
        Micros((self.written as f64 / rate * 1_000_000.0) as i64)
    }

    /// Finishes the file. A track that never got any sound leaves no file, and returns `false`.
    ///
    /// # Errors
    /// Returns an error if the encoder failed at any point.
    pub fn finish(self, end: Micros) -> Result<bool, MediaError> {
        if self.written == 0 {
            // SAFETY: a documented call on a writer this owns.
            unsafe { self.writer.cancelWriting() };
            let _ = std::fs::remove_file(&self.path);
            remove_leftovers(&self.path);
            return Ok(false);
        }
        finish_writer(&self.writer, &self.input, end.max(self.duration()))?;
        remove_leftovers(&self.path);
        Ok(true)
    }
}

/// The host clock's time now, which capture timestamps are measured on.
#[must_use]
pub fn host_now() -> Micros {
    // SAFETY: the host clock always exists, and reading it has no requirements.
    unsafe { micros_of(objc2_core_media::CMClock::host_time_clock().time()) }
}

/// A media time in microseconds.
#[must_use]
pub fn micros_of(time: CMTime) -> Micros {
    if time.timescale <= 0 {
        return Micros::ZERO;
    }
    Micros((i128::from(time.value) * 1_000_000 / i128::from(time.timescale)) as i64)
}

/// Packages interleaved 16-bit stereo as a sound buffer at a host time, as a capture source would.
///
/// # Errors
/// Returns an error if CoreMedia won't make the buffer.
pub fn pcm_sample(samples: &[i16], time: Micros) -> Result<CFRetained<CMSampleBuffer>, MediaError> {
    let format = super::encode::pcm_format()?;
    let bytes = std::mem::size_of_val(samples);
    let frames = (samples.len() / crate::mix::CHANNELS) as isize;
    // SAFETY: the block is made to hold `bytes`, which are copied from a slice of that length. Both
    // objects are checked and retained by these wrappers.
    unsafe {
        let mut block: *mut CMBlockBuffer = null_mut();
        let status = CMBlockBuffer::create_with_memory_block(
            None,
            null_mut(),
            bytes,
            None,
            null(),
            0,
            bytes,
            kCMBlockBufferAssureMemoryNowFlag,
            NonNull::from(&mut block),
        );
        let block = NonNull::new(block)
            .filter(|_| status == 0)
            .ok_or_else(|| MediaError(format!("Couldn't make room for sound (error {status}).")))?;
        let block: CFRetained<CMBlockBuffer> = CFRetained::from_raw(block);
        let status = CMBlockBuffer::replace_data_bytes(
            NonNull::new_unchecked(samples.as_ptr().cast::<std::ffi::c_void>().cast_mut()),
            &block,
            0,
            bytes,
        );
        if status != 0 {
            return Err(MediaError(format!("Couldn't copy sound (error {status}).")));
        }
        let description: &CMFormatDescription = &format;
        let mut sample: *mut CMSampleBuffer = null_mut();
        let status = CMAudioSampleBufferCreateReadyWithPacketDescriptions(
            None,
            &block,
            description,
            frames,
            time_of(time),
            null(),
            NonNull::from(&mut sample),
        );
        let sample = NonNull::new(sample)
            .filter(|_| status == 0)
            .ok_or_else(|| MediaError(format!("Couldn't package sound (error {status}).")))?;
        Ok(CFRetained::from_raw(sample))
    }
}

/// The average brightness across a frame's first plane. Tests read test patterns with this.
///
/// # Errors
/// Returns an error if the buffer can't be read.
pub fn average_luma(buffer: &CVPixelBuffer) -> Result<f64, MediaError> {
    use objc2_core_video::{
        CVPixelBufferGetBaseAddressOfPlane, CVPixelBufferGetBytesPerRowOfPlane,
        CVPixelBufferGetHeightOfPlane, CVPixelBufferGetWidthOfPlane, CVPixelBufferLockBaseAddress,
        CVPixelBufferLockFlags, CVPixelBufferUnlockBaseAddress,
    };
    // SAFETY: the buffer is locked for reading around these reads, which stay inside its first plane.
    unsafe {
        if CVPixelBufferLockBaseAddress(buffer, CVPixelBufferLockFlags::ReadOnly) != 0 {
            return Err(MediaError("A frame couldn't be read.".to_string()));
        }
        let base = CVPixelBufferGetBaseAddressOfPlane(buffer, 0).cast::<u8>();
        let (width, height) = (
            CVPixelBufferGetWidthOfPlane(buffer, 0),
            CVPixelBufferGetHeightOfPlane(buffer, 0),
        );
        let stride = CVPixelBufferGetBytesPerRowOfPlane(buffer, 0);
        let mut total = 0_u64;
        let mut count = 0_u64;
        if !base.is_null() {
            for y in (0..height).step_by((height / 16).max(1)) {
                for x in (0..width).step_by((width / 16).max(1)) {
                    total += u64::from(*base.add(y * stride + x));
                    count += 1;
                }
            }
        }
        CVPixelBufferUnlockBaseAddress(buffer, CVPixelBufferLockFlags::ReadOnly);
        Ok(total as f64 / count.max(1) as f64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: f64 = 48_000.0;

    #[test]
    fn sound_on_time_goes_straight_on() {
        assert_eq!(
            place_sound(48_000, 480, Micros::from_secs(1), RATE),
            Placement::Next
        );
    }

    #[test]
    fn a_small_wobble_is_ignored() {
        assert_eq!(
            place_sound(48_000, 480, Micros::from_ms(1015.0), RATE),
            Placement::Next
        );
        assert_eq!(
            place_sound(48_000, 480, Micros::from_ms(985.0), RATE),
            Placement::Next
        );
    }

    #[test]
    fn a_gap_is_filled_with_exactly_enough_silence() {
        assert_eq!(
            place_sound(0, 480, Micros::from_ms(300.0), RATE),
            Placement::AfterSilence(14_400)
        );
        assert_eq!(
            place_sound(48_000, 480, Micros::from_secs(2), RATE),
            Placement::AfterSilence(48_000)
        );
    }

    #[test]
    fn sound_entirely_behind_the_file_is_dropped() {
        assert_eq!(
            place_sound(48_000, 480, Micros::from_ms(900.0), RATE),
            Placement::Drop
        );
    }

    #[test]
    fn a_long_buffer_that_only_partly_overlaps_is_kept() {
        assert_eq!(
            place_sound(48_000, 9_600, Micros::from_ms(900.0), RATE),
            Placement::Next
        );
    }

    #[test]
    fn placement_keeps_sound_within_tolerance_whatever_the_clock_does() {
        // A source clock running 0.5% fast, and one 0.5% slow, for a minute of 10 ms buffers.
        for speed in [1.005, 0.995] {
            let mut written = 0_i64;
            let mut worst = 0.0_f64;
            for index in 0..6000 {
                let frames = 480;
                // The buffer has this many frames, but its timestamp runs at the other clock's speed.
                let time = Micros((f64::from(index) * 10_000.0 * speed) as i64);
                match place_sound(written, frames, time, RATE) {
                    Placement::Drop => continue,
                    Placement::AfterSilence(silence) => written += silence,
                    Placement::Next => {}
                }
                let error = (written as f64 / RATE - time.secs()).abs();
                worst = worst.max(error);
                written += frames;
            }
            assert!(
                worst <= TOLERANCE.secs() + 0.011,
                "{speed}: off by {worst} s"
            );
        }
    }
}
