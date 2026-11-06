//! The document being edited, with undo over all of it (§18, BUILD_PLAN 2.4).
//!
//! Recordly's undo leaves out the look settings and crop, and spends a step on every selection
//! change and keystroke (§58). Here a step is a snapshot of the whole project, a gesture like a
//! slider drag merges into one step, and selection lives outside history altogether.

use crate::edit::{Edit, EditError, Outcome};
use crate::project::Project;
use crate::timeline::ItemId;

/// How many steps back Undo goes (§18).
pub const UNDO_LEVELS: usize = 100;

/// What's selected. This isn't part of the document, so changing it never touches history.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Selection {
    #[default]
    None,
    Clip(ItemId),
    Zoom(ItemId),
    AllZooms,
    Annotation(ItemId),
    Audio(ItemId),
    Caption(ItemId),
}

/// Identifies a gesture, such as one slider's drag or typing in one field. Changes with the same
/// gesture merge into a single undo step until `Editor::end_gesture`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Gesture(pub String);

#[derive(Clone, Debug)]
pub struct Editor {
    project: Project,
    selection: Selection,
    undo: Vec<Project>,
    redo: Vec<Project>,
    gesture: Option<Gesture>,
}

impl Editor {
    /// Starts editing with an empty history, so what the project opened with can't be undone. In
    /// Recordly the automatic clip is an undo step, so Undo can empty the clip row (§58).
    #[must_use]
    pub fn new(project: Project) -> Self {
        Self {
            project,
            selection: Selection::None,
            undo: Vec::new(),
            redo: Vec::new(),
            gesture: None,
        }
    }

    #[must_use]
    pub fn project(&self) -> &Project {
        &self.project
    }

    #[must_use]
    pub fn selection(&self) -> &Selection {
        &self.selection
    }

    pub fn select(&mut self, selection: Selection) {
        self.selection = selection;
    }

    #[must_use]
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    #[must_use]
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Applies a timeline edit as one undo step. A refused edit leaves history alone.
    ///
    /// # Errors
    /// Returns why the edit wasn't allowed.
    pub fn apply(&mut self, edit: &Edit) -> Result<Outcome, EditError> {
        let before = self.project.clone();
        let outcome = self.project.timeline.apply(edit)?;
        self.gesture = None;
        self.push_step(before);
        Ok(outcome)
    }

    /// Changes anything in the project, such as a look setting or the crop. Changes with the same
    /// gesture merge into one step, and a change that changes nothing adds no step.
    pub fn change(&mut self, gesture: Option<Gesture>, update: impl FnOnce(&mut Project)) {
        let before = self.project.clone();
        update(&mut self.project);
        if self.project == before {
            return;
        }
        let merges = gesture.is_some() && gesture == self.gesture && !self.undo.is_empty();
        self.gesture = gesture;
        if merges {
            // The step already holds how things were before this gesture began.
            self.redo.clear();
        } else {
            self.push_step(before);
        }
    }

    /// Ends the current gesture, such as when a slider is released.
    pub fn end_gesture(&mut self) {
        self.gesture = None;
    }

    pub fn undo(&mut self) {
        if let Some(previous) = self.undo.pop() {
            self.redo
                .push(std::mem::replace(&mut self.project, previous));
            self.after_history_move();
        }
    }

    pub fn redo(&mut self) {
        if let Some(next) = self.redo.pop() {
            self.undo.push(std::mem::replace(&mut self.project, next));
            self.after_history_move();
        }
    }

    fn push_step(&mut self, before: Project) {
        self.undo.push(before);
        if self.undo.len() > UNDO_LEVELS {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    fn after_history_move(&mut self) {
        self.gesture = None;
        let timeline = &self.project.timeline;
        let exists = match &self.selection {
            Selection::None | Selection::AllZooms => true,
            Selection::Clip(id) => timeline.clips.iter().any(|item| item.id == *id),
            Selection::Zoom(id) => timeline.zooms.iter().any(|item| item.id == *id),
            Selection::Annotation(id) => timeline.annotations.iter().any(|item| item.id == *id),
            Selection::Audio(id) => timeline.audio.iter().any(|item| item.id == *id),
            Selection::Caption(id) => timeline.captions.iter().any(|item| item.id == *id),
        };
        if !exists {
            self.selection = Selection::None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edit::Edit;
    use crate::time::{Micros, TimelineTime};
    use crate::timeline::Timeline;
    use proptest::prelude::*;

    fn editor() -> Editor {
        Editor::new(Project {
            timeline: Timeline::for_recording(Micros::from_secs(30)),
            ..Project::default()
        })
    }

    fn split(at: f64) -> Edit {
        Edit::SplitClip {
            at: TimelineTime::from_ms(at),
        }
    }

    #[test]
    fn opening_a_project_gives_nothing_to_undo() {
        // In Recordly the automatic clip is recorded, so Undo can empty the clip row (§58).
        let mut editor = editor();
        assert!(!editor.can_undo());
        editor.undo();
        assert_eq!(editor.project().timeline.clips.len(), 1);
    }

    #[test]
    fn edits_undo_and_redo() {
        let mut editor = editor();
        editor.apply(&split(10_000.0)).unwrap();
        assert_eq!(editor.project().timeline.clips.len(), 2);
        editor.undo();
        assert_eq!(editor.project().timeline.clips.len(), 1);
        editor.redo();
        assert_eq!(editor.project().timeline.clips.len(), 2);
    }

    #[test]
    fn selection_changes_use_up_no_undo_steps() {
        let mut editor = editor();
        editor.apply(&split(10_000.0)).unwrap();
        editor.select(Selection::Clip("clip-1".into()));
        editor.select(Selection::Clip("clip-2".into()));
        editor.select(Selection::None);
        editor.undo();
        assert!(
            !editor.can_undo(),
            "one edit is one step, whatever was selected in between"
        );
    }

    #[test]
    fn look_settings_and_crop_can_be_undone() {
        // Recordly leaves these out of history (§18).
        let mut editor = editor();
        editor.change(None, |project| project.look.shadow = 0.1);
        editor.change(None, |project| project.look.crop.width = 0.5);
        editor.undo();
        assert_eq!(editor.project().look.crop.width, 1.0);
        assert_eq!(editor.project().look.shadow, 0.1);
        editor.undo();
        assert_eq!(editor.project().look.shadow, 0.67);
    }

    #[test]
    fn a_slider_drag_is_one_step() {
        let mut editor = editor();
        let gesture = Some(Gesture("look.shadow".to_string()));
        for value in [0.1, 0.2, 0.3, 0.4] {
            editor.change(gesture.clone(), |project| project.look.shadow = value);
        }
        editor.end_gesture();
        editor.undo();
        assert_eq!(editor.project().look.shadow, 0.67);
        assert!(!editor.can_undo());
    }

    #[test]
    fn typing_is_one_step_until_the_gesture_ends() {
        let mut editor = editor();
        let gesture = Some(Gesture("caption-1.text".to_string()));
        for text in ["h", "he", "hel"] {
            editor.change(gesture.clone(), |project| {
                project.caption_style.language = text.to_string()
            });
        }
        editor.end_gesture();
        editor.change(gesture.clone(), |project| {
            project.caption_style.language = "hello".to_string()
        });
        editor.undo();
        assert_eq!(editor.project().caption_style.language, "hel");
        editor.undo();
        assert_eq!(editor.project().caption_style.language, "auto");
    }

    #[test]
    fn a_different_gesture_starts_a_new_step() {
        let mut editor = editor();
        editor.change(Some(Gesture("a".to_string())), |project| {
            project.look.shadow = 0.1
        });
        editor.change(Some(Gesture("b".to_string())), |project| {
            project.look.corner_radius = 1.0
        });
        editor.undo();
        assert_eq!(editor.project().look.corner_radius, 12.5);
        assert_eq!(editor.project().look.shadow, 0.1);
    }

    #[test]
    fn a_change_that_changes_nothing_adds_no_step() {
        let mut editor = editor();
        editor.change(None, |project| project.look.shadow = 0.67);
        assert!(!editor.can_undo());
    }

    #[test]
    fn a_refused_edit_adds_no_step() {
        let mut editor = editor();
        assert!(editor.apply(&split(50.0)).is_err());
        assert!(!editor.can_undo());
    }

    #[test]
    fn a_new_edit_clears_redo() {
        let mut editor = editor();
        editor.apply(&split(10_000.0)).unwrap();
        editor.undo();
        editor.change(None, |project| project.look.shadow = 0.2);
        assert!(!editor.can_redo());
    }

    #[test]
    fn undo_stops_at_its_limit() {
        let mut editor = editor();
        for n in 0..(UNDO_LEVELS + 20) {
            editor.change(None, |project| project.look.corner_radius = n as f64 + 20.0);
        }
        let mut steps = 0;
        while editor.can_undo() {
            editor.undo();
            steps += 1;
        }
        assert_eq!(steps, UNDO_LEVELS);
    }

    #[test]
    fn undo_clears_a_selection_that_no_longer_exists() {
        let mut editor = editor();
        let created = editor.apply(&split(10_000.0)).unwrap().created.unwrap();
        editor.select(Selection::Clip(created));
        editor.undo();
        assert_eq!(*editor.selection(), Selection::None);
    }

    proptest! {
        #[test]
        fn undoing_everything_returns_to_the_start_and_redo_returns_to_the_end(
            values in prop::collection::vec((any::<bool>(), 1_000_i64..29_000), 1..30)
        ) {
            let mut editor = editor();
            let start = editor.project().clone();
            for (is_edit, value) in values {
                if is_edit {
                    let _ = editor.apply(&split(value as f64));
                } else {
                    editor.change(None, |project| project.look.corner_radius = value as f64 / 1000.0);
                }
            }
            let end = editor.project().clone();
            while editor.can_undo() {
                editor.undo();
            }
            prop_assert_eq!(editor.project(), &start);
            while editor.can_redo() {
                editor.redo();
            }
            prop_assert_eq!(editor.project(), &end);
        }
    }
}
