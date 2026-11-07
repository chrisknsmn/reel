//! How the app's windows talk to each other. Each window runs its own Dioxus tree, so signals don't
//! reach across them.

use std::path::PathBuf;
use std::sync::Mutex;

use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

#[derive(Clone, Debug, PartialEq)]
pub enum ToEditor {
    /// Open this project or video, and come to the front.
    Open(PathBuf),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToBar {
    /// The editor has closed, so the bar comes back (§48).
    EditorClosed,
    /// The countdown was cancelled with a click or Esc.
    CancelCountdown,
}

static EDITOR: Mutex<Option<UnboundedSender<ToEditor>>> = Mutex::new(None);
static BAR: Mutex<Option<UnboundedSender<ToBar>>> = Mutex::new(None);

fn store<T>(slot: &Mutex<Option<UnboundedSender<T>>>) -> UnboundedReceiver<T> {
    let (send, receive) = unbounded_channel();
    if let Ok(mut slot) = slot.lock() {
        *slot = Some(send);
    }
    receive
}

fn post<T>(slot: &Mutex<Option<UnboundedSender<T>>>, message: T) {
    if let Ok(slot) = slot.lock()
        && let Some(send) = slot.as_ref()
    {
        let _ = send.send(message);
    }
}

/// The editor calls this once, to start receiving.
pub fn listen_as_editor() -> UnboundedReceiver<ToEditor> {
    store(&EDITOR)
}

/// The bar calls this once, to start receiving.
pub fn listen_as_bar() -> UnboundedReceiver<ToBar> {
    store(&BAR)
}

pub fn tell_editor(message: ToEditor) {
    post(&EDITOR, message);
}

pub fn tell_bar(message: ToBar) {
    post(&BAR, message);
}
