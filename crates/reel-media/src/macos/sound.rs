//! Reading a file's sound with AVFoundation.

use std::ffi::c_void;
use std::path::Path;
use std::ptr::NonNull;

use objc2::rc::autoreleasepool;
use objc2_av_foundation::{AVAssetReader, AVAssetReaderTrackOutput, AVMediaTypeAudio, AVURLAsset};
use objc2_foundation::{NSNumber, NSString};

use crate::mix::{CHANNELS, Pcm, SAMPLE_RATE};

use super::{MediaError, dictionary, file_url};

const LINEAR_PCM: u32 = 0x6c70_636d;

/// Reads the sound in a file as stereo at the working sample rate, or `None` if it has none.
///
/// # Errors
/// Returns an error if there's sound but it can't be read.
pub fn read_pcm(path: &Path) -> Result<Option<Pcm>, MediaError> {
    // SAFETY: documented AVFoundation calls in their documented order, on objects made here. Each
    // copy is limited to the length the block buffer reports, within a buffer sized to match.
    unsafe {
        let asset = AVURLAsset::URLAssetWithURL_options(&file_url(path), None);
        #[allow(deprecated)]
        let tracks = asset.tracksWithMediaType(AVMediaTypeAudio.expect("this constant always exists"));
        let Some(track) = tracks.firstObject() else {
            return Ok(None);
        };
        let reader = AVAssetReader::assetReaderWithAsset_error(&asset)
            .map_err(|error| MediaError(format!("The audio in {} couldn't be opened: {error}", path.display())))?;

        let format = NSNumber::new_u32(LINEAR_PCM);
        let rate = NSNumber::new_f64(f64::from(SAMPLE_RATE));
        let channels = NSNumber::new_usize(CHANNELS);
        let depth = NSNumber::new_u32(16);
        let no = NSNumber::new_bool(false);
        let settings = dictionary(&[
            (&NSString::from_str("AVFormatIDKey"), format.as_ref()),
            (&NSString::from_str("AVSampleRateKey"), rate.as_ref()),
            (&NSString::from_str("AVNumberOfChannelsKey"), channels.as_ref()),
            (&NSString::from_str("AVLinearPCMBitDepthKey"), depth.as_ref()),
            (&NSString::from_str("AVLinearPCMIsFloatKey"), no.as_ref()),
            (&NSString::from_str("AVLinearPCMIsBigEndianKey"), no.as_ref()),
            (&NSString::from_str("AVLinearPCMIsNonInterleaved"), no.as_ref()),
        ]);
        let output =
            AVAssetReaderTrackOutput::assetReaderTrackOutputWithTrack_outputSettings(&track, Some(&settings));
        output.setAlwaysCopiesSampleData(false);
        reader.addOutput(&output);
        if !reader.startReading() {
            return Err(MediaError(format!("The audio in {} couldn't be read.", path.display())));
        }

        let mut samples: Vec<i16> = Vec::new();
        loop {
            let more = autoreleasepool(|_| {
                let Some(sample) = output.copyNextSampleBuffer() else {
                    return false;
                };
                if let Some(block) = sample.data_buffer() {
                    let bytes = block.data_length();
                    let start = samples.len();
                    samples.resize(start + bytes / 2, 0);
                    let destination = NonNull::new_unchecked(samples.as_mut_ptr().add(start).cast::<c_void>());
                    if block.copy_data_bytes(0, bytes, destination) != 0 {
                        samples.truncate(start);
                    }
                }
                true
            });
            if !more {
                break;
            }
        }
        Ok(Some(Pcm::new(samples)))
    }
}
