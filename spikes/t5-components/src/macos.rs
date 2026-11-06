//! Real input through AppKit, and window capture, for the self-test.

use std::ffi::c_void;
use std::path::Path;

use dioxus::desktop::tao::platform::macos::WindowExtMacOS;
use dioxus::desktop::tao::window::Window;
use objc2::MainThreadMarker;
use objc2_app_kit::{NSApplication, NSEvent, NSEventModifierFlags, NSEventType, NSWindow};
use objc2_foundation::{NSPoint, NSProcessInfo, NSRect, NSSize, NSString};

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

pub fn is_key(window: &Window) -> bool {
    ns_window(window).isKeyWindow()
}

fn to_window_point(window: &NSWindow, css_x: f64, css_y: f64) -> NSPoint {
    let height = window
        .contentView()
        .map_or(0.0, |view| view.frame().size.height);
    NSPoint::new(css_x, height - css_y)
}

pub fn send_mouse(window: &Window, kind: NSEventType, css_x: f64, css_y: f64) {
    let window = ns_window(window);
    let event = NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(
        kind,
        to_window_point(window, css_x, css_y),
        NSEventModifierFlags::empty(),
        NSProcessInfo::processInfo().systemUptime(),
        window.windowNumber(),
        None,
        0,
        1,
        if matches!(kind, NSEventType::LeftMouseUp | NSEventType::RightMouseUp) { 0.0 } else { 1.0 },
    )
    .expect("AppKit refused to build a mouse event");
    window.sendEvent(&event);
}

/// Sends one key press through the window, to whatever holds keyboard focus.
pub fn send_key(window: &Window, characters: &str, key_code: u16) {
    let window = ns_window(window);
    let text = NSString::from_str(characters);
    for kind in [NSEventType::KeyDown, NSEventType::KeyUp] {
        // SAFETY: documented constructor, with valid strings and no graphics context.
        let event = unsafe {
            NSEvent::keyEventWithType_location_modifierFlags_timestamp_windowNumber_context_characters_charactersIgnoringModifiers_isARepeat_keyCode(
                kind,
                NSPoint::new(0.0, 0.0),
                NSEventModifierFlags::empty(),
                NSProcessInfo::processInfo().systemUptime(),
                window.windowNumber(),
                None,
                &text,
                &text,
                false,
                key_code,
            )
        }
        .expect("AppKit refused to build a key event");
        window.sendEvent(&event);
    }
}

/// Hover goes straight to the webview, because a synthetic MouseMoved sent to the window doesn't
/// reach WebKit's tracking area.
pub fn hover(window: &Window, css_x: f64, css_y: f64) {
    let window = ns_window(window);
    let Some(content) = window.contentView() else {
        return;
    };
    let subviews = content.subviews();
    let Some(webview) = subviews.firstObject() else {
        return;
    };
    let event = NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(
        NSEventType::MouseMoved,
        to_window_point(window, css_x, css_y),
        NSEventModifierFlags::empty(),
        NSProcessInfo::processInfo().systemUptime(),
        window.windowNumber(),
        None,
        0,
        0,
        0.0,
    )
    .expect("AppKit refused to build a mouse event");
    // SAFETY: `mouseMoved:` is the documented NSResponder method, given a valid event.
    unsafe {
        let _: () = objc2::msg_send![&*webview, mouseMoved: &*event];
    }
}

pub fn describe(window: &Window) -> String {
    let window = ns_window(window);
    format!(
        "visible {} miniaturized {} key {} sharing {:?} frame {:?}",
        window.isVisible(),
        window.isMiniaturized(),
        window.isKeyWindow(),
        window.sharingType().0,
        window.frame().size,
    )
}

const WINDOW_LIST_INCLUDING_WINDOW: u32 = 1 << 3;
const IMAGE_BOUNDS_IGNORE_FRAMING: u32 = 1 << 0;
const BYTE_ORDER_MASK: u32 = 0x7000;
const BYTE_ORDER_32_LITTLE: u32 = 2 << 12;

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGWindowListCreateImage(bounds: NSRect, list: u32, window: u32, image: u32) -> *mut c_void;
    fn CGImageGetWidth(image: *mut c_void) -> usize;
    fn CGImageGetHeight(image: *mut c_void) -> usize;
    fn CGImageGetBytesPerRow(image: *mut c_void) -> usize;
    fn CGImageGetBitmapInfo(image: *mut c_void) -> u32;
    fn CGImageGetDataProvider(image: *mut c_void) -> *mut c_void;
    fn CGDataProviderCopyData(provider: *mut c_void) -> *const c_void;
    fn CGImageRelease(image: *mut c_void);
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFDataGetBytePtr(data: *const c_void) -> *const u8;
    fn CFDataGetLength(data: *const c_void) -> isize;
    fn CFRelease(object: *const c_void);
}

pub fn save_capture(window: &Window, path: &Path) {
    let number = ns_window(window).windowNumber() as u32;
    let null_rect = NSRect::new(
        NSPoint::new(f64::INFINITY, f64::INFINITY),
        NSSize::new(0.0, 0.0),
    );
    // SAFETY: each pointer is checked before use and comes straight from the call before it. The
    // image and data are released once, after their bytes have been copied out.
    let (width, height, rgba) = unsafe {
        let image = CGWindowListCreateImage(
            null_rect,
            WINDOW_LIST_INCLUDING_WINDOW,
            number,
            IMAGE_BOUNDS_IGNORE_FRAMING,
        );
        if image.is_null() {
            return;
        }
        let (width, height) = (CGImageGetWidth(image), CGImageGetHeight(image));
        let stride = CGImageGetBytesPerRow(image);
        let is_bgra = CGImageGetBitmapInfo(image) & BYTE_ORDER_MASK == BYTE_ORDER_32_LITTLE;
        let data = CGDataProviderCopyData(CGImageGetDataProvider(image));
        let bytes =
            std::slice::from_raw_parts(CFDataGetBytePtr(data), CFDataGetLength(data) as usize);
        let mut rgba = Vec::with_capacity(width * height * 4);
        for row in bytes.chunks_exact(stride).take(height) {
            for pixel in row[..width * 4].chunks_exact(4) {
                if is_bgra {
                    rgba.extend_from_slice(&[pixel[2], pixel[1], pixel[0], pixel[3]]);
                } else {
                    rgba.extend_from_slice(pixel);
                }
            }
        }
        CFRelease(data);
        CGImageRelease(image);
        let opaque = rgba.chunks_exact(4).filter(|p| p[3] > 200).count();
        println!(
            "capture {}: {width}x{height}, {:.0}% opaque",
            path.display(),
            100.0 * opaque as f64 / (width * height) as f64
        );
        (width, height, rgba)
    };
    if let Ok(file) = std::fs::File::create(path) {
        let mut encoder =
            png::Encoder::new(std::io::BufWriter::new(file), width as u32, height as u32);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        if let Ok(mut writer) = encoder.write_header() {
            let _ = writer.write_image_data(&rgba);
        }
    }
}
