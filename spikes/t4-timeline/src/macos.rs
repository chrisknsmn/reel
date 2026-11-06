//! Mouse events through AppKit for the self-test.

use dioxus::desktop::tao::platform::macos::WindowExtMacOS;
use dioxus::desktop::tao::window::Window;
use objc2::MainThreadMarker;
use objc2::runtime::AnyObject;
use objc2::{class, msg_send};
use objc2_app_kit::{NSApplication, NSEvent, NSEventModifierFlags, NSEventType, NSWindow};
use objc2_foundation::{NSPoint, NSProcessInfo};

fn ns_window(window: &Window) -> &NSWindow {
    // SAFETY: tao returns the live NSWindow it owns, and `window` keeps it alive for this borrow.
    unsafe { &*window.ns_window().cast::<NSWindow>() }
}

pub fn make_key(window: &Window) {
    if let Some(main_thread) = MainThreadMarker::new() {
        #[allow(deprecated)]
        NSApplication::sharedApplication(main_thread).activateIgnoringOtherApps(true);
    }
    ns_window(window).makeKeyAndOrderFront(None);
}

pub fn window_number(window: &Window) -> isize {
    ns_window(window).windowNumber()
}

#[derive(Clone, Copy)]
pub enum Kind {
    Down,
    Drag,
    Up,
}

/// Queues a mouse event as if it came from the window server. `y` counts up from the window's bottom.
pub fn post_mouse(window_number: isize, kind: Kind, x: f64, y: f64) {
    objc2::rc::autoreleasepool(|_| {
        let (kind, pressure) = match kind {
            Kind::Down => (NSEventType::LeftMouseDown, 1.0),
            Kind::Drag => (NSEventType::LeftMouseDragged, 1.0),
            Kind::Up => (NSEventType::LeftMouseUp, 0.0),
        };
        let event = NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(
            kind,
            NSPoint::new(x, y),
            NSEventModifierFlags::empty(),
            NSProcessInfo::processInfo().systemUptime(),
            window_number,
            None,
            0,
            1,
            pressure,
        )
        .expect("AppKit refused to build a mouse event");
        // SAFETY: `postEvent:atStart:` is documented as safe to call from any thread.
        unsafe {
            let app: *mut AnyObject = msg_send![class!(NSApplication), sharedApplication];
            let _: () = msg_send![app, postEvent: &*event, atStart: false];
        }
    });
}
