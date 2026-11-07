//! The floating webcam preview (§5): a small draggable window showing the camera live. It uses a
//! native preview layer, which needs no decoding or drawing of its own, and is kept out of recordings.

use std::cell::RefCell;

use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2_app_kit::{
    NSBackingStoreType, NSColor, NSFloatingWindowLevel, NSPanel, NSWindowCollectionBehavior,
    NSWindowSharingType, NSWindowStyleMask,
};
use objc2_av_foundation::{
    AVCaptureDevice, AVCaptureDeviceInput, AVCaptureSession, AVCaptureVideoPreviewLayer,
    AVLayerVideoGravityResizeAspectFill,
};
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};

const SIZE: f64 = 288.0;
const CORNER: f64 = 24.0;

struct Preview {
    panel: Retained<NSPanel>,
    session: Retained<AVCaptureSession>,
    device: String,
}

thread_local! {
    static CURRENT: RefCell<Option<Preview>> = const { RefCell::new(None) };
}

fn make(device_id: &str) -> Option<Preview> {
    let main_thread = MainThreadMarker::new()?;
    // SAFETY: documented AVFoundation and AppKit calls on the main thread, on objects made here. The
    // panel isn't released when closed, and is kept alive by `Preview` while it's showing.
    unsafe {
        let device = AVCaptureDevice::deviceWithUniqueID(&NSString::from_str(device_id))?;
        let input = AVCaptureDeviceInput::deviceInputWithDevice_error(&device).ok()?;
        let session = AVCaptureSession::new();
        session.beginConfiguration();
        if !session.canAddInput(&input) {
            return None;
        }
        session.addInput(&input);
        session.commitConfiguration();

        let rect = NSRect::new(NSPoint::new(80.0, 120.0), NSSize::new(SIZE, SIZE));
        let panel = NSPanel::initWithContentRect_styleMask_backing_defer(
            main_thread.alloc(),
            rect,
            NSWindowStyleMask::Borderless | NSWindowStyleMask::NonactivatingPanel,
            NSBackingStoreType::Buffered,
            false,
        );
        panel.setReleasedWhenClosed(false);
        panel.setOpaque(false);
        panel.setBackgroundColor(Some(&NSColor::clearColor()));
        panel.setHasShadow(true);
        panel.setMovableByWindowBackground(true);
        panel.setLevel(NSFloatingWindowLevel);
        panel.setCollectionBehavior(
            NSWindowCollectionBehavior::CanJoinAllSpaces
                | NSWindowCollectionBehavior::FullScreenAuxiliary,
        );
        panel.setSharingType(NSWindowSharingType::None);

        let view = panel.contentView()?;
        view.setWantsLayer(true);
        let host = view.layer()?;
        host.setCornerRadius(CORNER);
        host.setMasksToBounds(true);
        let layer = AVCaptureVideoPreviewLayer::layerWithSession(&session);
        if let Some(gravity) = AVLayerVideoGravityResizeAspectFill {
            layer.setVideoGravity(gravity);
        }
        layer.setFrame(view.bounds());
        if let Some(connection) = layer.connection() {
            connection.setAutomaticallyAdjustsVideoMirroring(false);
            connection.setVideoMirrored(true);
        }
        host.addSublayer(&layer);

        session.startRunning();
        panel.orderFrontRegardless();
        Some(Preview {
            panel,
            session,
            device: device_id.to_string(),
        })
    }
}

/// Shows the preview for a camera, replacing any other. Only call this once the camera is allowed.
pub fn show(device_id: &str) {
    CURRENT.with_borrow_mut(|current| {
        if current
            .as_ref()
            .is_some_and(|preview| preview.device == device_id)
        {
            return;
        }
        if let Some(old) = current.take() {
            close(&old);
        }
        *current = make(device_id);
    });
}

fn close(preview: &Preview) {
    // SAFETY: documented calls on objects `Preview` owns, on the thread that made them.
    unsafe {
        preview.session.stopRunning();
        preview.panel.orderOut(None);
    }
}

pub fn hide() {
    CURRENT.with_borrow_mut(|current| {
        if let Some(old) = current.take() {
            close(&old);
        }
    });
}
