//! AVFoundation decoding and encoding, chosen by T2 (`docs/spikes.md`).

mod encode;
mod sound;
mod video;

pub use encode::{Encoder, EncoderSettings};
pub use sound::read_pcm;
pub use video::{Decoder, Frame, VideoInfo, probe};

use std::path::Path;

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_foundation::{NSDictionary, NSString, NSURL};

/// Something went wrong reading or writing media, in words for the user.
#[derive(Debug)]
pub struct MediaError(pub String);

impl std::fmt::Display for MediaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for MediaError {}

fn file_url(path: &Path) -> Retained<NSURL> {
    NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy()))
}

fn dictionary(pairs: &[(&NSString, &AnyObject)]) -> Retained<NSDictionary<NSString, AnyObject>> {
    let keys: Vec<&NSString> = pairs.iter().map(|(key, _)| *key).collect();
    let values: Vec<&AnyObject> = pairs.iter().map(|(_, value)| *value).collect();
    NSDictionary::from_slices(&keys, &values)
}
