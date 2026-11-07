//! Whisper transcription, segmentation and caption layout. No UI or GPU dependencies.

pub mod editing;
pub mod models;
pub mod segment;
pub mod silence;
pub mod subtitles;
#[cfg(feature = "whisper")]
pub mod transcribe;
