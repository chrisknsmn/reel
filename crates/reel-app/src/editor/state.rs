//! What every part of the editor shares: the open project, and what's happening to it.

use std::path::PathBuf;
use std::time::Instant;

use dioxus::prelude::*;
use reel_core::edit::{Edit, Outcome};
use reel_core::history::{Gesture, Selection};
use reel_core::project::Project;
use reel_core::session::Session;
use reel_core::time::{Micros, TimelineTime};

use super::math::View;
use crate::preview;

/// Which panel the icon rail has chosen (§7).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rail {
    Scene,
    Cursor,
    Webcam,
    Captions,
    Settings,
}

/// The dialog or side panel that's open, if any.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Overlay {
    None,
    Crop,
    Export,
    Projects,
    Shortcuts,
    Presets,
}

/// Shared with every component in the editor through the context.
#[derive(Clone, Copy, PartialEq)]
pub struct Ed {
    pub session: Signal<Option<Session>>,
    pub view: Signal<View>,
    pub position: Signal<Micros>,
    pub playing: Signal<bool>,
    pub notice: Signal<Option<String>>,
    pub rail: Signal<Rail>,
    pub overlay: Signal<Overlay>,
    /// Markers only: they aren't saved and can't be undone (§14).
    pub keyframes: Signal<Vec<TimelineTime>>,
    /// Where the preview's clear area is, in CSS pixels.
    pub hole: Signal<(f64, f64, f64, f64)>,
    pub dark: Signal<bool>,
    /// Whether something on the timeline is being dragged.
    pub dragging: Signal<bool>,
}

impl Ed {
    /// The project as it stands. Reading this subscribes to changes.
    #[must_use]
    pub fn project(&self) -> Option<Project> {
        self.session
            .read()
            .as_ref()
            .map(|session| session.editor.project().clone())
    }

    #[must_use]
    pub fn selection(&self) -> Selection {
        self.session
            .read()
            .as_ref()
            .map_or(Selection::None, |session| {
                session.editor.selection().clone()
            })
    }

    pub fn select(mut self, selection: Selection) {
        if let Some(session) = self.session.write().as_mut() {
            session.editor.select(selection);
        }
    }

    #[must_use]
    pub fn total(&self) -> Micros {
        self.session
            .read()
            .as_ref()
            .map_or(Micros::ZERO, |session| {
                session.editor.project().timeline.duration()
            })
    }

    #[must_use]
    pub fn now(&self) -> TimelineTime {
        TimelineTime::ZERO + *self.position.read()
    }

    fn after_change(session: &mut Session) {
        session.touch(Instant::now());
        preview::update(session.editor.project().clone());
    }

    /// Makes an edit, as its own undo step. A refused edit explains itself in a notice.
    pub fn apply(mut self, edit: &Edit) -> Option<Outcome> {
        let result = {
            let mut guard = self.session.write();
            let session = guard.as_mut()?;
            let result = session.editor.apply(edit);
            if result.is_ok() {
                Self::after_change(session);
            }
            result
        };
        match result {
            Ok(outcome) => Some(outcome),
            Err(error) => {
                self.notice.set(Some(error.to_string()));
                None
            }
        }
    }

    /// Makes a change that's part of a longer gesture, like dragging a slider. All the changes with
    /// the same name become one undo step.
    pub fn change(mut self, gesture: &str, update: impl FnOnce(&mut Project)) {
        let mut guard = self.session.write();
        if let Some(session) = guard.as_mut() {
            session
                .editor
                .change(Some(Gesture(gesture.to_string())), update);
            Self::after_change(session);
        }
    }

    /// Ends a gesture, so the next change is a new undo step.
    pub fn end_gesture(mut self) {
        if let Some(session) = self.session.write().as_mut() {
            session.editor.end_gesture();
        }
    }

    pub fn undo(mut self) {
        if let Some(session) = self.session.write().as_mut() {
            session.editor.undo();
            Self::after_change(session);
        }
    }

    pub fn redo(mut self) {
        if let Some(session) = self.session.write().as_mut() {
            session.editor.redo();
            Self::after_change(session);
        }
    }

    pub fn seek(mut self, time: TimelineTime) {
        preview::seek(time);
        self.position.set(time - TimelineTime::ZERO);
    }

    /// The folder relative paths in the project are measured from.
    #[must_use]
    pub fn folder(&self) -> Option<PathBuf> {
        self.session
            .read()
            .as_ref()
            .and_then(|session| session.folder().map(std::path::Path::to_path_buf))
    }
}

/// Reads the shared state, inside any editor component.
#[must_use]
pub fn use_ed() -> Ed {
    use_context::<Ed>()
}
