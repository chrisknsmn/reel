//! AppKit calls the app's windows need, with the approach T3 proved out (`docs/spikes.md`).

use std::ffi::CStr;

use dioxus::desktop::tao::platform::macos::WindowExtMacOS;
use dioxus::desktop::tao::window::Window;
use objc2::ffi::{class_replaceMethod, objc_getClass};
use objc2::runtime::{AnyObject, Sel};
use objc2::sel;
use objc2_app_kit::{NSAutoresizingMaskOptions, NSWindow, NSWindowSharingType};
use objc2_foundation::{NSPoint, NSRect, NSSize};

pub fn ns_window(window: &Window) -> &NSWindow {
    // SAFETY: tao returns the live NSWindow it owns, and `window` keeps it alive for this borrow.
    unsafe { &*window.ns_window().cast::<NSWindow>() }
}

/// Lets the click that activates a window also count, so the bar's controls work on the first click
/// when another app is in front. Dioxus doesn't expose wry's option for this.
pub fn accept_first_mouse() {
    extern "C-unwind" fn always(
        _this: &AnyObject,
        _cmd: Sel,
        _event: *mut AnyObject,
    ) -> objc2::runtime::Bool {
        objc2::runtime::Bool::YES
    }
    // SAFETY: `acceptsFirstMouse:` takes an NSEvent and returns a BOOL, matching both the function
    // above and the type encoding here. A webview exists when this runs, so the class is registered.
    unsafe {
        let class = objc_getClass(c"WryWebView".as_ptr());
        if class.is_null() {
            return;
        }
        let types: &CStr = c"B@:@";
        let function: unsafe extern "C-unwind" fn() = std::mem::transmute(
            always as extern "C-unwind" fn(&AnyObject, Sel, *mut AnyObject) -> objc2::runtime::Bool,
        );
        class_replaceMethod(
            class.cast_mut(),
            sel!(acceptsFirstMouse:),
            function,
            types.as_ptr(),
        );
    }
}

/// Fixes the webview at its largest size, anchored to the window's bottom edge. The window then only
/// reveals more or less of a page that never changes size, so WebKit has nothing to lay out and the
/// bar can't jump (T3).
pub fn pin_webview(window: &Window, width: f64, height: f64) {
    let window = ns_window(window);
    let Some(content) = window.contentView() else {
        return;
    };
    let subviews = content.subviews();
    let Some(webview) = subviews.firstObject() else {
        return;
    };
    webview.setAutoresizingMask(NSAutoresizingMaskOptions::empty());
    webview.setFrame(NSRect::new(
        NSPoint::new(0.0, 0.0),
        NSSize::new(width, height),
    ));
}

/// Grows or shrinks a window upward: an NSWindow's origin is its bottom-left corner, so keeping the
/// origin keeps the bar where it is.
pub fn set_height(window: &Window, height: f64) {
    let window = ns_window(window);
    let frame = window.frame();
    window.setFrame_display(
        NSRect::new(frame.origin, NSSize::new(frame.size.width, height)),
        true,
    );
}

/// The window's height, in points.
pub fn height(window: &Window) -> f64 {
    ns_window(window).frame().size.height
}

/// Its bottom-left corner, in AppKit's coordinates.
pub fn origin(window: &Window) -> (f64, f64) {
    let frame = ns_window(window).frame();
    (frame.origin.x, frame.origin.y)
}

/// Puts a window's bottom-left corner somewhere, if that's still on a connected screen. A saved place
/// can be left over from a monitor that's gone, and a bar off screen couldn't be got back.
pub fn place_at(window: &Window, x: f64, y: f64) -> bool {
    let Some(main_thread) = objc2::MainThreadMarker::new() else {
        return false;
    };
    let window = ns_window(window);
    let size = window.frame().size;
    let visible = objc2_app_kit::NSScreen::screens(main_thread)
        .iter()
        .any(|screen| {
            let area = screen.visibleFrame();
            x + size.width / 2.0 >= area.origin.x
                && x + size.width / 2.0 <= area.origin.x + area.size.width
                && y >= area.origin.y
                && y <= area.origin.y + area.size.height
        });
    if visible {
        window.setFrameOrigin(NSPoint::new(x, y));
    }
    visible
}

/// Puts a window at the bottom centre of the screen it's on, above the Dock.
pub fn place_bottom_center(window: &Window, margin: f64) {
    let window = ns_window(window);
    let Some(screen) = window.screen() else {
        return;
    };
    let area = screen.visibleFrame();
    let frame = window.frame();
    window.setFrameOrigin(NSPoint::new(
        area.origin.x + (area.size.width - frame.size.width) / 2.0,
        area.origin.y + margin,
    ));
}

/// Centres a window on the screen it's on.
pub fn center(window: &Window) {
    ns_window(window).center();
}

/// Sizes the editor at 85% of the work area and centres it (§47).
pub fn size_as_editor(window: &Window) {
    let window = ns_window(window);
    let Some(screen) = window.screen() else {
        return;
    };
    let area = screen.visibleFrame();
    let size = NSSize::new(area.size.width * 0.85, area.size.height * 0.85);
    window.setContentSize(size);
    window.center();
}

/// Marks a window as not to be captured. The display filter leaves out Reel's windows too. This is the
/// second line of defence for capture tools that don't use that filter.
pub fn set_capture_protected(window: &Window, protected: bool) {
    ns_window(window).setSharingType(if protected {
        NSWindowSharingType::None
    } else {
        NSWindowSharingType::ReadOnly
    });
}

/// Keeps a window above full-screen apps and on every Space, as the countdown has to be.
pub fn float_over_everything(window: &Window) {
    use objc2_app_kit::{NSScreenSaverWindowLevel, NSWindowCollectionBehavior};
    let window = ns_window(window);
    window.setLevel(NSScreenSaverWindowLevel);
    window.setCollectionBehavior(
        NSWindowCollectionBehavior::CanJoinAllSpaces
            | NSWindowCollectionBehavior::FullScreenAuxiliary,
    );
}

/// Brings the app forward with this window on top.
pub fn bring_forward(window: &Window) {
    if let Some(main_thread) = objc2::MainThreadMarker::new() {
        #[allow(deprecated)]
        objc2_app_kit::NSApplication::sharedApplication(main_thread)
            .activateIgnoringOtherApps(true);
    }
    ns_window(window).makeKeyAndOrderFront(None);
}
