//! Platform capture and cursor tracking. No UI or GPU dependencies.

pub mod clock;
pub mod config;
pub mod cursor_log;
pub mod cursor_shape;
pub mod disk;
pub mod retention;
pub mod session;
pub mod sources;

#[cfg(target_os = "macos")]
pub mod macos;
