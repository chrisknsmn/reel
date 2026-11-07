//! Keyboard shortcuts (§23), kept apart from the UI so they can be tested.

use std::collections::BTreeMap;

/// Everything a key can do in the editor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    AddZoom,
    Split,
    AddAnnotation,
    AddKeyframe,
    DeleteSelected,
    PlayPause,
    Undo,
    Redo,
    SelectAllZooms,
    NextAnnotation,
    PreviousAnnotation,
    Cancel,
    OpenProjects,
    Save,
    SaveAs,
}

/// The ones a user can change, with their names in settings and on screen, and their defaults.
pub const REBINDABLE: [(Action, &str, &str, &str); 6] = [
    (Action::AddZoom, "add_zoom", "Add Zoom", "Z"),
    (Action::Split, "split", "Split Clip", "C"),
    (
        Action::AddAnnotation,
        "add_annotation",
        "Add Annotation",
        "A",
    ),
    (Action::AddKeyframe, "add_keyframe", "Add Keyframe", "F"),
    (
        Action::DeleteSelected,
        "delete_selected",
        "Delete Selected",
        "Cmd+D",
    ),
    (Action::PlayPause, "play_pause", "Play / Pause", "Space"),
];

/// Keys that always do the same thing, so nothing else can use them.
pub const FIXED: [(&str, Action); 12] = [
    ("Cmd+Z", Action::Undo),
    ("Cmd+Shift+Z", Action::Redo),
    ("Delete", Action::DeleteSelected),
    ("Backspace", Action::DeleteSelected),
    ("Cmd+A", Action::SelectAllZooms),
    ("Tab", Action::NextAnnotation),
    ("Shift+Tab", Action::PreviousAnnotation),
    ("Esc", Action::Cancel),
    ("Cmd+O", Action::OpenProjects),
    ("Cmd+S", Action::Save),
    ("Cmd+Shift+S", Action::SaveAs),
    // Windows and Linux use this for redo too.
    ("Ctrl+Y", Action::Redo),
];

/// A key press as text, such as `Cmd+Shift+Z`.
#[must_use]
pub fn describe(key: &str, meta: bool, ctrl: bool, alt: bool, shift: bool) -> String {
    let name = match key {
        " " => "Space".to_string(),
        "Escape" => "Esc".to_string(),
        other if other.chars().count() == 1 => other.to_uppercase(),
        other => other.to_string(),
    };
    let mut text = String::new();
    // Cmd on a Mac, and Ctrl elsewhere, mean the same in these shortcuts.
    if meta || (ctrl && !cfg!(target_os = "macos")) {
        text.push_str("Cmd+");
    } else if ctrl {
        text.push_str("Ctrl+");
    }
    if alt {
        text.push_str("Alt+");
    }
    if shift {
        text.push_str("Shift+");
    }
    text.push_str(&name);
    text
}

/// Whether a key press is only a modifier held down, which isn't a shortcut yet.
#[must_use]
pub fn is_modifier(key: &str) -> bool {
    matches!(
        key,
        "Shift" | "Meta" | "Control" | "Alt" | "CapsLock" | "Fn"
    )
}

/// What a key does, given what's been rebound.
#[must_use]
pub fn action_for(pressed: &str, bound: &BTreeMap<String, String>) -> Option<Action> {
    FIXED
        .iter()
        .find(|(key, _)| *key == pressed)
        .map(|(_, action)| *action)
        .or_else(|| {
            REBINDABLE.iter().find_map(|(action, name, _, default)| {
                let key = bound.get(*name).map_or(*default, String::as_str);
                (key == pressed).then_some(*action)
            })
        })
}

/// What rebinding a shortcut would run into.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Rebind {
    Fine,
    /// Fixed keys are reserved (§23).
    Reserved,
    /// Another shortcut has it, which it could swap with.
    Conflict {
        with: &'static str,
        label: &'static str,
    },
}

/// Checks a new key for a shortcut.
#[must_use]
pub fn check_rebind(name: &str, pressed: &str, bound: &BTreeMap<String, String>) -> Rebind {
    if FIXED.iter().any(|(key, _)| *key == pressed) {
        return Rebind::Reserved;
    }
    REBINDABLE
        .iter()
        .filter(|(_, other, _, _)| *other != name)
        .find(|(_, other, _, default)| {
            bound.get(*other).map_or(*default, String::as_str) == pressed
        })
        .map_or(Rebind::Fine, |(_, other, label, _)| Rebind::Conflict {
            with: other,
            label,
        })
}

/// The key currently set for a shortcut.
#[must_use]
pub fn current_key<'a>(
    name: &str,
    default: &'a str,
    bound: &'a BTreeMap<String, String>,
) -> &'a str {
    bound.get(name).map_or(default, String::as_str)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_presses_read_as_their_names() {
        assert_eq!(describe("z", false, false, false, false), "Z");
        assert_eq!(describe("z", true, false, false, true), "Cmd+Shift+Z");
        assert_eq!(describe(" ", false, false, false, false), "Space");
        assert_eq!(describe("Escape", false, false, false, false), "Esc");
        assert_eq!(describe("Tab", false, false, false, true), "Shift+Tab");
        assert_eq!(describe("F5", false, false, true, false), "Alt+F5");
    }

    #[test]
    fn modifiers_on_their_own_arent_shortcuts() {
        assert!(is_modifier("Shift") && is_modifier("Meta"));
        assert!(!is_modifier("Z"));
    }

    #[test]
    fn the_documented_defaults_work() {
        let none = BTreeMap::new();
        assert_eq!(action_for("Z", &none), Some(Action::AddZoom));
        assert_eq!(action_for("C", &none), Some(Action::Split));
        assert_eq!(action_for("A", &none), Some(Action::AddAnnotation));
        assert_eq!(action_for("F", &none), Some(Action::AddKeyframe));
        assert_eq!(action_for("Cmd+D", &none), Some(Action::DeleteSelected));
        assert_eq!(action_for("Space", &none), Some(Action::PlayPause));
        assert_eq!(action_for("Cmd+Z", &none), Some(Action::Undo));
        assert_eq!(action_for("Cmd+Shift+Z", &none), Some(Action::Redo));
        assert_eq!(action_for("Delete", &none), Some(Action::DeleteSelected));
        assert_eq!(action_for("Q", &none), None);
    }

    #[test]
    fn rebinding_moves_a_shortcut_to_its_new_key() {
        let bound = BTreeMap::from([("add_zoom".to_string(), "Q".to_string())]);
        assert_eq!(action_for("Q", &bound), Some(Action::AddZoom));
        assert_eq!(action_for("Z", &bound), None);
    }

    #[test]
    fn fixed_keys_cant_be_taken() {
        assert_eq!(
            check_rebind("add_zoom", "Cmd+Z", &BTreeMap::new()),
            Rebind::Reserved
        );
        assert_eq!(
            check_rebind("add_zoom", "Esc", &BTreeMap::new()),
            Rebind::Reserved
        );
    }

    #[test]
    fn conflicts_name_what_they_conflict_with() {
        let none = BTreeMap::new();
        assert_eq!(
            check_rebind("add_zoom", "C", &none),
            Rebind::Conflict {
                with: "split",
                label: "Split Clip"
            }
        );
        assert_eq!(check_rebind("add_zoom", "Q", &none), Rebind::Fine);
        assert_eq!(
            check_rebind("add_zoom", "Z", &none),
            Rebind::Fine,
            "its own key is no conflict"
        );
    }

    #[test]
    fn no_default_uses_a_reserved_key() {
        for (_, name, _, default) in REBINDABLE {
            assert_eq!(
                check_rebind(name, default, &BTreeMap::new()),
                Rebind::Fine,
                "{name}"
            );
        }
    }
}
