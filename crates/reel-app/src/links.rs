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

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ToBar {
    /// The editor has closed, so the bar comes back (§48).
    EditorClosed,
    /// The editor is on screen with something open, so the bar gets out of the way.
    EditorOpened,
    /// A file couldn't be opened, and the editor isn't on screen to say so.
    OpenFailed(String),
    /// The countdown was cancelled with a click or Esc.
    CancelCountdown,
}

/// A window's inbox. What's sent before the window is listening waits for it, since a file given at
/// launch can finish opening before the bar's window has loaded.
enum Inbox<T> {
    Waiting(Vec<T>),
    Listening(UnboundedSender<T>),
}

static EDITOR: Mutex<Inbox<ToEditor>> = Mutex::new(Inbox::Waiting(Vec::new()));
static BAR: Mutex<Inbox<ToBar>> = Mutex::new(Inbox::Waiting(Vec::new()));

fn store<T>(slot: &Mutex<Inbox<T>>) -> UnboundedReceiver<T> {
    let (send, receive) = unbounded_channel();
    if let Ok(mut slot) = slot.lock() {
        if let Inbox::Waiting(waiting) = &mut *slot {
            for message in waiting.drain(..) {
                let _ = send.send(message);
            }
        }
        *slot = Inbox::Listening(send);
    }
    receive
}

fn post<T>(slot: &Mutex<Inbox<T>>, message: T) {
    if let Ok(mut slot) = slot.lock() {
        match &mut *slot {
            Inbox::Waiting(waiting) => waiting.push(message),
            Inbox::Listening(send) => {
                let _ = send.send(message);
            }
        }
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
