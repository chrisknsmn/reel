//! Listing and capturing the screen with ScreenCaptureKit (§27).

use std::cell::OnceCell;
use std::path::Path;
use std::sync::Arc;
use std::sync::mpsc;

use block2::RcBlock;
use dispatch2::{DispatchQueue, DispatchQueueAttr, DispatchRetained};
use objc2::rc::Retained;
use objc2::runtime::{NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{AnyThread, DefinedClass, define_class, msg_send};
use objc2_app_kit::{NSBitmapImageFileType, NSBitmapImageRep};
use objc2_core_foundation::CFRetained;
use objc2_core_graphics::{CGImage, CGMainDisplayID};
use objc2_core_media::{CMSampleBuffer, CMTime};
use objc2_core_video::{CVPixelBuffer, kCVPixelFormatType_32BGRA};
use objc2_foundation::{NSArray, NSDictionary, NSError, NSNumber, NSString};
use objc2_screen_capture_kit::{
    SCContentFilter, SCDisplay, SCFrameStatus, SCRunningApplication, SCScreenshotManager,
    SCShareableContent, SCStream, SCStreamConfiguration, SCStreamDelegate, SCStreamFrameInfoStatus,
    SCStreamOutput, SCStreamOutputType, SCWindow,
};
use reel_media::macos::record::micros_of;

use super::recorder::{Recorder, Track};
use crate::config::{RecordConfig, SourceChoice};
use crate::sources::{
    DisplaySource, WindowSource, display_name, even_size, is_reel, should_offer, window_name,
};

const QUEUE_DEPTH: isize = 6;
const THUMBNAIL_WIDTH: usize = 480;

fn shareable_content() -> Result<Retained<SCShareableContent>, String> {
    let (send, receive) = mpsc::channel();
    let handler = RcBlock::new(
        move |content: *mut SCShareableContent, error: *mut NSError| {
            // SAFETY: the system passes either a live object or null for each, retained for this call.
            let result = unsafe {
                match Retained::retain(content) {
                    Some(content) => Ok(content),
                    None => Err(error
                        .as_ref()
                        .map_or_else(|| "no reason given".to_string(), ToString::to_string)),
                }
            };
            let _ = send.send(result.map(Sendable));
        },
    );
    // SAFETY: a documented call, and the block outlives it because the system copies it.
    unsafe {
        SCShareableContent::getShareableContentExcludingDesktopWindows_onScreenWindowsOnly_completionHandler(
            true, true, &handler,
        );
    }
    receive
        .recv()
        .map_err(|_| "The system didn't answer.".to_string())?
        .map(|content| content.0)
        .map_err(|error| format!("What's on screen couldn't be listed: {error}"))
}

/// Carries an object across a channel from a system callback.
struct Sendable<T>(T);

// SAFETY: these are immutable objects from ScreenCaptureKit, handed once from a callback to the thread
// waiting for them.
unsafe impl<T> Send for Sendable<T> {}

fn own_string(text: &NSString) -> String {
    text.to_string()
}

/// What can be recorded. Needs the Screen Recording permission.
///
/// # Errors
/// Returns an error if the system won't say, which almost always means the permission is missing.
pub fn list(thumbnails: Option<&Path>) -> Result<(Vec<DisplaySource>, Vec<WindowSource>), String> {
    let content = shareable_content()?;
    let main = CGMainDisplayID();
    // SAFETY: documented property reads on objects returned by the system.
    let (displays, windows) = unsafe {
        let displays: Vec<DisplaySource> = content
            .displays()
            .iter()
            .enumerate()
            .map(|(index, display)| {
                let frame = display.frame();
                let primary = display.displayID() == main;
                DisplaySource {
                    id: display.displayID(),
                    name: display_name(index + 1, primary),
                    primary,
                    x: frame.origin.x,
                    y: frame.origin.y,
                    width: frame.size.width,
                    height: frame.size.height,
                    scale: display_scale(&display),
                    thumbnail: None,
                }
            })
            .collect();
        let windows: Vec<WindowSource> = content
            .windows()
            .iter()
            .map(|window| {
                let frame = window.frame();
                let application = window.owningApplication();
                let app = application
                    .as_ref()
                    .map_or_else(String::new, |app| own_string(&app.applicationName()));
                let bundle = application
                    .as_ref()
                    .map_or_else(String::new, |app| own_string(&app.bundleIdentifier()));
                let title = window
                    .title()
                    .map_or_else(String::new, |title| own_string(&title));
                WindowSource {
                    id: window.windowID(),
                    name: window_name(&app, &title),
                    app,
                    bundle,
                    title,
                    layer: window.windowLayer() as i64,
                    width: frame.size.width,
                    height: frame.size.height,
                    on_screen: window.isOnScreen(),
                    thumbnail: None,
                }
            })
            .filter(should_offer)
            .collect();
        (displays, windows)
    };

    let (mut displays, mut windows) = (displays, windows);
    if let Some(folder) = thumbnails {
        let _ = std::fs::create_dir_all(folder);
        for item in &mut displays {
            let path = folder.join(format!("display-{}.png", item.id));
            if let Some(filter) =
                filter_for(&content, &SourceChoice::Display { id: item.id }, &[], true).ok()
                && save_thumbnail(&filter, item.width / item.height.max(1.0), &path)
            {
                item.thumbnail = Some(path);
            }
        }
        for item in &mut windows {
            let path = folder.join(format!("window-{}.png", item.id));
            if let Some(filter) =
                filter_for(&content, &SourceChoice::Window { id: item.id }, &[], true).ok()
                && save_thumbnail(&filter, item.width / item.height.max(1.0), &path)
            {
                item.thumbnail = Some(path);
            }
        }
    }
    Ok((displays, windows))
}

fn display_scale(display: &SCDisplay) -> f64 {
    // SAFETY: a documented read, and the width comes from the same object.
    unsafe {
        let frame = display.frame();
        let mode = objc2_core_graphics::CGDisplayCopyDisplayMode(display.displayID());
        let pixels = mode.map_or(0, |mode| {
            objc2_core_graphics::CGDisplayMode::pixel_width(Some(&mode))
        });
        if frame.size.width > 0.0 && pixels > 0 {
            pixels as f64 / frame.size.width
        } else {
            1.0
        }
    }
}

fn filter_for(
    content: &SCShareableContent,
    source: &SourceChoice,
    exclude_bundles: &[String],
    hide_own_windows: bool,
) -> Result<Retained<SCContentFilter>, String> {
    // SAFETY: documented calls with objects returned by the system.
    unsafe {
        match source {
            SourceChoice::Display { id } => {
                let display = content
                    .displays()
                    .iter()
                    .find(|display| display.displayID() == *id)
                    .ok_or_else(|| "That display isn't connected any more.".to_string())?;
                // Reel's own windows are left out here, rather than relying on them being marked as
                // not to be captured (§47).
                let excluded: Vec<Retained<SCRunningApplication>> = content
                    .applications()
                    .iter()
                    .filter(|app| {
                        let bundle = own_string(&app.bundleIdentifier());
                        (hide_own_windows && is_reel(&bundle)) || exclude_bundles.contains(&bundle)
                    })
                    .collect();
                let excluded = NSArray::from_retained_slice(&excluded);
                let excepting: Retained<NSArray<SCWindow>> = NSArray::new();
                Ok(
                    SCContentFilter::initWithDisplay_excludingApplications_exceptingWindows(
                        SCContentFilter::alloc(),
                        &display,
                        &excluded,
                        &excepting,
                    ),
                )
            }
            SourceChoice::Window { id } => {
                let window = content
                    .windows()
                    .iter()
                    .find(|window| window.windowID() == *id)
                    .ok_or_else(|| "That window isn't open any more.".to_string())?;
                Ok(SCContentFilter::initWithDesktopIndependentWindow(
                    SCContentFilter::alloc(),
                    &window,
                ))
            }
            SourceChoice::Synthetic { .. } => Err("A test pattern has no filter.".to_string()),
        }
    }
}

fn save_thumbnail(filter: &SCContentFilter, aspect: f64, path: &Path) -> bool {
    let (send, receive) = mpsc::channel();
    let handler = RcBlock::new(move |image: *mut CGImage, _error: *mut NSError| {
        // SAFETY: the system passes either a live image or null, valid for this call.
        let image = unsafe { image.as_ref() }.map(|image| Sendable(CFRetained::from(image)));
        let _ = send.send(image);
    });
    // SAFETY: documented calls with objects made here, and the system copies the block.
    let image = unsafe {
        let config = SCStreamConfiguration::new();
        config.setWidth(THUMBNAIL_WIDTH);
        config.setHeight(((THUMBNAIL_WIDTH as f64 / aspect.max(0.01)) as usize).max(1));
        SCScreenshotManager::captureImageWithFilter_configuration_completionHandler(
            filter,
            &config,
            Some(&handler),
        );
        receive.recv().ok().flatten()
    };
    let Some(image) = image else { return false };
    // SAFETY: documented calls on a live image.
    unsafe {
        let representation = NSBitmapImageRep::initWithCGImage(NSBitmapImageRep::alloc(), &image.0);
        let properties: Retained<NSDictionary<_, _>> = NSDictionary::new();
        representation
            .representationUsingType_properties(NSBitmapImageFileType::PNG, &properties)
            .is_some_and(|data| std::fs::write(path, data.to_vec()).is_ok())
    }
}

/// What the stream's callbacks need.
struct OutputState {
    recorder: Arc<Recorder>,
    system_audio: bool,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[name = "ReelStreamOutput"]
    #[ivars = OnceCell<OutputState>]
    struct StreamOutput;

    unsafe impl NSObjectProtocol for StreamOutput {}

    unsafe impl SCStreamOutput for StreamOutput {
        #[unsafe(method(stream:didOutputSampleBuffer:ofType:))]
        unsafe fn did_output(
            &self,
            _stream: &SCStream,
            sample: &CMSampleBuffer,
            kind: SCStreamOutputType,
        ) {
            let Some(state) = self.ivars().get() else { return };
            // SAFETY: documented reads on a live sample buffer.
            let host = unsafe { micros_of(sample.presentation_time_stamp()) };
            if kind == SCStreamOutputType::Screen {
                if let Some(buffer) = complete_frame(sample) {
                    state.recorder.on_frame(&buffer, host);
                }
            } else if kind == SCStreamOutputType::Audio && state.system_audio {
                state.recorder.on_sound(Track::System, sample, host);
            }
        }
    }

    unsafe impl SCStreamDelegate for StreamOutput {
        #[unsafe(method(stream:didStopWithError:))]
        unsafe fn did_stop(&self, _stream: &SCStream, error: &NSError) {
            if let Some(state) = self.ivars().get() {
                state.recorder.source_ended(format!("Capture stopped: {error}"));
            }
        }
    }
);

impl StreamOutput {
    fn new(state: OutputState) -> Retained<Self> {
        let this = Self::alloc().set_ivars(OnceCell::new());
        // SAFETY: NSObject's init, called once on a fresh allocation.
        let this: Retained<Self> = unsafe { msg_send![super(this), init] };
        let _ = this.ivars().set(state);
        this
    }
}

/// The picture in a sample, if it's a complete new frame. Idle and partial updates have nothing to keep.
fn complete_frame(sample: &CMSampleBuffer) -> Option<CFRetained<CVPixelBuffer>> {
    // SAFETY: documented reads on a live sample buffer. The attachments array holds dictionaries that
    // are toll-free bridged to NSDictionary, as documented, and they're only read.
    unsafe {
        let attachments = sample.sample_attachments_array(false)?;
        let raw: *const objc2_core_foundation::CFArray = &*attachments;
        let array = &*raw.cast::<NSArray<NSDictionary<NSString, NSObject>>>();
        let info = array.firstObject()?;
        let status = info.objectForKey(SCStreamFrameInfoStatus)?;
        let status = status.downcast_ref::<NSNumber>()?;
        if status.integerValue() != SCFrameStatus::Complete.0 {
            return None;
        }
        sample.image_buffer()
    }
}

/// A screen capture in progress.
pub struct ScreenCapture {
    stream: Retained<SCStream>,
    _output: Retained<StreamOutput>,
    _video_queue: DispatchRetained<DispatchQueue>,
    _audio_queue: DispatchRetained<DispatchQueue>,
}

// SAFETY: starting and stopping a stream may be done from any thread.
unsafe impl Send for ScreenCapture {}

/// Works out the pixel size to record a source at.
///
/// # Errors
/// Returns an error if the source has gone, or the permission is missing.
pub fn size_of(source: &SourceChoice) -> Result<(u32, u32), String> {
    let content = shareable_content()?;
    let filter = filter_for(&content, source, &[], true)?;
    // SAFETY: documented reads on a filter made just above.
    let (rect, scale) = unsafe { (filter.contentRect(), f64::from(filter.pointPixelScale())) };
    Ok(even_size(rect.size.width * scale, rect.size.height * scale))
}

fn wait_for_start(
    start: impl FnOnce(&block2::DynBlock<dyn Fn(*mut NSError)>),
) -> Result<(), String> {
    let (send, receive) = mpsc::channel();
    let handler = RcBlock::new(move |error: *mut NSError| {
        // SAFETY: the system passes either a live error or null, valid for this call.
        let error = unsafe { error.as_ref() }.map(ToString::to_string);
        let _ = send.send(error);
    });
    start(&handler);
    match receive.recv() {
        Ok(None) => Ok(()),
        Ok(Some(error)) => Err(error),
        Err(_) => Err("The system didn't answer.".to_string()),
    }
}

impl ScreenCapture {
    /// Starts capturing. Frames and sound go to the recorder from here on.
    ///
    /// # Errors
    /// Returns why capture won't start.
    pub fn start(
        config: &RecordConfig,
        size: (u32, u32),
        recorder: Arc<Recorder>,
    ) -> Result<Self, String> {
        let content = shareable_content()?;
        let filter = filter_for(
            &content,
            &config.source,
            &config.exclude_bundles,
            config.hide_own_windows,
        )?;
        let output = StreamOutput::new(OutputState {
            recorder,
            system_audio: config.system_audio,
        });
        let attributes = DispatchQueueAttr::with_autorelease_frequency(
            None,
            dispatch2::DispatchAutoReleaseFrequency::WORK_ITEM,
        );
        let video_queue = DispatchQueue::new("dev.reel.capture.video", Some(&attributes));
        let audio_queue = DispatchQueue::new("dev.reel.capture.audio", Some(&attributes));
        // SAFETY: documented setup calls, in their documented order, on objects made here.
        unsafe {
            let settings = SCStreamConfiguration::new();
            settings.setWidth(size.0 as usize);
            settings.setHeight(size.1 as usize);
            settings.setMinimumFrameInterval(CMTime::new(1, config.fps.max(1) as i32));
            settings.setPixelFormat(kCVPixelFormatType_32BGRA);
            settings.setQueueDepth(QUEUE_DEPTH);
            // The cursor is drawn by the renderer, from the recorded data.
            settings.setShowsCursor(false);
            settings.setIgnoreShadowsSingleWindow(true);
            if config.system_audio {
                settings.setCapturesAudio(true);
                settings.setSampleRate(48_000);
                settings.setChannelCount(2);
                settings.setExcludesCurrentProcessAudio(true);
            }
            let delegate: &ProtocolObject<dyn SCStreamDelegate> =
                ProtocolObject::from_ref(&*output);
            let stream = SCStream::initWithFilter_configuration_delegate(
                SCStream::alloc(),
                &filter,
                &settings,
                Some(delegate),
            );
            let sink: &ProtocolObject<dyn SCStreamOutput> = ProtocolObject::from_ref(&*output);
            stream
                .addStreamOutput_type_sampleHandlerQueue_error(
                    sink,
                    SCStreamOutputType::Screen,
                    Some(&video_queue),
                )
                .map_err(|error| format!("Capture couldn't be set up: {error}"))?;
            if config.system_audio {
                stream
                    .addStreamOutput_type_sampleHandlerQueue_error(
                        sink,
                        SCStreamOutputType::Audio,
                        Some(&audio_queue),
                    )
                    .map_err(|error| format!("System audio couldn't be set up: {error}"))?;
            }
            wait_for_start(|handler| stream.startCaptureWithCompletionHandler(Some(handler)))
                .map_err(|error| format!("Capture wouldn't start: {error}"))?;
            Ok(Self {
                stream,
                _output: output,
                _video_queue: video_queue,
                _audio_queue: audio_queue,
            })
        }
    }

    /// Stops capturing. Nothing more reaches the recorder after this returns.
    pub fn stop(self) {
        // SAFETY: a documented call on a stream this owns, and the system copies the block.
        let _ = unsafe {
            wait_for_start(|handler| self.stream.stopCaptureWithCompletionHandler(Some(handler)))
        };
    }
}
