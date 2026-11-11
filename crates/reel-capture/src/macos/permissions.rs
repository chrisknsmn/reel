//! Checking and asking for permissions. Checking never shows a prompt, and asking only happens when
//! a feature needs it, never just because the app launched.

use std::sync::mpsc;

use block2::RcBlock;
use objc2::runtime::Bool;
use objc2_av_foundation::{
    AVAuthorizationStatus, AVCaptureDevice, AVMediaTypeAudio, AVMediaTypeVideo,
};
use objc2_core_graphics::{CGPreflightScreenCaptureAccess, CGRequestScreenCaptureAccess};
use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Access {
    Granted,
    Denied,
    NotAsked,
}

/// Whether the screen can be recorded. This doesn't prompt.
#[must_use]
pub fn screen_recording() -> Access {
    if CGPreflightScreenCaptureAccess() {
        Access::Granted
    } else {
        // macOS doesn't say whether it's been asked before, only that it isn't allowed.
        Access::NotAsked
    }
}

fn device_access(microphone: bool) -> Access {
    // SAFETY: these constants always exist, and this is a documented read that doesn't prompt.
    let status = unsafe {
        let kind = if microphone {
            AVMediaTypeAudio
        } else {
            AVMediaTypeVideo
        };
        AVCaptureDevice::authorizationStatusForMediaType(kind.expect("this constant always exists"))
    };
    match status {
        AVAuthorizationStatus::Authorized => Access::Granted,
        AVAuthorizationStatus::NotDetermined => Access::NotAsked,
        _ => Access::Denied,
    }
}

#[must_use]
pub fn microphone() -> Access {
    device_access(true)
}

#[must_use]
pub fn camera() -> Access {
    device_access(false)
}

/// Shows the system's prompt for the screen, if it hasn't been answered. The app has to restart before
/// a new answer takes effect.
#[must_use]
pub fn request_screen_recording() -> bool {
    CGRequestScreenCaptureAccess()
}

fn request_device(microphone: bool) -> bool {
    let (send, receive) = mpsc::channel();
    let handler = RcBlock::new(move |granted: Bool| {
        let _ = send.send(granted.as_bool());
    });
    // SAFETY: these constants always exist, and the block outlives the call, which copies it.
    unsafe {
        let kind = if microphone {
            AVMediaTypeAudio
        } else {
            AVMediaTypeVideo
        };
        AVCaptureDevice::requestAccessForMediaType_completionHandler(
            kind.expect("this constant always exists"),
            &handler,
        );
    }
    receive.recv().unwrap_or(false)
}

/// Shows the system's prompt for the mic, if it hasn't been answered, and waits for the answer.
#[must_use]
pub fn request_microphone() -> bool {
    request_device(true)
}

/// Shows the system's prompt for the camera, if it hasn't been answered, and waits for the answer.
#[must_use]
pub fn request_camera() -> bool {
    request_device(false)
}
