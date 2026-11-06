//! Demux, decode, encode, mux, audio mixing.

pub mod bitrate;
pub mod mix;

#[cfg(target_os = "macos")]
pub mod macos;
