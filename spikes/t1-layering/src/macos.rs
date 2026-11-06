//! The macOS parts of the self-test: sending clicks through AppKit, and capturing the window.

use std::ffi::c_void;
use std::path::Path;

use dioxus::desktop::tao::platform::macos::WindowExtMacOS;
use dioxus::desktop::tao::window::Window;
use objc2::MainThreadMarker;
use objc2::runtime::AnyObject;
use objc2::{class, msg_send};
use objc2_app_kit::{
    NSApplication, NSEvent, NSEventModifierFlags, NSEventType, NSWindow, NSWindowOcclusionState,
};
use objc2_foundation::{NSArray, NSPoint, NSProcessInfo, NSRect, NSSize, NSString};

#[link(name = "QuartzCore", kind = "framework")]
unsafe extern "C" {
    static kCAGravityTopLeft: &'static NSString;
}

pub fn content_view(window: &Window) -> *mut c_void {
    window.ns_view()
}

/// By default Core Animation stretches a drawable to fill its layer, which until the next frame
/// arrives smears everything, including edges that aren't moving. Pinning it to the top left keeps
/// those edges exact, and the background colours whatever a growing window has just uncovered.
///
/// T1_GRAVITY=resize keeps the default, for comparison.
pub fn configure_metal_layer(content_view: *mut c_void) {
    if std::env::var("T1_GRAVITY").as_deref() == Ok("resize") {
        return;
    }
    let view = content_view.cast::<AnyObject>();
    // SAFETY: `view` is the window's live content view, these are the documented selectors with
    // their documented types, and this runs on the main thread during window setup.
    unsafe {
        let root: *mut AnyObject = msg_send![view, layer];
        let sublayers: *mut NSArray<AnyObject> = msg_send![root, sublayers];
        let Some(sublayers) = sublayers.as_ref() else {
            return;
        };
        for layer in sublayers.iter() {
            let is_metal: bool = msg_send![&*layer, isKindOfClass: class!(CAMetalLayer)];
            if is_metal {
                let _: () = msg_send![&*layer, setContentsGravity: kCAGravityTopLeft];
                // The same grey as the render pass's clear colour, once sRGB-encoded.
                let color = CGColorCreateSRGB(0.246, 0.246, 0.262, 1.0);
                let _: () = msg_send![&*layer, setBackgroundColor: color];
                CFRelease(color.cast_const().cast());
            }
        }
    }
}

fn ns_window(window: &Window) -> &NSWindow {
    // SAFETY: tao returns the live NSWindow it owns, and `window` keeps it alive for this borrow.
    unsafe { &*window.ns_window().cast::<NSWindow>() }
}

pub fn is_visible_and_key(window: &Window) -> (bool, bool) {
    let window = ns_window(window);
    (
        window
            .occlusionState()
            .contains(NSWindowOcclusionState::Visible),
        window.isKeyWindow(),
    )
}

/// AppKit swallows the first click on a window that isn't key, because it only activates the window.
/// The test window can lose key status if someone clicks elsewhere during a run, so reclaim it.
pub fn make_key(window: &Window) {
    if let Some(main_thread) = MainThreadMarker::new() {
        #[allow(deprecated)]
        NSApplication::sharedApplication(main_thread).activateIgnoringOtherApps(true);
    }
    ns_window(window).makeKeyAndOrderFront(None);
}

/// Hands focus to another app, to test what the first click on a background window does.
pub fn deactivate_app() {
    if let Some(main_thread) = MainThreadMarker::new() {
        NSApplication::sharedApplication(main_thread).deactivate();
    }
}

/// CSS pixels from the content view's top-left to AppKit's window coordinates, bottom-left origin.
fn to_window_point(window: &NSWindow, css_x: f64, css_y: f64) -> NSPoint {
    let height = window
        .contentView()
        .map_or(0.0, |view| view.frame().size.height);
    NSPoint::new(css_x, height - css_y)
}

/// Which view AppKit would route a click to at this point, by class name.
pub fn hit_test(window: &Window, css_x: f64, css_y: f64) -> String {
    let window = ns_window(window);
    let point = to_window_point(window, css_x, css_y);
    let hit = window.contentView().and_then(|view| view.hitTest(point));
    hit.map_or_else(
        || "nothing".to_string(),
        |view| {
            let object: &objc2::runtime::AnyObject = view.as_ref();
            object.class().name().to_string_lossy().into_owned()
        },
    )
}

/// Sends one half of a click through `-[NSWindow sendEvent:]`, the path real mouse events take from
/// here on: AppKit hit-tests the event and delivers it to whichever view is on top.
pub fn send_mouse(window: &Window, css_x: f64, css_y: f64, down: bool) {
    let window = ns_window(window);
    let kind = if down {
        NSEventType::LeftMouseDown
    } else {
        NSEventType::LeftMouseUp
    };
    let event = NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(
        kind,
        to_window_point(window, css_x, css_y),
        NSEventModifierFlags::empty(),
        NSProcessInfo::processInfo().systemUptime(),
        window.windowNumber(),
        None,
        0,
        1,
        if down { 1.0 } else { 0.0 },
    )
    .expect("AppKit refused to build a mouse event");
    window.sendEvent(&event);
}

pub fn window_number(window: &Window) -> isize {
    ns_window(window).windowNumber()
}

/// An address that can cross threads, for the one read-only property the drag thread needs.
pub fn window_address(window: &Window) -> usize {
    window.ns_window() as usize
}

pub fn in_live_resize(window_address: usize) -> bool {
    // SAFETY: the window outlives the self-test, and this only reads a BOOL property.
    unsafe { msg_send![window_address as *mut AnyObject, inLiveResize] }
}

/// Queues a mouse event as if it had come from the window server. Unlike `send_mouse`, this returns
/// straight away, so the main thread can fall into AppKit's modal resize loop as it would for a real
/// drag, and this can be called from another thread while it's in there.
pub fn post_mouse(window_number: isize, kind: NSEventType, x: f64, y: f64) {
    objc2::rc::autoreleasepool(|_| {
        let event = NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(
            kind,
            NSPoint::new(x, y),
            NSEventModifierFlags::empty(),
            NSProcessInfo::processInfo().systemUptime(),
            window_number,
            None,
            0,
            1,
            if kind == NSEventType::LeftMouseUp { 0.0 } else { 1.0 },
        )
        .expect("AppKit refused to build a mouse event");
        // SAFETY: `postEvent:atStart:` is documented as safe to call from any thread.
        unsafe {
            let app: *mut AnyObject = msg_send![class!(NSApplication), sharedApplication];
            let _: () = msg_send![app, postEvent: &*event, atStart: false];
        }
    });
}

const WINDOW_LIST_INCLUDING_WINDOW: u32 = 1 << 3;
const IMAGE_BOUNDS_IGNORE_FRAMING: u32 = 1 << 0;
const IMAGE_BEST_RESOLUTION: u32 = 1 << 3;
const BYTE_ORDER_MASK: u32 = 0x7000;
const BYTE_ORDER_32_LITTLE: u32 = 2 << 12;

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    // The SDK marks this obsolete from macOS 15 in favour of ScreenCaptureKit, but it's still
    // exported. An app can capture its own windows this way without Screen Recording permission.
    fn CGWindowListCreateImage(bounds: NSRect, list: u32, window: u32, image: u32) -> *mut c_void;
    fn CGImageGetWidth(image: *mut c_void) -> usize;
    fn CGImageGetHeight(image: *mut c_void) -> usize;
    fn CGImageGetBytesPerRow(image: *mut c_void) -> usize;
    fn CGImageGetBitsPerPixel(image: *mut c_void) -> usize;
    fn CGImageGetBitmapInfo(image: *mut c_void) -> u32;
    fn CGImageGetDataProvider(image: *mut c_void) -> *mut c_void;
    fn CGDataProviderCopyData(provider: *mut c_void) -> *const c_void;
    fn CGImageRelease(image: *mut c_void);
    fn CGPreflightScreenCaptureAccess() -> bool;
    fn CGColorCreateSRGB(red: f64, green: f64, blue: f64, alpha: f64) -> *mut AnyObject;
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFDataGetBytePtr(data: *const c_void) -> *const u8;
    fn CFDataGetLength(data: *const c_void) -> isize;
    fn CFRelease(object: *const c_void);
}

pub fn has_screen_recording_permission() -> bool {
    // SAFETY: takes no arguments and only reads the permission state, without prompting.
    unsafe { CGPreflightScreenCaptureAccess() }
}

/// A window image from the window server, so it holds what's really composited on screen.
pub struct Capture {
    pub width: usize,
    pub height: usize,
    pub rgba: Vec<u8>,
}

impl Capture {
    pub fn pixel(&self, x: usize, y: usize) -> [u8; 3] {
        let at = (y * self.width + x) * 4;
        [self.rgba[at], self.rgba[at + 1], self.rgba[at + 2]]
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let file = std::io::BufWriter::new(std::fs::File::create(path)?);
        let mut encoder = png::Encoder::new(file, self.width as u32, self.height as u32);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().map_err(std::io::Error::other)?;
        writer
            .write_image_data(&self.rgba)
            .map_err(std::io::Error::other)?;
        writer.finish().map_err(std::io::Error::other)
    }
}

pub fn capture(window: &Window) -> Option<Capture> {
    capture_number(ns_window(window).windowNumber())
}

/// The same as `capture`, for threads that can't hold on to the window itself.
pub fn capture_number(window_number: isize) -> Option<Capture> {
    let number = u32::try_from(window_number).ok()?;
    let null_rect = NSRect::new(
        NSPoint::new(f64::INFINITY, f64::INFINITY),
        NSSize::new(0.0, 0.0),
    );
    // SAFETY: each pointer is checked before use and comes straight from the call before it. The
    // image and data are released once, after their bytes have been copied out.
    unsafe {
        let image = CGWindowListCreateImage(
            null_rect,
            WINDOW_LIST_INCLUDING_WINDOW,
            number,
            IMAGE_BOUNDS_IGNORE_FRAMING | IMAGE_BEST_RESOLUTION,
        );
        if image.is_null() {
            return None;
        }
        let width = CGImageGetWidth(image);
        let height = CGImageGetHeight(image);
        let stride = CGImageGetBytesPerRow(image);
        let bits = CGImageGetBitsPerPixel(image);
        let is_bgra = CGImageGetBitmapInfo(image) & BYTE_ORDER_MASK == BYTE_ORDER_32_LITTLE;
        let data = CGDataProviderCopyData(CGImageGetDataProvider(image));
        let mut rgba = Vec::new();
        if !data.is_null() {
            if bits == 32 && width > 0 && height > 0 {
                let bytes = std::slice::from_raw_parts(
                    CFDataGetBytePtr(data),
                    CFDataGetLength(data) as usize,
                );
                rgba.reserve_exact(width * height * 4);
                for row in bytes.chunks_exact(stride).take(height) {
                    for pixel in row[..width * 4].chunks_exact(4) {
                        if is_bgra {
                            rgba.extend_from_slice(&[pixel[2], pixel[1], pixel[0], 255]);
                        } else {
                            rgba.extend_from_slice(&[pixel[0], pixel[1], pixel[2], 255]);
                        }
                    }
                }
            }
            CFRelease(data);
        }
        CGImageRelease(image);
        (!rgba.is_empty()).then_some(Capture {
            width,
            height,
            rgba,
        })
    }
}
