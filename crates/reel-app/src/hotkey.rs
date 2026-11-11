//! A global shortcut to start and stop recording, from any app.

use std::cell::RefCell;

use global_hotkey::hotkey::{Code, HotKey, Modifiers};
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};

thread_local! {
    /// It has to stay alive, on the thread that made it, for the shortcut to keep working.
    static MANAGER: RefCell<Option<GlobalHotKeyManager>> = const { RefCell::new(None) };
}

/// Option-Command-R. Registering needs no permission. If something else already has it, the app
/// carries on without one.
pub fn register() {
    let Ok(manager) = GlobalHotKeyManager::new() else {
        return;
    };
    let hotkey = HotKey::new(Some(Modifiers::ALT | Modifiers::META), Code::KeyR);
    if manager.register(hotkey).is_ok() {
        MANAGER.with_borrow_mut(|slot| *slot = Some(manager));
    }
}

/// Whether it's been pressed since this was last asked.
#[must_use]
pub fn was_pressed() -> bool {
    let mut pressed = false;
    while let Ok(event) = GlobalHotKeyEvent::receiver().try_recv() {
        pressed |= event.state == HotKeyState::Pressed;
    }
    pressed
}
