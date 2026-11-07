//! The launch bar (§2, §47): a small floating window to choose what to record and start recording.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use dioxus::desktop::tao::dpi::LogicalSize;
use dioxus::desktop::tao::event::{Event as TaoEvent, WindowEvent};
use dioxus::desktop::{Config, WindowBuilder, WindowCloseBehaviour, use_wry_event_handler};
use dioxus::prelude::*;
use icons::{Check, Ellipsis, Mic, Minus, Monitor, Pause, Play, Square, Timer, Video, X};
use reel_capture::config::{DeviceChoice, Event, RecordConfig, SourceChoice};
use reel_capture::disk;
use reel_core::settings::SettingsStore;
use reel_core::time::Micros;

use crate::links::{self, ToBar, ToEditor};
use crate::native;
use crate::recording::{
    self, Devices, Permissions, RecorderProcess, Sources, list_devices, list_sources,
};
use crate::ui::button::{Button, ButtonSize, ButtonVariant};
use crate::ui::spinner::Spinner;

pub const BAR_WIDTH: f64 = 780.0;
pub const BAR_HEIGHT: f64 = 72.0;
pub const OPEN_HEIGHT: f64 = 472.0;
const COUNTDOWNS: [u32; 4] = [0, 3, 5, 10];
const POP_WIDTH: f64 = 300.0;

/// Which popover is open. Only one ever is (§2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pop {
    Source,
    Audio,
    Webcam,
    Countdown,
    More,
    ConfirmCancel,
}

impl Pop {
    /// Roughly above its button.
    fn left(self) -> f64 {
        let center = match self {
            Self::Source => 130.0,
            Self::Audio => 240.0,
            Self::Webcam => 290.0,
            Self::Countdown => 340.0,
            Self::More => 650.0,
            Self::ConfirmCancel => 620.0,
        };
        (center - POP_WIDTH / 2.0).clamp(8.0, BAR_WIDTH - POP_WIDTH - 8.0)
    }
}

/// What the bar is doing.
#[derive(Clone, Debug, PartialEq)]
pub enum Mode {
    Idle,
    /// The recorder is warming up, so that the countdown is the only wait there is (§27).
    Starting,
    Counting,
    Recording {
        paused: bool,
    },
    Finalizing,
}

/// A chosen source, with what's shown for it.
#[derive(Clone, Debug, PartialEq)]
pub struct ChosenSource {
    pub choice: SourceChoice,
    pub name: String,
}

/// `MM:SS`, which is what the bar shows while recording.
#[must_use]
pub fn timer_text(elapsed: Duration) -> String {
    let total = elapsed.as_secs();
    format!("{:02}:{:02}", total / 60, total % 60)
}

/// A level from 0 to 100 as the five-bar meter in §27. Blue, then yellow, then red.
#[must_use]
pub fn meter_bars(level: f64) -> [Option<&'static str>; 5] {
    const STEPS: [(f64, &str); 5] = [
        (10.0, "green"),
        (25.0, "green"),
        (45.0, "green"),
        (65.0, "yellow"),
        (85.0, "red"),
    ];
    STEPS.map(|(threshold, color)| (level >= threshold).then_some(color))
}

/// The label for a countdown choice.
#[must_use]
pub fn countdown_label(seconds: u32) -> String {
    if seconds == 0 {
        "No delay".to_string()
    } else {
        format!("{seconds} seconds")
    }
}

/// Tests can record a pattern instead of the screen, which needs no permission.
fn test_source() -> Option<ChosenSource> {
    std::env::var_os("REEL_TEST_SOURCE").map(|_| ChosenSource {
        choice: SourceChoice::Synthetic {
            width: 1280,
            height: 720,
            mic_delay_ms: 0,
        },
        name: "Test pattern".to_string(),
    })
}

fn settings_path() -> PathBuf {
    recording::support_folder().join("settings.json")
}

fn recordings_folder(settings: &SettingsStore) -> PathBuf {
    settings
        .get()
        .recordings_dir
        .clone()
        .unwrap_or_else(|| recording::support_folder().join("recordings"))
}

/// A data URL for a thumbnail, since the webview can't read files from disk.
fn thumbnail_url(path: &Path) -> Option<String> {
    let bytes = std::fs::read(path).ok()?;
    Some(format!("data:image/png;base64,{}", crate::base64(&bytes)))
}

/// Opens the bar's window.
pub fn open_window() {
    let window = WindowBuilder::new()
        .with_title("Reel")
        .with_decorations(false)
        .with_transparent(true)
        .with_resizable(false)
        .with_always_on_top(true)
        .with_visible_on_all_workspaces(true)
        .with_inner_size(LogicalSize::new(BAR_WIDTH, BAR_HEIGHT));
    let config = Config::new()
        .with_window(window)
        .with_menu(None)
        .with_close_behaviour(WindowCloseBehaviour::WindowHides)
        .with_background_color((0, 0, 0, 0))
        .with_custom_head(crate::head());
    let dom = VirtualDom::new(Bar);
    let _pending = dioxus::desktop::window().new_window(dom, config);
}

fn set_open(mut open: Signal<Option<Pop>>, value: Option<Pop>) {
    let window = dioxus::desktop::window();
    // Grow before showing it, and shrink after hiding it, so nothing is ever cut off (T3).
    if value.is_some() {
        native::set_height(&window.window, OPEN_HEIGHT);
    }
    open.set(value);
    if value.is_none() {
        native::set_height(&window.window, BAR_HEIGHT);
    }
}

#[component]
pub fn Bar() -> Element {
    let open = use_signal(|| None::<Pop>);
    let mut mode = use_signal(|| Mode::Idle);
    let mut message = use_signal(|| None::<String>);
    let source = use_signal(test_source);
    let mut sources = use_signal(Sources::default);
    let mut devices = use_signal(Devices::default);
    let mut allowed = use_signal(Permissions::default);
    let mut system_audio = use_signal(|| false);
    let mut microphone = use_signal(|| None::<String>);
    let camera = use_signal(|| None::<String>);
    let floating_preview = use_signal(|| true);
    let mut countdown = use_signal(|| 3_u32);
    let mut hide_from_recording = use_signal(|| true);
    let mut elapsed = use_signal(|| Duration::ZERO);
    let mut running_since = use_signal(|| None::<Instant>);
    let mut recorder = use_signal(|| None::<RecorderProcess>);
    let mut folder = use_signal(|| None::<PathBuf>);

    use_hook(|| {
        let window = dioxus::desktop::window();
        native::accept_first_mouse();
        native::pin_webview(&window.window, BAR_WIDTH, OPEN_HEIGHT);
        // Back where it was left, on any monitor that's still there (§2).
        let saved = SettingsStore::open(&settings_path()).get().bar_position;
        let placed = saved.is_some_and(|(x, y)| native::place_at(&window.window, x, y));
        if !placed {
            native::place_bottom_center(&window.window, 16.0);
        }
        native::set_capture_protected(&window.window, true);
        // Each window has its own page, so each needs telling about the theme.
        let theme = SettingsStore::open(&settings_path()).get().theme;
        document::eval(&format!(
            "document.documentElement.classList.toggle('dark', {});",
            crate::theme_is_dark(theme)
        ));
    });

    // Loads what was chosen last time, and looks for anything a crash left behind (§4, §59).
    use_future(move || async move {
        let store = SettingsStore::open(&settings_path());
        let saved = store.get().clone();
        system_audio.set(saved.system_audio_enabled);
        if saved.microphone_enabled {
            microphone.set(saved.microphone_device.clone().or(Some(String::new())));
        }
        if saved.countdown_seconds > 0 || COUNTDOWNS.contains(&saved.countdown_seconds) {
            countdown.set(saved.countdown_seconds);
        }
        hide_from_recording.set(saved.hide_bar_from_recording);
        let place = recordings_folder(&store);
        folder.set(Some(place.clone()));
        for result in recording::recover_interrupted(place).await {
            match result {
                Ok(saved) => {
                    message.set(Some(
                        "A recording was cut off. What could be saved is ready to open."
                            .to_string(),
                    ));
                    links::tell_editor(ToEditor::Open(saved.project));
                }
                Err(error) => message.set(Some(format!(
                    "A cut-off recording couldn't be saved: {error}"
                ))),
            }
        }
    });

    // Remembers choices between sessions (§4).
    use_effect(move || {
        let (system, mic, wait, hide) = (
            system_audio(),
            microphone(),
            countdown(),
            hide_from_recording(),
        );
        let chosen_folder = folder();
        // Only once what was saved has been loaded, so the defaults don't overwrite it.
        if chosen_folder.is_some() {
            let mut store = SettingsStore::open(&settings_path());
            let _ = store.update(|settings| {
                settings.system_audio_enabled = system;
                settings.microphone_enabled = mic.is_some();
                settings.microphone_device = mic.filter(|id| !id.is_empty());
                settings.countdown_seconds = wait;
                settings.hide_bar_from_recording = hide;
            });
        }
    });

    // Reel's own windows stay out of recordings unless the user wants them in (§2).
    use_effect(move || {
        native::set_capture_protected(&dioxus::desktop::window().window, hide_from_recording());
    });

    // The timer leaves out paused time, and comes from a clock rather than from counting ticks.
    use_future(move || async move {
        let mut tick = tokio::time::interval(Duration::from_millis(200));
        loop {
            tick.tick().await;
            if let Some(since) = running_since() {
                elapsed.set(since.elapsed());
            }
        }
    });

    let mut stop_recording = move || {
        if let Some(process) = recorder.write().as_mut() {
            process.stop();
        }
        running_since.set(None);
        mode.set(Mode::Finalizing);
    };

    // Everything the recorder reports.
    use_future(move || async move {
        let mut tick = tokio::time::interval(Duration::from_millis(30));
        loop {
            tick.tick().await;
            let event = recorder
                .write()
                .as_mut()
                .and_then(|process| process.events.try_recv().ok());
            let Some(event) = event else { continue };
            match event {
                Event::Started { .. } => {
                    if mode() == Mode::Starting {
                        run_countdown(countdown(), mode, recorder, running_since);
                    }
                }
                Event::Paused | Event::Resumed => {}
                Event::Warning { message: text } => message.set(Some(text)),
                Event::WindowUnavailable => {
                    message.set(Some("The window being recorded has closed.".to_string()));
                    mode.set(Mode::Finalizing);
                    running_since.set(None);
                }
                Event::LowDisk { .. } => {
                    message.set(Some(
                        "The disk is nearly full, so recording has stopped.".to_string(),
                    ));
                    mode.set(Mode::Finalizing);
                    running_since.set(None);
                }
                Event::Stopped {
                    output,
                    duration_ms,
                    ..
                } => {
                    if let Some(process) = recorder.write().take() {
                        process.wait();
                    }
                    let duration = Micros::from_ms(duration_ms);
                    match recording::save_project_for(&output, duration) {
                        Ok(project) => {
                            let window = dioxus::desktop::window();
                            window.set_visible(false);
                            links::tell_editor(ToEditor::Open(project));
                            let store = SettingsStore::open(&settings_path());
                            if store.get().retention_enabled {
                                let _ =
                                    recording::apply_retention(&recordings_folder(&store), &output);
                            }
                        }
                        Err(error) => message.set(Some(error)),
                    }
                    elapsed.set(Duration::ZERO);
                    mode.set(Mode::Idle);
                }
                Event::Failed { message: text } => {
                    if let Some(process) = recorder.write().take() {
                        process.wait();
                    }
                    message.set(Some(text));
                    running_since.set(None);
                    elapsed.set(Duration::ZERO);
                    mode.set(Mode::Idle);
                }
            }
        }
    });

    // Messages from the other windows.
    use_future(move || async move {
        let mut inbox = links::listen_as_bar();
        while let Some(message_in) = inbox.recv().await {
            match message_in {
                ToBar::EditorClosed => dioxus::desktop::window().set_visible(true),
                ToBar::CancelCountdown => {
                    if let Some(process) = recorder.write().take() {
                        process.discard();
                    }
                    running_since.set(None);
                    mode.set(Mode::Idle);
                }
            }
        }
    });

    // A global shortcut starts and stops recording from anywhere (§23, §59).
    use_future(move || async move {
        let mut tick = tokio::time::interval(Duration::from_millis(100));
        loop {
            tick.tick().await;
            if crate::hotkey::was_pressed() {
                match mode() {
                    Mode::Recording { .. } => stop_recording(),
                    Mode::Idle if source().is_some() => {
                        start(
                            source,
                            system_audio,
                            microphone,
                            camera,
                            hide_from_recording,
                            folder,
                            mode,
                            message,
                            recorder,
                        );
                    }
                    Mode::Idle => {
                        dioxus::desktop::window().set_visible(true);
                        set_open(open, Some(Pop::Source));
                    }
                    _ => {}
                }
            }
        }
    });

    #[cfg(feature = "self-test")]
    use_future(move || async move {
        if std::env::var("REEL_SELF_TEST").as_deref() == Ok("bar") {
            crate::bar_test::run().await;
        }
    });

    // Clicking on another app closes whatever's open, and the Dock icon brings a hidden bar back.
    use_wry_event_handler(move |event, _| match event {
        TaoEvent::WindowEvent {
            event: WindowEvent::Focused(false),
            window_id,
            ..
        } if *window_id == dioxus::desktop::window().id() && open.peek().is_some() => {
            set_open(open, None);
        }
        TaoEvent::WindowEvent {
            event: WindowEvent::Moved(_),
            window_id,
            ..
        } if *window_id == dioxus::desktop::window().id() => {
            let place = native::origin(&dioxus::desktop::window().window);
            let mut store = SettingsStore::open(&settings_path());
            if store.get().bar_position != Some(place) {
                let _ = store.update(|settings| settings.bar_position = Some(place));
            }
        }
        TaoEvent::Reopen { .. } => dioxus::desktop::window().set_visible(true),
        _ => {}
    });

    let toggle = move |which: Pop| {
        let next = if open() == Some(which) {
            None
        } else {
            Some(which)
        };
        set_open(open, next);
        // Lists are fetched when they're opened, so they're always current (§4).
        match next {
            Some(Pop::Source) => {
                spawn(async move {
                    allowed.set(recording::permissions().await);
                    if allowed().screen_allowed() {
                        let folder = recording::support_folder().join("thumbnails");
                        match list_sources(&folder).await {
                            Ok(found) => sources.set(found),
                            Err(error) => message.set(Some(error)),
                        }
                    }
                });
            }
            Some(Pop::Audio | Pop::Webcam) => {
                spawn(async move {
                    allowed.set(recording::permissions().await);
                    if let Ok(found) = list_devices().await {
                        devices.set(found);
                    }
                });
            }
            _ => {}
        }
    };

    let on_record = move |_| {
        message.set(None);
        if source().is_none() {
            toggle(Pop::Source);
            return;
        }
        set_open(open, None);
        start(
            source,
            system_audio,
            microphone,
            camera,
            hide_from_recording,
            folder,
            mode,
            message,
            recorder,
        );
    };

    let is_open = |which: Pop| open() == Some(which);
    let source_label = source().map_or_else(|| "Screen".to_string(), |chosen| chosen.name);
    let current_mode = mode();

    rsx! {
        div { class: "bar-root",
            if let Some(which) = open() {
                div {
                    class: "bar-pop",
                    id: "bar-pop",
                    "data-pop": "{which:?}",
                    style: "left: {which.left()}px",
                    match which {
                        Pop::Source => rsx! { SourceList { source, sources, allowed, open } },
                        Pop::Audio => rsx! { AudioList { devices, allowed, system_audio, microphone } },
                        Pop::Webcam => rsx! { WebcamList { devices, allowed, camera, floating_preview } },
                        Pop::Countdown => rsx! { CountdownList { countdown } },
                        Pop::More => rsx! { MoreList { hide_from_recording, folder } },
                        Pop::ConfirmCancel => rsx! {
                            h3 { "Discard this recording?" }
                            p { class: "bar-note", "This can't be undone." }
                            div { class: "bar-confirm",
                                Button {
                                    id: "cancel-keep",
                                    variant: ButtonVariant::Outline,
                                    size: ButtonSize::Sm,
                                    onclick: move |_| set_open(open, None),
                                    "Keep recording"
                                }
                                Button {
                                    id: "cancel-discard",
                                    variant: ButtonVariant::Destructive,
                                    size: ButtonSize::Sm,
                                    onclick: move |_| {
                                        set_open(open, None);
                                        if let Some(process) = recorder.write().take() {
                                            process.discard();
                                        }
                                        running_since.set(None);
                                        elapsed.set(Duration::ZERO);
                                        mode.set(Mode::Idle);
                                    },
                                    "Discard"
                                }
                            }
                        },
                    }
                }
            }
            div { class: "bar", id: "bar",
                div {
                    class: "bar-handle",
                    id: "bar-handle",
                    onmousedown: move |_| dioxus::desktop::window().drag(),
                }
                match current_mode {
                    Mode::Idle | Mode::Starting | Mode::Counting => rsx! {
                        button {
                            class: "bar-button bar-source",
                            id: "pop-source",
                            "data-open": "{is_open(Pop::Source)}",
                            "aria-label": "Choose what to record",
                            onclick: move |_| toggle(Pop::Source),
                            Monitor {}
                            span { "{source_label}" }
                        }
                        button {
                            class: "bar-button",
                            id: "pop-audio",
                            "data-open": "{is_open(Pop::Audio)}",
                            "aria-label": "Audio",
                            onclick: move |_| toggle(Pop::Audio),
                            Mic {}
                        }
                        button {
                            class: "bar-button",
                            id: "pop-webcam",
                            "data-open": "{is_open(Pop::Webcam)}",
                            "aria-label": "Webcam",
                            onclick: move |_| toggle(Pop::Webcam),
                            Video {}
                        }
                        button {
                            class: "bar-button",
                            id: "pop-countdown",
                            "data-open": "{is_open(Pop::Countdown)}",
                            "aria-label": "Countdown",
                            onclick: move |_| toggle(Pop::Countdown),
                            Timer {}
                            if countdown() > 0 { span { "{countdown()}s" } }
                        }
                        if let Some(text) = message() {
                            span { class: "bar-message", role: "alert", title: "{text}", "{text}" }
                        }
                        div { class: "bar-spacer" }
                        button {
                            class: "bar-record",
                            id: "record",
                            disabled: mode() != Mode::Idle,
                            onclick: on_record,
                            svg { view_box: "0 0 16 16", circle { cx: "8", cy: "8", r: "6" } }
                            "Record"
                        }
                        button {
                            class: "bar-button",
                            id: "pop-more",
                            "data-open": "{is_open(Pop::More)}",
                            "aria-label": "More",
                            onclick: move |_| toggle(Pop::More),
                            Ellipsis {}
                        }
                        button {
                            class: "bar-button",
                            id: "bar-hide",
                            "aria-label": "Hide",
                            onclick: move |_| dioxus::desktop::window().set_visible(false),
                            Minus {}
                        }
                        button {
                            class: "bar-button",
                            id: "bar-close",
                            "aria-label": "Close",
                            onclick: move |_| close_bar(),
                            X {}
                        }
                    },
                    Mode::Recording { paused } => rsx! {
                        span {
                            class: "bar-status",
                            id: "bar-status",
                            span { class: "bar-dot", "data-paused": "{paused}" }
                            if paused { "PAUSED" } else { "REC" }
                            span { id: "bar-timer", " {timer_text(elapsed())}" }
                        }
                        if let Some(text) = message() {
                            span { class: "bar-message", role: "alert", title: "{text}", "{text}" }
                        }
                        div { class: "bar-spacer" }
                        if paused {
                            button {
                                class: "bar-button bar-resume",
                                id: "bar-resume",
                                "aria-label": "Resume",
                                onclick: move |_| {
                                    if let Some(process) = recorder.write().as_mut() {
                                        process.resume();
                                    }
                                    let so_far = elapsed();
                                    running_since.set(Instant::now().checked_sub(so_far));
                                    mode.set(Mode::Recording { paused: false });
                                },
                                Play {}
                            }
                        } else {
                            button {
                                class: "bar-button",
                                id: "bar-pause",
                                "aria-label": "Pause",
                                onclick: move |_| {
                                    if let Some(process) = recorder.write().as_mut() {
                                        process.pause();
                                    }
                                    running_since.set(None);
                                    mode.set(Mode::Recording { paused: true });
                                },
                                Pause {}
                            }
                        }
                        button {
                            class: "bar-button",
                            id: "bar-stop",
                            "aria-label": "Stop",
                            onclick: move |_| stop_recording(),
                            Square {}
                        }
                        button {
                            class: "bar-button",
                            id: "bar-hide",
                            "aria-label": "Hide",
                            onclick: move |_| dioxus::desktop::window().set_visible(false),
                            Minus {}
                        }
                        button {
                            class: "bar-button",
                            id: "bar-cancel",
                            "aria-label": "Cancel and discard",
                            onclick: move |_| set_open(open, Some(Pop::ConfirmCancel)),
                            X {}
                        }
                    },
                    Mode::Finalizing => rsx! {
                        div { class: "bar-finalizing", id: "bar-finalizing",
                            Spinner {}
                            div {
                                "Preparing recording"
                                small { "Opening the editor in a moment" }
                            }
                        }
                    },
                }
            }
        }
    }
}

/// With no project open, closing the bar leaves nothing showing, so the app quits (§2).
fn close_bar() {
    if crate::editor_is_showing() {
        dioxus::desktop::window().set_visible(false);
    } else {
        std::process::exit(0);
    }
}

#[allow(clippy::too_many_arguments)]
fn start(
    source: Signal<Option<ChosenSource>>,
    system_audio: Signal<bool>,
    microphone: Signal<Option<String>>,
    camera: Signal<Option<String>>,
    hide: Signal<bool>,
    folder: Signal<Option<PathBuf>>,
    mut mode: Signal<Mode>,
    mut message: Signal<Option<String>>,
    mut recorder: Signal<Option<RecorderProcess>>,
) {
    let Some(chosen) = source() else { return };
    let place = folder().unwrap_or_else(|| recording::support_folder().join("recordings"));
    if !disk::may_start(disk::free_bytes(&place)) {
        message.set(Some(
            "There isn't enough disk space to start recording.".to_string(),
        ));
        return;
    }
    let is_test = matches!(chosen.choice, SourceChoice::Synthetic { .. });
    let config = RecordConfig {
        source: chosen.choice,
        output: recording::new_recording_path(&place),
        fps: 60,
        system_audio: system_audio(),
        microphone: microphone().map(|id| DeviceChoice {
            id: Some(id).filter(|id| !id.is_empty()),
        }),
        webcam: camera().map(|id| DeviceChoice {
            id: Some(id).filter(|id| !id.is_empty()),
        }),
        cursor: true,
        clicks: !is_test,
        // Warm start: capture is already running when the countdown ends (§27).
        start_paused: true,
        exclude_bundles: Vec::new(),
        hide_own_windows: hide(),
    };
    match RecorderProcess::start(&config) {
        Ok(process) => {
            recorder.set(Some(process));
            mode.set(Mode::Starting);
        }
        Err(error) => message.set(Some(error)),
    }
}

/// Shows the countdown, then starts keeping frames.
fn run_countdown(
    seconds: u32,
    mut mode: Signal<Mode>,
    mut recorder: Signal<Option<RecorderProcess>>,
    mut running_since: Signal<Option<Instant>>,
) {
    mode.set(Mode::Counting);
    if seconds > 0 {
        crate::countdown::open_window(seconds);
    }
    spawn(async move {
        tokio::time::sleep(Duration::from_secs(u64::from(seconds))).await;
        // A cancel will have taken the recorder away.
        if mode() != Mode::Counting {
            return;
        }
        if let Some(process) = recorder.write().as_mut() {
            process.resume();
        }
        running_since.set(Some(Instant::now()));
        mode.set(Mode::Recording { paused: false });
    });
}

#[component]
fn SourceList(
    source: Signal<Option<ChosenSource>>,
    sources: Signal<Sources>,
    allowed: Signal<Permissions>,
    open: Signal<Option<Pop>>,
) -> Element {
    let found = sources();
    let selected = source();
    let is_selected = move |choice: &SourceChoice| {
        selected
            .as_ref()
            .is_some_and(|chosen| chosen.choice == *choice)
    };
    if !allowed().screen_allowed() {
        return rsx! {
            h3 { "Screen Recording" }
            p { class: "bar-note",
                "Reel needs permission to record your screen. After you allow it in System Settings, quit Reel and open it again."
            }
            div { class: "bar-confirm",
                Button {
                    id: "allow-screen",
                    size: ButtonSize::Sm,
                    onclick: move |_| {
                        spawn(async move {
                            let _ = recording::request_permission("screen").await;
                        });
                    },
                    "Allow"
                }
            }
        };
    }
    let mut choose =
        move |choice: SourceChoice, name: String, bounds: Option<(f64, f64, f64, f64)>| {
            source.set(Some(ChosenSource { choice, name }));
            set_open(open, None);
            if let Some(area) = bounds {
                crate::countdown::open_highlight(area);
            }
        };
    rsx! {
        h3 { "Screens" }
        for display in found.displays {
            {
                let choice = SourceChoice::Display { id: display.id };
                let name = display.name.clone();
                let area = (display.x, display.y, display.width, display.height);
                let thumbnail = display.thumbnail.as_deref().and_then(thumbnail_url);
                let selected = is_selected(&choice);
                rsx! {
                    button {
                        class: "bar-row",
                        "data-selected": "{selected}",
                        onclick: move |_| choose(choice.clone(), name.clone(), Some(area)),
                        if let Some(url) = thumbnail { img { class: "bar-thumb", src: "{url}", alt: "" } }
                        span { "{display.name}" }
                    }
                }
            }
        }
        h3 { "Windows" }
        for window in found.windows {
            {
                let choice = SourceChoice::Window { id: window.id };
                let name = window.name.clone();
                let thumbnail = window.thumbnail.as_deref().and_then(thumbnail_url);
                let selected = is_selected(&choice);
                let id = window.id;
                rsx! {
                    button {
                        class: "bar-row",
                        "data-selected": "{selected}",
                        onclick: move |_| {
                            let bounds = reel_capture::macos::windows::window_bounds(id)
                                .map(|area| (area.x, area.y, area.width, area.height));
                            choose(choice.clone(), name.clone(), bounds);
                        },
                        if let Some(url) = thumbnail { img { class: "bar-thumb", src: "{url}", alt: "" } }
                        span { "{window.name}" }
                    }
                }
            }
        }
    }
}

#[component]
fn Meter(id: String) -> Element {
    let mut level = use_signal(|| 0.0_f64);
    let device = id.clone();
    use_future(move || {
        let device = device.clone();
        async move {
            let Ok(mut process) = crate::meter::Meter::start(&device) else {
                return;
            };
            let mut tick = tokio::time::interval(Duration::from_millis(80));
            loop {
                tick.tick().await;
                if let Some(value) = process.latest() {
                    level.set(value);
                }
            }
        }
    });
    rsx! {
        span { class: "bar-meter", "aria-hidden": "true",
            for color in meter_bars(level()) {
                i { "data-on": color.unwrap_or("off") }
            }
        }
    }
}

#[component]
fn AudioList(
    devices: Signal<Devices>,
    allowed: Signal<Permissions>,
    system_audio: Signal<bool>,
    microphone: Signal<Option<String>>,
) -> Element {
    let found = devices().microphones;
    let meters = allowed().microphone_allowed();
    rsx! {
        h3 { "System audio" }
        button {
            class: "bar-row",
            id: "toggle-system-audio",
            "data-selected": "{system_audio()}",
            onclick: move |_| system_audio.toggle(),
            span { "Record system audio" }
            span { class: "bar-check", if system_audio() { Check {} } }
        }
        h3 { "Microphone" }
        button {
            class: "bar-row",
            id: "mic-off",
            "data-selected": "{microphone().is_none()}",
            onclick: move |_| microphone.set(None),
            span { "Off" }
            span { class: "bar-check", if microphone().is_none() { Check {} } }
        }
        if found.is_empty() {
            p { class: "bar-note", "No microphones found" }
        }
        for device in found {
            {
                let id = device.id.clone();
                let selected = microphone().as_deref() == Some(id.as_str());
                let meter_id = device.id.clone();
                rsx! {
                    button {
                        class: "bar-row",
                        "data-selected": "{selected}",
                        onclick: move |_| {
                            let chosen = id.clone();
                            microphone.set(Some(chosen));
                            // Asking only now, when a feature needs it (§48).
                            spawn(async move {
                                if recording::permissions().await.microphone == "not_asked" {
                                    let _ = recording::request_permission("microphone").await;
                                }
                            });
                        },
                        span { "{device.name}" }
                        if meters { Meter { key: "{meter_id}", id: meter_id.clone() } }
                        span { class: "bar-check", if selected { Check {} } }
                    }
                }
            }
        }
    }
}

#[component]
fn WebcamList(
    devices: Signal<Devices>,
    allowed: Signal<Permissions>,
    camera: Signal<Option<String>>,
    floating_preview: Signal<bool>,
) -> Element {
    let found = devices().cameras;
    let _ = allowed;
    rsx! {
        h3 { "Webcam" }
        button {
            class: "bar-row",
            id: "camera-off",
            "data-selected": "{camera().is_none()}",
            onclick: move |_| {
                camera.set(None);
                crate::camera_preview::hide();
            },
            span { "Off" }
            span { class: "bar-check", if camera().is_none() { Check {} } }
        }
        if found.is_empty() {
            p { class: "bar-note", "No webcams found" }
        }
        for device in found {
            {
                let id = device.id.clone();
                let selected = camera().as_deref() == Some(id.as_str());
                rsx! {
                    button {
                        class: "bar-row",
                        "data-selected": "{selected}",
                        onclick: move |_| {
                            let chosen = id.clone();
                            camera.set(Some(chosen.clone()));
                            let float = floating_preview();
                            spawn(async move {
                                if recording::permissions().await.camera == "not_asked" {
                                    let _ = recording::request_permission("camera").await;
                                }
                                if float && recording::permissions().await.camera_allowed() {
                                    crate::camera_preview::show(&chosen);
                                }
                            });
                        },
                        span { "{device.name}" }
                        span { class: "bar-check", if selected { Check {} } }
                    }
                }
            }
        }
        h3 { "Preview" }
        button {
            class: "bar-row",
            id: "toggle-floating-preview",
            "data-selected": "{floating_preview()}",
            onclick: move |_| {
                floating_preview.toggle();
                if floating_preview() {
                    if let Some(id) = camera() {
                        crate::camera_preview::show(&id);
                    }
                } else {
                    crate::camera_preview::hide();
                }
            },
            span { "Show floating preview" }
            span { class: "bar-check", if floating_preview() { Check {} } }
        }
    }
}

#[component]
fn CountdownList(countdown: Signal<u32>) -> Element {
    rsx! {
        h3 { "Countdown" }
        for seconds in COUNTDOWNS {
            button {
                class: "bar-row",
                id: "countdown-{seconds}",
                "data-selected": "{countdown() == seconds}",
                onclick: move |_| countdown.set(seconds),
                span { "{countdown_label(seconds)}" }
                span { class: "bar-check", if countdown() == seconds { Check {} } }
            }
        }
    }
}

#[component]
fn MoreList(hide_from_recording: Signal<bool>, folder: Signal<Option<PathBuf>>) -> Element {
    let place = folder().map_or_else(String::new, |path| path.display().to_string());
    rsx! {
        h3 { "Recording" }
        button {
            class: "bar-row",
            id: "toggle-hide-bar",
            "data-selected": "{hide_from_recording()}",
            onclick: move |_| hide_from_recording.toggle(),
            span { "Hide this bar from recordings" }
            span { class: "bar-check", if hide_from_recording() { Check {} } }
        }
        button {
            class: "bar-row",
            id: "choose-folder",
            title: "{place}",
            onclick: move |_| {
                let picked = rfd::FileDialog::new().set_title("Choose where recordings go").pick_folder();
                if let Some(path) = picked {
                    let mut store = SettingsStore::open(&settings_path());
                    let chosen = path.clone();
                    let _ = store.update(|settings| settings.recordings_dir = Some(chosen));
                    folder.set(Some(path));
                }
            },
            span { "Recordings folder…" }
        }
        h3 { "Open" }
        button {
            class: "bar-row",
            id: "open-file",
            onclick: move |_| {
                let picked = rfd::FileDialog::new()
                    .set_title("Open a video or project")
                    .add_filter("Videos and projects", &["mp4", "mov", "m4v", "reel", "recordly", "openscreen"])
                    .pick_file();
                if let Some(path) = picked {
                    dioxus::desktop::window().set_visible(false);
                    links::tell_editor(ToEditor::Open(path));
                }
            },
            span { "Open a video or project…" }
        }
        p { class: "bar-note", "Reel {env!(\"CARGO_PKG_VERSION\")}" }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_timer_reads_as_minutes_and_seconds() {
        assert_eq!(timer_text(Duration::ZERO), "00:00");
        assert_eq!(timer_text(Duration::from_secs(61)), "01:01");
        assert_eq!(timer_text(Duration::from_secs(3725)), "62:05");
    }

    #[test]
    fn the_meter_follows_the_documented_steps() {
        assert_eq!(meter_bars(0.0), [None; 5]);
        assert_eq!(meter_bars(10.0)[0], Some("green"));
        assert_eq!(meter_bars(9.9)[0], None);
        assert_eq!(
            meter_bars(70.0),
            [
                Some("green"),
                Some("green"),
                Some("green"),
                Some("yellow"),
                None
            ]
        );
        assert_eq!(meter_bars(100.0)[4], Some("red"));
    }

    #[test]
    fn countdown_choices_are_labelled() {
        assert_eq!(countdown_label(0), "No delay");
        assert_eq!(countdown_label(3), "3 seconds");
    }

    #[test]
    fn popovers_stay_inside_the_window() {
        for pop in [
            Pop::Source,
            Pop::Audio,
            Pop::Webcam,
            Pop::Countdown,
            Pop::More,
            Pop::ConfirmCancel,
        ] {
            let left = pop.left();
            assert!(
                left >= 8.0 && left + POP_WIDTH <= BAR_WIDTH - 8.0,
                "{pop:?} at {left}"
            );
        }
    }
}
