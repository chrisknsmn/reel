//! Writing H.264 MP4 through the hardware encoder, with AVAssetWriter.

use std::ffi::c_void;
use std::path::{Path, PathBuf};
use std::ptr::{NonNull, null, null_mut};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use block2::RcBlock;
use objc2::rc::Retained;
use objc2_av_foundation::{
    AVAssetWriter, AVAssetWriterInput, AVAssetWriterInputPixelBufferAdaptor, AVAssetWriterStatus,
    AVFileTypeMPEG4, AVMediaTypeAudio, AVMediaTypeVideo, AVVideoAverageBitRateKey, AVVideoCodecKey,
    AVVideoCodecTypeH264, AVVideoColorPrimaries_ITU_R_709_2, AVVideoColorPrimariesKey,
    AVVideoColorPropertiesKey, AVVideoCompressionPropertiesKey, AVVideoHeightKey,
    AVVideoMaxKeyFrameIntervalKey, AVVideoTransferFunction_ITU_R_709_2, AVVideoTransferFunctionKey,
    AVVideoWidthKey, AVVideoYCbCrMatrix_ITU_R_709_2, AVVideoYCbCrMatrixKey,
};
use objc2_core_audio_types::{
    AudioStreamBasicDescription, kAudioFormatFlagIsPacked, kAudioFormatFlagIsSignedInteger,
};
use objc2_core_foundation::CFRetained;
use objc2_core_media::{
    CMAudioFormatDescription, CMAudioFormatDescriptionCreate,
    CMAudioSampleBufferCreateReadyWithPacketDescriptions, CMBlockBuffer, CMFormatDescription,
    CMSampleBuffer, CMTime, kCMBlockBufferAssureMemoryNowFlag,
};
use objc2_core_video::{
    CVPixelBuffer, CVPixelBufferPool, kCVPixelFormatType_32BGRA, kCVReturnSuccess,
};
use objc2_foundation::{NSNumber, NSString};

use crate::mix::{CHANNELS, SAMPLE_RATE};

use super::{MediaError, dictionary, file_url};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EncoderSettings {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub bitrate: u32,
    pub keyframe_interval: u32,
    pub audio: bool,
}

/// `kAudioFormatMPEG4AAC`, which is the four characters `aac `.
const AAC: u32 = 0x6161_6320;
const AAC_BITRATE: u32 = 192_000;

/// Writes an MP4. Frames are rendered straight into buffers this hands out, so nothing is copied.
pub struct Encoder {
    writer: Retained<AVAssetWriter>,
    path: PathBuf,
    video: Retained<AVAssetWriterInput>,
    adaptor: Retained<AVAssetWriterInputPixelBufferAdaptor>,
    pool: Retained<CVPixelBufferPool>,
    audio: Option<Retained<AVAssetWriterInput>>,
    audio_format: Option<CFRetained<CMAudioFormatDescription>>,
    fps: u32,
    frames: i64,
    audio_frames: i64,
}

// SAFETY: the encoder is used from one thread at a time, which AVAssetWriter allows.
unsafe impl Send for Encoder {}

impl Encoder {
    /// Starts a new file, replacing any that's there.
    ///
    /// # Errors
    /// Returns an error if the writer won't start, such as when the folder can't be written.
    pub fn create(path: &Path, settings: EncoderSettings) -> Result<Self, MediaError> {
        if path.exists() {
            std::fs::remove_file(path).map_err(|error| {
                MediaError(format!("{} can't be replaced: {error}", path.display()))
            })?;
        }
        // SAFETY: documented AVFoundation calls in their documented order. Every dictionary holds the
        // key and value types documented for it, and the constants always exist.
        unsafe {
            let writer = AVAssetWriter::assetWriterWithURL_fileType_error(
                &file_url(path),
                AVFileTypeMPEG4.expect("this constant always exists"),
            )
            .map_err(|error| MediaError(format!("{} can't be written: {error}", path.display())))?;
            writer.setShouldOptimizeForNetworkUse(true);

            let video = Self::video_input(&settings);
            video.setExpectsMediaDataInRealTime(false);
            if !writer.canAddInput(&video) {
                return Err(MediaError(
                    "The encoder doesn't accept these video settings.".to_string(),
                ));
            }
            writer.addInput(&video);

            let format = NSNumber::new_u32(kCVPixelFormatType_32BGRA);
            let width = NSNumber::new_u32(settings.width);
            let height = NSNumber::new_u32(settings.height);
            let metal = NSNumber::new_bool(true);
            let attributes = dictionary(&[
                (&NSString::from_str("PixelFormatType"), format.as_ref()),
                (&NSString::from_str("Width"), width.as_ref()),
                (&NSString::from_str("Height"), height.as_ref()),
                (&NSString::from_str("MetalCompatibility"), metal.as_ref()),
            ]);
            let adaptor = AVAssetWriterInputPixelBufferAdaptor::assetWriterInputPixelBufferAdaptorWithAssetWriterInput_sourcePixelBufferAttributes(
                &video,
                Some(&attributes),
            );

            let (audio, audio_format) = if settings.audio {
                let input = Self::audio_input();
                input.setExpectsMediaDataInRealTime(false);
                if !writer.canAddInput(&input) {
                    return Err(MediaError(
                        "The encoder doesn't accept these audio settings.".to_string(),
                    ));
                }
                writer.addInput(&input);
                (Some(input), Some(pcm_format()?))
            } else {
                (None, None)
            };

            if !writer.startWriting() {
                return Err(MediaError(format!(
                    "The encoder wouldn't start: {}",
                    writer
                        .error()
                        .map_or_else(|| "no reason given".to_string(), |error| error.to_string())
                )));
            }
            writer.startSessionAtSourceTime(CMTime::new(0, settings.fps as i32));
            let pool = adaptor
                .pixelBufferPool()
                .ok_or_else(|| MediaError("The encoder has no frame buffers.".to_string()))?;

            Ok(Self {
                writer,
                path: path.to_path_buf(),
                video,
                adaptor,
                pool,
                audio,
                audio_format,
                fps: settings.fps,
                frames: 0,
                audio_frames: 0,
            })
        }
    }

    fn video_input(settings: &EncoderSettings) -> Retained<AVAssetWriterInput> {
        // SAFETY: documented keys with their documented value types, and these constants always exist.
        unsafe {
            let bitrate = NSNumber::new_u32(settings.bitrate);
            let keyframes = NSNumber::new_u32(settings.keyframe_interval);
            let compression = dictionary(&[
                (AVVideoAverageBitRateKey.expect("exists"), bitrate.as_ref()),
                (
                    AVVideoMaxKeyFrameIntervalKey.expect("exists"),
                    keyframes.as_ref(),
                ),
            ]);
            // BT.709, tagged so players show the colours as they were composited (§39).
            let color = dictionary(&[
                (
                    AVVideoColorPrimariesKey.expect("exists"),
                    AVVideoColorPrimaries_ITU_R_709_2.expect("exists").as_ref(),
                ),
                (
                    AVVideoTransferFunctionKey.expect("exists"),
                    AVVideoTransferFunction_ITU_R_709_2
                        .expect("exists")
                        .as_ref(),
                ),
                (
                    AVVideoYCbCrMatrixKey.expect("exists"),
                    AVVideoYCbCrMatrix_ITU_R_709_2.expect("exists").as_ref(),
                ),
            ]);
            let width = NSNumber::new_u32(settings.width);
            let height = NSNumber::new_u32(settings.height);
            let output = dictionary(&[
                (
                    AVVideoCodecKey.expect("exists"),
                    AVVideoCodecTypeH264.expect("exists").as_ref(),
                ),
                (AVVideoWidthKey.expect("exists"), width.as_ref()),
                (AVVideoHeightKey.expect("exists"), height.as_ref()),
                (
                    AVVideoCompressionPropertiesKey.expect("exists"),
                    compression.as_ref(),
                ),
                (AVVideoColorPropertiesKey.expect("exists"), color.as_ref()),
            ]);
            AVAssetWriterInput::assetWriterInputWithMediaType_outputSettings(
                AVMediaTypeVideo.expect("exists"),
                Some(&output),
            )
        }
    }

    fn audio_input() -> Retained<AVAssetWriterInput> {
        // SAFETY: documented keys with their documented value types, and this constant always exists.
        unsafe {
            let format = NSNumber::new_u32(AAC);
            let rate = NSNumber::new_f64(f64::from(SAMPLE_RATE));
            let channels = NSNumber::new_usize(CHANNELS);
            let bitrate = NSNumber::new_u32(AAC_BITRATE);
            let output = dictionary(&[
                (&NSString::from_str("AVFormatIDKey"), format.as_ref()),
                (&NSString::from_str("AVSampleRateKey"), rate.as_ref()),
                (
                    &NSString::from_str("AVNumberOfChannelsKey"),
                    channels.as_ref(),
                ),
                (&NSString::from_str("AVEncoderBitRateKey"), bitrate.as_ref()),
            ]);
            AVAssetWriterInput::assetWriterInputWithMediaType_outputSettings(
                AVMediaTypeAudio.expect("exists"),
                Some(&output),
            )
        }
    }

    /// A buffer to render the next frame into.
    ///
    /// # Errors
    /// Returns an error if the encoder's pool has run out, which means something else has gone wrong.
    pub fn next_buffer(&self) -> Result<CFRetained<CVPixelBuffer>, MediaError> {
        let mut raw: *mut CVPixelBuffer = null_mut();
        // SAFETY: the pool is live, and the out pointer is valid for the call.
        let status = unsafe {
            CVPixelBufferPool::create_pixel_buffer(None, &self.pool, NonNull::from(&mut raw))
        };
        if status != kCVReturnSuccess {
            return Err(MediaError(format!(
                "The encoder has no free frame buffers (error {status})."
            )));
        }
        let buffer = NonNull::new(raw)
            .ok_or_else(|| MediaError("The encoder gave an empty frame buffer.".to_string()))?;
        // SAFETY: the pool returned this with a +1 retain count, which this takes over.
        Ok(unsafe { CFRetained::from_raw(buffer) })
    }

    fn wait_for(&self, input: &AVAssetWriterInput) -> Result<(), MediaError> {
        // SAFETY: documented property reads on objects this owns.
        unsafe {
            while !input.isReadyForMoreMediaData() {
                if self.writer.status() != AVAssetWriterStatus::Writing {
                    return Err(self.failure());
                }
                std::thread::sleep(std::time::Duration::from_micros(200));
            }
        }
        Ok(())
    }

    fn failure(&self) -> MediaError {
        // SAFETY: a documented property read.
        let reason = unsafe { self.writer.error() };
        MediaError(format!(
            "Encoding failed: {}",
            reason.map_or_else(|| "no reason given".to_string(), |error| error.to_string())
        ))
    }

    /// Adds the next frame, which is always one frame's time after the last (§39).
    ///
    /// # Errors
    /// Returns an error if the encoder has failed.
    pub fn append_video(&mut self, buffer: &CVPixelBuffer) -> Result<(), MediaError> {
        self.wait_for(&self.video)?;
        let pts = CMTime::new_at(self.frames, self.fps);
        // SAFETY: the buffer is finished with, since the caller has waited for the GPU.
        let accepted = unsafe {
            self.adaptor
                .appendPixelBuffer_withPresentationTime(buffer, pts)
        };
        if !accepted {
            return Err(self.failure());
        }
        self.frames += 1;
        Ok(())
    }

    /// Adds the next stretch of sound, in interleaved stereo.
    ///
    /// # Errors
    /// Returns an error if the encoder has failed.
    pub fn append_audio(&mut self, samples: &[i16]) -> Result<(), MediaError> {
        let (Some(input), Some(format)) = (&self.audio, &self.audio_format) else {
            return Ok(());
        };
        if samples.is_empty() {
            return Ok(());
        }
        self.wait_for(input)?;
        let bytes = std::mem::size_of_val(samples);
        let frames = (samples.len() / CHANNELS) as isize;
        let pts = CMTime::new_at(self.audio_frames, SAMPLE_RATE);
        // SAFETY: the block buffer is created to hold exactly `bytes`, which are copied in from a
        // slice of that length. Both objects are checked before use and retained by these wrappers.
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
                MediaError(format!(
                    "Couldn't make room for the sound (error {status})."
                ))
            })?;
            let block: CFRetained<CMBlockBuffer> = CFRetained::from_raw(block);
            let status = CMBlockBuffer::replace_data_bytes(
                NonNull::new_unchecked(samples.as_ptr().cast::<c_void>().cast_mut()),
                &block,
                0,
                bytes,
            );
            if status != 0 {
                return Err(MediaError(format!(
                    "Couldn't copy the sound (error {status})."
                )));
            }
            let description: &CMFormatDescription = format;
            let mut sample: *mut CMSampleBuffer = null_mut();
            let status = CMAudioSampleBufferCreateReadyWithPacketDescriptions(
                None,
                &block,
                description,
                frames,
                pts,
                null(),
                NonNull::from(&mut sample),
            );
            let sample = NonNull::new(sample)
                .filter(|_| status == 0)
                .ok_or_else(|| {
                    MediaError(format!("Couldn't package the sound (error {status})."))
                })?;
            let sample: CFRetained<CMSampleBuffer> = CFRetained::from_raw(sample);
            if !input.appendSampleBuffer(&sample) {
                return Err(self.failure());
            }
        }
        self.audio_frames += frames as i64;
        Ok(())
    }

    /// Finishes the file. Until this returns, it isn't playable.
    ///
    /// # Errors
    /// Returns an error if the encoder failed at any point.
    pub fn finish(self) -> Result<(), MediaError> {
        // SAFETY: documented calls, in order, on objects this owns. The block outlives the call.
        unsafe {
            self.video.markAsFinished();
            if let Some(audio) = &self.audio {
                audio.markAsFinished();
            }
            self.writer
                .endSessionAtSourceTime(CMTime::new_at(self.frames, self.fps));
            // This finishes on its own queue, then calls back. Until then, the file isn't complete.
            let done = Arc::new(AtomicBool::new(false));
            let flag = done.clone();
            let handler = RcBlock::new(move || flag.store(true, Ordering::Release));
            self.writer.finishWritingWithCompletionHandler(&handler);
            while !done.load(Ordering::Acquire) {
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            if self.writer.status() != AVAssetWriterStatus::Completed {
                return Err(self.failure());
            }
        }
        remove_safe_save_leftovers(&self.path);
        Ok(())
    }

    /// Stops and discards the file, as cancelling an export does.
    pub fn cancel(self) {
        // SAFETY: a documented call on a writer this owns.
        unsafe { self.writer.cancelWriting() };
        remove_safe_save_leftovers(&self.path);
    }
}

/// With network optimisation on, AVAssetWriter moves the index to the front through a temporary file
/// named `<output>.sb-<random>`, and doesn't always clean it up before the process ends.
fn remove_safe_save_leftovers(path: &Path) {
    let (Some(folder), Some(name)) = (path.parent(), path.file_name()) else {
        return;
    };
    let prefix = format!("{}.sb-", name.to_string_lossy());
    let Ok(entries) = std::fs::read_dir(folder) else {
        return;
    };
    for entry in entries.flatten() {
        if entry.file_name().to_string_lossy().starts_with(&prefix) {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

trait TimeAt {
    fn new_at(value: i64, timescale: u32) -> CMTime;
}

impl TimeAt for CMTime {
    fn new_at(value: i64, timescale: u32) -> CMTime {
        // SAFETY: makes a plain time value.
        unsafe { CMTime::new(value, timescale as i32) }
    }
}

fn pcm_format() -> Result<CFRetained<CMAudioFormatDescription>, MediaError> {
    let bytes_per_frame = (CHANNELS * 2) as u32;
    let mut description = AudioStreamBasicDescription {
        mSampleRate: f64::from(SAMPLE_RATE),
        mFormatID: 0x6c70_636d,
        mFormatFlags: kAudioFormatFlagIsSignedInteger | kAudioFormatFlagIsPacked,
        mBytesPerPacket: bytes_per_frame,
        mFramesPerPacket: 1,
        mBytesPerFrame: bytes_per_frame,
        mChannelsPerFrame: CHANNELS as u32,
        mBitsPerChannel: 16,
        mReserved: 0,
    };
    let mut raw: *const CMAudioFormatDescription = null();
    // SAFETY: the description is valid for the call, and the out pointer is checked afterwards.
    unsafe {
        let status = CMAudioFormatDescriptionCreate(
            None,
            NonNull::from(&mut description),
            0,
            null(),
            0,
            null(),
            None,
            NonNull::from(&mut raw),
        );
        let raw = NonNull::new(raw.cast_mut())
            .filter(|_| status == 0)
            .ok_or_else(|| {
                MediaError(format!(
                    "Couldn't describe the sound format (error {status})."
                ))
            })?;
        Ok(CFRetained::from_raw(raw))
    }
}
