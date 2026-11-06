//! Demux, decode, encode, mux, audio mixing.

pub mod bitrate;
pub mod mix;
pub mod player;

#[cfg(target_os = "macos")]
pub mod macos;
