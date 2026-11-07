//! Recording on macOS: ScreenCaptureKit for the screen and system audio, AVFoundation for the mic and
//! webcam, and an event tap for clicks.

pub mod devices;
pub mod permissions;
pub mod pointer;
pub mod recorder;
pub mod run;
pub mod screen;
pub mod synthetic;
pub mod windows;
