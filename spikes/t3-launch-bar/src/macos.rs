//! AppKit helpers for the launch bar and its self-test.

use std::ffi::{CStr, c_void};
use std::future::Future;
use std::path::Path;

use dioxus::desktop::tao::platform::macos::WindowExtMacOS;
use dioxus::desktop::tao::window::Window;
use objc2::MainThreadMarker;
use objc2::ffi::{class_replaceMethod, objc_getClass};
use objc2::runtime::{AnyObject, Sel};
use objc2::sel;
use objc2_app_kit::{
    NSApplication, NSAutoresizingMaskOptions, NSEvent, NSEventModifierFlags, NSEventType, NSWindow,
};
use objc2_foundation::{NSPoint, NSProcessInfo, NSRect, NSSize};

fn ns_window(window: &Window) -> &NSWindow {
    // SAFETY: tao returns the live NSWindow it owns, and `window` keeps it alive for this borrow.
    unsafe { &*window.ns_window().cast::<NSWindow>() }
}

/// wry can accept the click that activates a window, but Dioxus doesn't expose that option. wry
/// asks `WryWebView` through this method, so make the method always answer yes.
pub fn accept_first_mouse() {
    extern "C-unwind" fn always(
        _this: &AnyObject,
        _cmd: Sel,
        _event: *mut AnyObject,
    ) -> objc2::runtime::Bool {
        objc2::runtime::Bool::YES
    }
    // SAFETY: `acceptsFirstMouse:` takes an NSEvent and returns a BOOL, matching both the function
    // above and the type encoding given here. The class name comes from wry's own class definition,
    // and a webview already exists when this runs, so the class is registered.
    unsafe {
        let class = objc_getClass(c"WryWebView".as_ptr());
        assert!(!class.is_null(), "wry's webview class has moved");
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

/// Fixes the webview at its largest size, anchored to the window's bottom edge. The window then
/// only reveals more or less of a page that never changes size, so WebKit has nothing to lay out
/// and the bar can't jump. Left to resize with the window, it visibly does: until the new layout
/// arrives, WebKit keeps the old content at the top of the taller view.
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

/// Grows or shrinks the window upward: an NSWindow's origin is its bottom-left corner, so keeping
/// the origin keeps the bar where it is. `setFrame` applies the whole change at once.
pub fn set_height(window: &Window, height: f64) {
    let window = ns_window(window);
    let frame = window.frame();
    window.setFrame_display(
        NSRect::new(frame.origin, NSSize::new(frame.size.width, height)),
        true,
    );
}

pub fn window_frame(window: &Window) -> (f64, f64, f64, f64) {
    let frame = ns_window(window).frame();
    (
        frame.origin.x,
        frame.origin.y,
        frame.size.width,
        frame.size.height,
    )
}

pub fn window_number(window: &Window) -> isize {
    ns_window(window).windowNumber()
}

pub fn is_key(window: &Window) -> bool {
    ns_window(window).isKeyWindow()
}

pub fn make_key(window: &Window) {
    if let Some(main_thread) = MainThreadMarker::new() {
        #[allow(deprecated)]
        NSApplication::sharedApplication(main_thread).activateIgnoringOtherApps(true);
    }
    ns_window(window).makeKeyAndOrderFront(None);
}

pub fn deactivate_app() {
    if let Some(main_thread) = MainThreadMarker::new() {
        NSApplication::sharedApplication(main_thread).deactivate();
    }
}

/// CSS pixels to window coordinates. The page's height never changes, because of `pin_webview`.
fn to_window_point(css_x: f64, css_y: f64) -> NSPoint {
    NSPoint::new(css_x, crate::OPEN_HEIGHT - css_y)
}

pub fn hit_test(window: &Window, css_x: f64, css_y: f64) -> String {
    let window = ns_window(window);
    let point = to_window_point(css_x, css_y);
    window
        .contentView()
        .and_then(|view| view.hitTest(point))
        .map_or_else(
            || "nothing".to_string(),
            |view| {
                let object: &AnyObject = view.as_ref();
                object.class().name().to_string_lossy().into_owned()
            },
        )
}

fn send_mouse(window: &Window, css_x: f64, css_y: f64, down: bool) {
    let window = ns_window(window);
    let kind = if down {
        NSEventType::LeftMouseDown
    } else {
        NSEventType::LeftMouseUp
    };
    let event = NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(
        kind,
        to_window_point(css_x, css_y),
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

pub async fn click(window: &Window, x: f64, y: f64, gap: impl Future<Output = ()>) {
    send_mouse(window, x, y, true);
    gap.await;
    send_mouse(window, x, y, false);
}

const WINDOW_LIST_INCLUDING_WINDOW: u32 = 1 << 3;
const IMAGE_BOUNDS_IGNORE_FRAMING: u32 = 1 << 0;
const IMAGE_BEST_RESOLUTION: u32 = 1 << 3;
const BYTE_ORDER_MASK: u32 = 0x7000;
const BYTE_ORDER_32_LITTLE: u32 = 2 << 12;

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
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

pub struct Capture {
    pub width: usize,
    pub height: usize,
    pub rgba: Vec<u8>,
}

impl Capture {
    pub fn pixel(&self, x: usize, y: usize) -> [u8; 4] {
        let at = (y * self.width + x) * 4;
        [
            self.rgba[at],
            self.rgba[at + 1],
            self.rgba[at + 2],
            self.rgba[at + 3],
        ]
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
                            rgba.extend_from_slice(&[pixel[2], pixel[1], pixel[0], pixel[3]]);
                        } else {
                            rgba.extend_from_slice(pixel);
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
