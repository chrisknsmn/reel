//! Following the cursor: where it is, what shape it is, and when it clicks (§28).
//!
//! Position is sampled every 33 ms on a timer that corrects for drift. Clicks come from a listen-only
//! event tap, which can't change or swallow anything. Shape comes from comparing the system's current
//! cursor with the standard ones.

use std::ffi::c_void;
use std::ptr::NonNull;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use objc2::rc::Retained;
use objc2_app_kit::{NSCursor, NSImage};
use objc2_core_foundation::{
    CFMachPort, CFRetained, CFRunLoop, CGPoint, CGRect, CGSize, kCFRunLoopCommonModes,
};
use objc2_core_graphics::{
    CGBitmapContextCreate, CGContext, CGDisplayBounds, CGEvent, CGEventMask, CGEventTapLocation,
    CGEventTapOptions, CGEventTapPlacement, CGEventTapProxy, CGEventType, CGImageAlphaInfo,
};
use reel_core::time::Micros;
use reel_media::macos::record::host_now;
use reel_motion::cursor::{CursorShape, Interaction};

use super::recorder::Recorder;
use super::windows::window_bounds;
use crate::config::SourceChoice;
use crate::cursor_log::{Bounds, Button, ButtonEvent, ClickTracker};
use crate::cursor_shape::{SAMPLE_SIZE, Signature, classify};

const SAMPLE_EVERY: Micros = Micros(33_000);
const SHAPE_EVERY: Duration = Duration::from_millis(50);
const WINDOW_EVERY: Duration = Duration::from_millis(250);

fn shape_code(shape: CursorShape) -> u8 {
    shape as u8
}

fn shape_from(code: u8) -> CursorShape {
    const ALL: [CursorShape; 9] = [
        CursorShape::Arrow,
        CursorShape::Text,
        CursorShape::Pointer,
        CursorShape::Crosshair,
        CursorShape::OpenHand,
        CursorShape::ClosedHand,
        CursorShape::ResizeEw,
        CursorShape::ResizeNs,
        CursorShape::NotAllowed,
    ];
    ALL.iter()
        .copied()
        .find(|shape| shape_code(*shape) == code)
        .unwrap_or(CursorShape::Arrow)
}

/// Reduces a cursor to its signature, by drawing it into a 32×32 mask.
#[must_use]
pub fn signature_of(cursor: &NSCursor) -> Option<Signature> {
    let image: Retained<NSImage> = cursor.image();
    // SAFETY: the context is made over a buffer of exactly the size given, which outlives it. The image
    // comes from a live NSImage, and is only drawn from.
    unsafe {
        let source = image.size();
        if source.width <= 0.0 || source.height <= 0.0 {
            return None;
        }
        let cg_image =
            image.CGImageForProposedRect_context_hints(std::ptr::null_mut(), None, None)?;
        let mut alpha = vec![0_u8; SAMPLE_SIZE * SAMPLE_SIZE];
        let context = CGBitmapContextCreate(
            alpha.as_mut_ptr().cast::<c_void>(),
            SAMPLE_SIZE,
            SAMPLE_SIZE,
            8,
            SAMPLE_SIZE,
            None,
            CGImageAlphaInfo::Only.0,
        )?;
        let size = SAMPLE_SIZE as f64;
        let scale = (size / source.width).min(size / source.height);
        let (width, height) = (source.width * scale, source.height * scale);
        CGContext::draw_image(
            Some(&context),
            CGRect {
                origin: CGPoint {
                    x: (size - width) / 2.0,
                    y: (size - height) / 2.0,
                },
                size: CGSize { width, height },
            },
            Some(&cg_image),
        );
        drop(context);
        let hotspot = cursor.hotSpot();
        Some(Signature {
            aspect: source.width / source.height.max(1.0),
            hotspot_x: hotspot.x / source.width.max(1.0),
            hotspot_y: hotspot.y / source.height.max(1.0),
            samples: alpha
                .iter()
                .map(|value| if *value > 24 { 255 } else { 0 })
                .collect(),
        })
    }
}

/// The standard cursors, to compare against.
#[must_use]
#[allow(deprecated)] // Apps still show these older cursors, so they still need recognising.
pub fn known_signatures() -> Vec<(CursorShape, Signature)> {
    let candidates = [
        (CursorShape::Arrow, NSCursor::arrowCursor()),
        (CursorShape::Text, NSCursor::IBeamCursor()),
        (CursorShape::Text, NSCursor::IBeamCursorForVerticalLayout()),
        (CursorShape::Pointer, NSCursor::pointingHandCursor()),
        (CursorShape::Crosshair, NSCursor::crosshairCursor()),
        (CursorShape::OpenHand, NSCursor::openHandCursor()),
        (CursorShape::ClosedHand, NSCursor::closedHandCursor()),
        (CursorShape::ResizeEw, NSCursor::resizeLeftCursor()),
        (CursorShape::ResizeEw, NSCursor::resizeRightCursor()),
        (CursorShape::ResizeEw, NSCursor::resizeLeftRightCursor()),
        (CursorShape::ResizeNs, NSCursor::resizeUpCursor()),
        (CursorShape::ResizeNs, NSCursor::resizeDownCursor()),
        (CursorShape::ResizeNs, NSCursor::resizeUpDownCursor()),
        (
            CursorShape::NotAllowed,
            NSCursor::operationNotAllowedCursor(),
        ),
    ];
    candidates
        .iter()
        .filter_map(|(shape, cursor)| Some((*shape, signature_of(cursor)?)))
        .collect()
}

/// Which cursor the system is showing now, in any app.
#[must_use]
#[allow(deprecated)]
pub fn current_shape(known: &[(CursorShape, Signature)]) -> CursorShape {
    let cursor = NSCursor::currentSystemCursor().unwrap_or_else(NSCursor::currentCursor);
    signature_of(&cursor).map_or(CursorShape::Arrow, |signature| classify(&signature, known))
}

/// Where the cursor is, in global points. This needs no permission.
#[must_use]
pub fn position() -> Option<(f64, f64)> {
    let event = CGEvent::new(None)?;
    let at = CGEvent::location(Some(&event));
    Some((at.x, at.y))
}

fn bounds_of(source: &SourceChoice, size: (u32, u32)) -> Bounds {
    match source {
        SourceChoice::Display { id } => {
            let rect = CGDisplayBounds(*id);
            Bounds {
                x: rect.origin.x,
                y: rect.origin.y,
                width: rect.size.width,
                height: rect.size.height,
            }
        }
        SourceChoice::Window { id } => window_bounds(*id).unwrap_or(Bounds {
            x: 0.0,
            y: 0.0,
            width: f64::from(size.0),
            height: f64::from(size.1),
        }),
        // A test pattern has no place on screen, so measure against the main display.
        SourceChoice::Synthetic { .. } => {
            let rect = CGDisplayBounds(objc2_core_graphics::CGMainDisplayID());
            Bounds {
                x: rect.origin.x,
                y: rect.origin.y,
                width: rect.size.width,
                height: rect.size.height,
            }
        }
    }
}

struct TapContext {
    recorder: Arc<Recorder>,
    bounds: Arc<Mutex<Bounds>>,
    shape: Arc<AtomicU8>,
    clicks: Mutex<ClickTracker>,
    port: Mutex<Option<CFRetained<CFMachPort>>>,
}

// SAFETY: everything in here is either thread-safe or behind a lock, and a mach port is only used to
// turn its tap back on, which CoreGraphics allows from the tap's own callback.
unsafe impl Send for TapContext {}
unsafe impl Sync for TapContext {}

unsafe extern "C-unwind" fn on_event(
    _proxy: CGEventTapProxy,
    kind: CGEventType,
    event: NonNull<CGEvent>,
    info: *mut c_void,
) -> *mut CGEvent {
    // SAFETY: `info` is the context that `Tap::start` leaked for the life of the process, and the event
    // is valid for the length of this call.
    let context = unsafe { &*info.cast::<TapContext>() };
    let event_ref = unsafe { event.as_ref() };

    // The system turns a tap off if it thinks it's slow. Turn it straight back on.
    if kind == CGEventType::TapDisabledByTimeout || kind == CGEventType::TapDisabledByUserInput {
        if let Ok(port) = context.port.lock()
            && let Some(port) = port.as_ref()
        {
            CGEvent::tap_enable(port, true);
        }
        return event.as_ptr();
    }
    let button = match kind {
        CGEventType::LeftMouseDown => Some(ButtonEvent::Down(Button::Left)),
        CGEventType::RightMouseDown => Some(ButtonEvent::Down(Button::Right)),
        CGEventType::OtherMouseDown => Some(ButtonEvent::Down(Button::Middle)),
        CGEventType::LeftMouseUp | CGEventType::RightMouseUp | CGEventType::OtherMouseUp => {
            Some(ButtonEvent::Up)
        }
        _ => None,
    };
    if let Some(button) = button {
        let host = host_now();
        let at = CGEvent::location(Some(event_ref));
        let bounds = context.bounds.lock().map(|bounds| *bounds);
        if let (Ok(bounds), Some(time)) = (bounds, context.recorder.place(host)) {
            let (x, y) = bounds.normalise(at.x, at.y);
            let interaction = context
                .clicks
                .lock()
                .map_or(Interaction::Click, |mut clicks| {
                    clicks.classify(button, time, x, y)
                });
            let shape = shape_from(context.shape.load(Ordering::Relaxed));
            context.recorder.on_cursor(x, y, interaction, shape, host);
        }
    }
    event.as_ptr()
}

const fn mask(kind: CGEventType) -> CGEventMask {
    1 << kind.0
}

/// Listens for clicks on its own thread, with its own run loop.
struct Tap {
    run_loop: CFRetained<CFRunLoop>,
    thread: JoinHandle<()>,
}

// SAFETY: a CFRunLoop can be stopped from any thread, which is all that's done with it from elsewhere.
unsafe impl Send for Tap {}

struct RunLoopRef(CFRetained<CFRunLoop>);

// SAFETY: only used to hand the run loop back to the thread that started the tap.
unsafe impl Send for RunLoopRef {}

impl Tap {
    fn start(context: TapContext) -> Option<Self> {
        let (send, receive) = std::sync::mpsc::channel();
        let thread = std::thread::Builder::new()
            .name("reel-clicks".into())
            .spawn(move || {
                let context: &'static TapContext = Box::leak(Box::new(context));
                let events = mask(CGEventType::LeftMouseDown)
                    | mask(CGEventType::RightMouseDown)
                    | mask(CGEventType::OtherMouseDown)
                    | mask(CGEventType::LeftMouseUp)
                    | mask(CGEventType::RightMouseUp)
                    | mask(CGEventType::OtherMouseUp);
                // SAFETY: the callback's signature is the documented one, and its context is leaked, so
                // it outlives the tap.
                let port = unsafe {
                    CGEvent::tap_create(
                        CGEventTapLocation::SessionEventTap,
                        CGEventTapPlacement::HeadInsertEventTap,
                        CGEventTapOptions::ListenOnly,
                        events,
                        Some(on_event),
                        std::ptr::from_ref(context).cast_mut().cast::<c_void>(),
                    )
                };
                let Some(port) = port else {
                    let _ = send.send(None);
                    return;
                };
                let source = CFMachPort::new_run_loop_source(None, Some(&port), 0);
                let run_loop = CFRunLoop::current();
                if let (Some(source), Some(run_loop)) = (source, run_loop) {
                    // SAFETY: this constant always exists.
                    let mode = unsafe { kCFRunLoopCommonModes };
                    run_loop.add_source(Some(&source), mode);
                    CGEvent::tap_enable(&port, true);
                    if let Ok(mut slot) = context.port.lock() {
                        *slot = Some(port);
                    }
                    let _ = send.send(Some(RunLoopRef(run_loop)));
                    CFRunLoop::run();
                } else {
                    let _ = send.send(None);
                }
            })
            .ok()?;
        let run_loop = receive.recv().ok().flatten()?;
        Some(Self {
            run_loop: run_loop.0,
            thread,
        })
    }

    fn stop(self) {
        self.run_loop.stop();
        let _ = self.thread.join();
    }
}

/// Keeps a recording's cursor data coming.
pub struct CursorTracker {
    stop: Arc<AtomicBool>,
    sampler: JoinHandle<()>,
    window_poll: Option<JoinHandle<()>>,
    tap: Option<Tap>,
    shape: Arc<AtomicU8>,
    known: Vec<(CursorShape, Signature)>,
}

impl CursorTracker {
    /// Starts following the cursor. Without a click listener, it still records where the cursor goes,
    /// and says so.
    #[must_use]
    pub fn start(
        recorder: Arc<Recorder>,
        source: &SourceChoice,
        size: (u32, u32),
        clicks: bool,
    ) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let bounds = Arc::new(Mutex::new(bounds_of(source, size)));
        let shape = Arc::new(AtomicU8::new(shape_code(CursorShape::Arrow)));

        let tap = if clicks {
            let tap = Tap::start(TapContext {
                recorder: recorder.clone(),
                bounds: bounds.clone(),
                shape: shape.clone(),
                clicks: Mutex::new(ClickTracker::default()),
                port: Mutex::new(None),
            });
            if tap.is_none() {
                recorder.warn(
                    "Clicks won't be recorded, because macOS didn't allow listening for them. The cursor's movement still is."
                        .to_string(),
                );
            }
            tap
        } else {
            None
        };

        let window_poll = if let SourceChoice::Window { id } = source {
            let (id, stop, bounds) = (*id, stop.clone(), bounds.clone());
            Some(std::thread::spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    if let Some(now) = window_bounds(id)
                        && let Ok(mut slot) = bounds.lock()
                    {
                        *slot = now;
                    }
                    std::thread::sleep(WINDOW_EVERY);
                }
            }))
        } else {
            None
        };

        let sampler = {
            let (stop, shape) = (stop.clone(), shape.clone());
            std::thread::spawn(move || {
                let mut next = host_now();
                while !stop.load(Ordering::Relaxed) {
                    if let Some((x, y)) = position()
                        && let Ok(area) = bounds.lock().map(|bounds| *bounds)
                    {
                        let (cx, cy) = area.normalise(x, y);
                        let current = shape_from(shape.load(Ordering::Relaxed));
                        recorder.on_cursor(cx, cy, Interaction::Move, current, host_now());
                    }
                    // From the last due time rather than from now, so that it doesn't drift.
                    next = Micros(next.0 + SAMPLE_EVERY.0);
                    let wait = next.0 - host_now().0;
                    if wait > 0 {
                        std::thread::sleep(Duration::from_micros(wait as u64));
                    }
                }
            })
        };
        Self {
            stop,
            sampler,
            window_poll,
            tap,
            shape,
            known: known_signatures(),
        }
    }

    /// Updates the shape. AppKit's cursor calls belong on the main thread, so its loop calls this.
    pub fn poll_shape(&self) {
        let current = current_shape(&self.known);
        self.shape.store(shape_code(current), Ordering::Relaxed);
    }

    pub fn stop(self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = self.sampler.join();
        if let Some(poll) = self.window_poll {
            let _ = poll.join();
        }
        if let Some(tap) = self.tap {
            tap.stop();
        }
    }
}

/// How often the main loop should call [`CursorTracker::poll_shape`].
pub const POLL_SHAPE_EVERY: Duration = SHAPE_EVERY;
