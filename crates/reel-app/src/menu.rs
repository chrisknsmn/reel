//! The macOS menu (§49). Dioxus's default one gives Edit a native Undo and Select All, which would catch
//! those keys before the page sees them, and adds developer tools (BUILD_PLAN 10.2). So this one sends
//! them to the editor, and has no developer tools at all.

use dioxus::desktop::muda::accelerator::{Accelerator, Code, Modifiers};
use dioxus::desktop::muda::{Menu, MenuItem, PredefinedMenuItem, Submenu};

pub const OPEN: &str = "open-projects";
pub const SAVE: &str = "save";
pub const SAVE_AS: &str = "save-as";
pub const UNDO: &str = "undo";
pub const REDO: &str = "redo";
pub const SELECT_ALL: &str = "select-all";

fn item(id: &str, label: &str, modifiers: Modifiers, key: Code) -> MenuItem {
    MenuItem::with_id(
        id,
        label,
        true,
        Some(Accelerator::new(Some(modifiers), key)),
    )
}

/// Builds the menu bar. It can only fail if AppKit refuses an item, which is a bug, not a condition.
#[must_use]
pub fn build() -> Menu {
    let menu = Menu::new();
    let command = Modifiers::SUPER;
    let command_shift = Modifiers::SUPER | Modifiers::SHIFT;

    let app = Submenu::new("Reel", true);
    let file = Submenu::new("File", true);
    let edit = Submenu::new("Edit", true);
    let window = Submenu::new("Window", true);
    let built = (|| -> Result<(), dioxus::desktop::muda::Error> {
        app.append_items(&[
            &PredefinedMenuItem::about(None, None),
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::services(None),
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::hide(None),
            &PredefinedMenuItem::hide_others(None),
            &PredefinedMenuItem::show_all(None),
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::quit(None),
        ])?;
        file.append_items(&[
            &item(OPEN, "Open Projects…", command, Code::KeyO),
            &PredefinedMenuItem::separator(),
            &item(SAVE, "Save", command, Code::KeyS),
            &item(SAVE_AS, "Save As…", command_shift, Code::KeyS),
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::close_window(None),
        ])?;
        edit.append_items(&[
            &item(UNDO, "Undo", command, Code::KeyZ),
            &item(REDO, "Redo", command_shift, Code::KeyZ),
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::cut(None),
            &PredefinedMenuItem::copy(None),
            &PredefinedMenuItem::paste(None),
            &item(SELECT_ALL, "Select All", command, Code::KeyA),
        ])?;
        window.append_items(&[
            &PredefinedMenuItem::minimize(None),
            &PredefinedMenuItem::maximize(None),
            &PredefinedMenuItem::fullscreen(None),
        ])?;
        menu.append_items(&[&app, &file, &edit, &window])
    })();
    if let Err(error) = built {
        eprintln!("The menu couldn't be built: {error}");
    }
    menu
}
