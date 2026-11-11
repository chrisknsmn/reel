//! The microphone and webcam, through AVFoundation.

use std::cell::OnceCell;
use std::sync::Arc;

use dispatch2::{DispatchQueue, DispatchRetained};
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{AnyThread, DefinedClass, define_class, msg_send};
use objc2_av_foundation::{
    AVCaptureAudioDataOutput, AVCaptureAudioDataOutputSampleBufferDelegate, AVCaptureConnection,
    AVCaptureDevice, AVCaptureDeviceDiscoverySession, AVCaptureDeviceInput,
    AVCaptureDevicePosition, AVCaptureDeviceTypeBuiltInWideAngleCamera,
    AVCaptureDeviceTypeContinuityCamera, AVCaptureDeviceTypeExternal,
    AVCaptureDeviceTypeMicrophone, AVCaptureOutput, AVCaptureSession,
    AVCaptureSessionPreset1280x720, AVCaptureVideoDataOutput,
    AVCaptureVideoDataOutputSampleBufferDelegate, AVMediaTypeAudio, AVMediaTypeVideo,
};
use objc2_core_media::{CMClock, CMSampleBuffer, CMSyncConvertTime, CMTime};
use objc2_core_video::kCVPixelFormatType_32BGRA;
use objc2_foundation::{NSArray, NSDictionary, NSNumber, NSString};
use reel_core::time::Micros;
use reel_media::macos::record::micros_of;
use serde::Serialize;

use super::recorder::{Recorder, Track};
use crate::config::DeviceChoice;

/// `kAudioFormatLinearPCM`.
const LINEAR_PCM: u32 = 0x6c70_636d;
const WEBCAM_FPS: i32 = 30;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct DeviceInfo {
    pub id: String,
    pub name: String,
}

fn discover(microphone: bool) -> Vec<Retained<AVCaptureDevice>> {
    // SAFETY: documented calls. These constants always exist on the supported systems.
    unsafe {
        let session = if microphone {
            let kinds = NSArray::from_slice(&[AVCaptureDeviceTypeMicrophone]);
            AVCaptureDeviceDiscoverySession::discoverySessionWithDeviceTypes_mediaType_position(
                &kinds,
                AVMediaTypeAudio,
                AVCaptureDevicePosition::Unspecified,
            )
        } else {
            let kinds = NSArray::from_slice(&[
                AVCaptureDeviceTypeBuiltInWideAngleCamera,
                AVCaptureDeviceTypeExternal,
                AVCaptureDeviceTypeContinuityCamera,
            ]);
            AVCaptureDeviceDiscoverySession::discoverySessionWithDeviceTypes_mediaType_position(
                &kinds,
                AVMediaTypeVideo,
                AVCaptureDevicePosition::Unspecified,
            )
        };
        session.devices().iter().collect()
    }
}

fn describe(devices: Vec<Retained<AVCaptureDevice>>) -> Vec<DeviceInfo> {
    devices
        .iter()
        // SAFETY: documented property reads on devices the system returned.
        .map(|device| unsafe {
            DeviceInfo {
                id: device.uniqueID().to_string(),
                name: device.localizedName().to_string(),
            }
        })
        .collect()
}

/// The microphones available. Listing them needs no permission.
#[must_use]
pub fn microphones() -> Vec<DeviceInfo> {
    describe(discover(true))
}

/// The cameras available. Listing them needs no permission.
#[must_use]
pub fn cameras() -> Vec<DeviceInfo> {
    describe(discover(false))
}

fn choose(choice: &DeviceChoice, microphone: bool) -> Option<Retained<AVCaptureDevice>> {
    let devices = discover(microphone);
    if let Some(id) = &choice.id {
        // SAFETY: documented property read on devices the system returned.
        let found = devices
            .iter()
            .find(|device| unsafe { device.uniqueID().to_string() } == *id)
            .cloned();
        if found.is_some() {
            return found;
        }
    }
    // SAFETY: these constants always exist, and this is a documented call.
    let default = unsafe {
        let kind = if microphone {
            AVMediaTypeAudio
        } else {
            AVMediaTypeVideo
        };
        AVCaptureDevice::defaultDeviceWithMediaType(kind.expect("this constant always exists"))
    };
    default.or_else(|| devices.into_iter().next())
}

struct DeviceState {
    recorder: Arc<Recorder>,
    clock: Option<ClockRef>,
}

struct ClockRef(Retained<CMClock>);

/// Timestamps from a capture session are on its own clock. Everything else is on the host's.
fn to_host(state: &DeviceState, time: CMTime) -> Micros {
    let Some(clock) = &state.clock else {
        return micros_of(time);
    };
    // SAFETY: both are live clocks, and this is a documented conversion.
    unsafe {
        let host = CMClock::host_time_clock();
        micros_of(CMSyncConvertTime(time, &clock.0, &host))
    }
}

define_class!(
    #[unsafe(super(NSObject))]
    #[name = "ReelMicOutput"]
    #[ivars = OnceCell<DeviceState>]
    struct MicOutput;

    unsafe impl NSObjectProtocol for MicOutput {}

    unsafe impl AVCaptureAudioDataOutputSampleBufferDelegate for MicOutput {
        #[unsafe(method(captureOutput:didOutputSampleBuffer:fromConnection:))]
        unsafe fn did_output(
            &self,
            _output: &AVCaptureOutput,
            sample: &CMSampleBuffer,
            _connection: &AVCaptureConnection,
        ) {
            let Some(state) = self.ivars().get() else { return };
            // SAFETY: a documented read on a live buffer.
            let host = to_host(state, unsafe { sample.presentation_time_stamp() });
            state.recorder.on_sound(Track::Microphone, sample, host);
        }
    }
);

define_class!(
    #[unsafe(super(NSObject))]
    #[name = "ReelCameraOutput"]
    #[ivars = OnceCell<DeviceState>]
    struct CameraOutput;

    unsafe impl NSObjectProtocol for CameraOutput {}

    unsafe impl AVCaptureVideoDataOutputSampleBufferDelegate for CameraOutput {
        #[unsafe(method(captureOutput:didOutputSampleBuffer:fromConnection:))]
        unsafe fn did_output(
            &self,
            _output: &AVCaptureOutput,
            sample: &CMSampleBuffer,
            _connection: &AVCaptureConnection,
        ) {
            let Some(state) = self.ivars().get() else { return };
            // SAFETY: documented reads on a live buffer.
            unsafe {
                let host = to_host(state, sample.presentation_time_stamp());
                if let Some(buffer) = sample.image_buffer() {
                    state.recorder.on_webcam_frame(&buffer, host);
                }
            }
        }
    }
);

/// A running capture from one device.
pub struct DeviceCapture {
    session: Retained<AVCaptureSession>,
    _delegate: Retained<NSObject>,
    _queue: DispatchRetained<DispatchQueue>,
}

// SAFETY: AVCaptureSession may be stopped from any thread.
unsafe impl Send for DeviceCapture {}

fn add_input(session: &AVCaptureSession, device: &AVCaptureDevice) -> Result<(), String> {
    // SAFETY: documented calls during session configuration.
    unsafe {
        let input = AVCaptureDeviceInput::deviceInputWithDevice_error(device)
            .map_err(|error| error.to_string())?;
        if !session.canAddInput(&input) {
            return Err("The device can't be used right now.".to_string());
        }
        session.addInput(&input);
    }
    Ok(())
}

fn session_clock(session: &AVCaptureSession) -> Option<ClockRef> {
    // SAFETY: a documented read. Older systems only have the older name.
    unsafe {
        let clock = if session.respondsToSelector(objc2::sel!(synchronizationClock)) {
            session.synchronizationClock()
        } else {
            #[allow(deprecated)]
            session.masterClock()
        };
        clock.map(ClockRef)
    }
}

impl DeviceCapture {
    /// Starts recording the mic.
    ///
    /// # Errors
    /// Returns why it won't start. The recording carries on without it, and tells the user.
    pub fn microphone(choice: &DeviceChoice, recorder: Arc<Recorder>) -> Result<Self, String> {
        let device = choose(choice, true).ok_or_else(|| "No microphone was found.".to_string())?;
        // SAFETY: documented setup calls, in order, on objects made here. The dictionary holds the
        // documented key and value types.
        unsafe {
            let session = AVCaptureSession::new();
            session.beginConfiguration();
            add_input(&session, &device)?;
            let output = AVCaptureAudioDataOutput::new();
            if !session.canAddOutput(&output) {
                return Err("The microphone can't be read right now.".to_string());
            }
            session.addOutput(&output);
            // One format whatever the device is, so a track never changes format part way.
            let (format, rate, channels, depth, yes, no) = (
                NSNumber::new_u32(LINEAR_PCM),
                NSNumber::new_f64(48_000.0),
                NSNumber::new_u32(2),
                NSNumber::new_u32(32),
                NSNumber::new_bool(true),
                NSNumber::new_bool(false),
            );
            let keys = [
                NSString::from_str("AVFormatIDKey"),
                NSString::from_str("AVSampleRateKey"),
                NSString::from_str("AVNumberOfChannelsKey"),
                NSString::from_str("AVLinearPCMBitDepthKey"),
                NSString::from_str("AVLinearPCMIsFloatKey"),
                NSString::from_str("AVLinearPCMIsNonInterleaved"),
            ];
            let keys: Vec<&NSString> = keys.iter().map(|key| &**key).collect();
            let values: [&AnyObject; 6] = [&format, &rate, &channels, &depth, &yes, &no];
            let settings: Retained<NSDictionary<NSString, AnyObject>> =
                NSDictionary::from_slices(&keys, &values);
            output.setAudioSettings(Some(&settings));
            session.commitConfiguration();

            let this = MicOutput::alloc().set_ivars(OnceCell::new());
            let delegate: Retained<MicOutput> = msg_send![super(this), init];
            let _ = delegate.ivars().set(DeviceState {
                recorder,
                clock: session_clock(&session),
            });
            let queue = DispatchQueue::new("dev.reel.capture.mic", None);
            let sink: &ProtocolObject<dyn AVCaptureAudioDataOutputSampleBufferDelegate> =
                ProtocolObject::from_ref(&*delegate);
            output.setSampleBufferDelegate_queue(Some(sink), Some(&queue));
            session.startRunning();
            if !session.isRunning() {
                return Err("The microphone wouldn't start.".to_string());
            }
            Ok(Self {
                session,
                _delegate: Retained::into_super(delegate),
                _queue: queue,
            })
        }
    }

    /// Starts recording the webcam, to its own file at 1280×720 and 30 fps.
    ///
    /// # Errors
    /// Returns why it won't start. The recording carries on without it, and tells the user.
    pub fn camera(choice: &DeviceChoice, recorder: Arc<Recorder>) -> Result<Self, String> {
        let device = choose(choice, false).ok_or_else(|| "No camera was found.".to_string())?;
        // SAFETY: documented setup calls, in order, on objects made here. The dictionary holds the
        // documented key and value types.
        unsafe {
            let session = AVCaptureSession::new();
            session.beginConfiguration();
            let preset = AVCaptureSessionPreset1280x720;
            if session.canSetSessionPreset(preset) {
                session.setSessionPreset(preset);
            }
            add_input(&session, &device)?;
            let output = AVCaptureVideoDataOutput::new();
            output.setAlwaysDiscardsLateVideoFrames(true);
            let format = NSNumber::new_u32(kCVPixelFormatType_32BGRA);
            let key = NSString::from_str("PixelFormatType");
            let settings: Retained<NSDictionary<NSString, AnyObject>> =
                NSDictionary::from_slices(&[&*key], &[format.as_ref() as &AnyObject]);
            output.setVideoSettings(Some(&settings));
            if !session.canAddOutput(&output) {
                return Err("The camera can't be read right now.".to_string());
            }
            session.addOutput(&output);
            if device.lockForConfiguration().is_ok() {
                // Not every camera allows this, and it isn't worth failing over.
                device.setActiveVideoMinFrameDuration(CMTime::new(1, WEBCAM_FPS));
                device.unlockForConfiguration();
            }
            session.commitConfiguration();

            recorder.open_webcam();
            let this = CameraOutput::alloc().set_ivars(OnceCell::new());
            let delegate: Retained<CameraOutput> = msg_send![super(this), init];
            let _ = delegate.ivars().set(DeviceState {
                recorder,
                clock: session_clock(&session),
            });
            let queue = DispatchQueue::new("dev.reel.capture.camera", None);
            let sink: &ProtocolObject<dyn AVCaptureVideoDataOutputSampleBufferDelegate> =
                ProtocolObject::from_ref(&*delegate);
            output.setSampleBufferDelegate_queue(Some(sink), Some(&queue));
            session.startRunning();
            if !session.isRunning() {
                return Err("The camera wouldn't start.".to_string());
            }
            Ok(Self {
                session,
                _delegate: Retained::into_super(delegate),
                _queue: queue,
            })
        }
    }

    pub fn stop(self) {
        // SAFETY: a documented call on a session this owns.
        unsafe { self.session.stopRunning() };
    }
}

/// A level from 0 to 100, from a power in decibels.
#[must_use]
pub fn level_from_decibels(decibels: f64) -> f64 {
    ((decibels + 60.0) / 60.0 * 100.0).clamp(0.0, 100.0)
}

/// Reports a microphone's level about 12 times a second, until told to stop.
///
/// # Errors
/// Returns why the microphone can't be read.
pub fn run_meter(id: &str, mut report: impl FnMut(f64) -> bool) -> Result<(), String> {
    let choice = DeviceChoice {
        id: Some(id.to_string()),
    };
    let device = choose(&choice, true).ok_or_else(|| "No microphone was found.".to_string())?;
    // SAFETY: documented setup calls, in order, on objects made here, and documented property reads.
    unsafe {
        let session = AVCaptureSession::new();
        session.beginConfiguration();
        add_input(&session, &device)?;
        let output = AVCaptureAudioDataOutput::new();
        if !session.canAddOutput(&output) {
            return Err("The microphone can't be read right now.".to_string());
        }
        session.addOutput(&output);
        session.commitConfiguration();
        session.startRunning();
        loop {
            std::thread::sleep(std::time::Duration::from_millis(80));
            let connections = output.connections();
            let Some(connection) = connections.firstObject() else {
                continue;
            };
            let channels = connection.audioChannels();
            let peak = channels
                .iter()
                .map(|channel| f64::from(channel.averagePowerLevel()))
                .fold(-160.0, f64::max);
            if !report(level_from_decibels(peak)) {
                break;
            }
        }
        session.stopRunning();
    }
    Ok(())
}
