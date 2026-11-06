//! A project open for editing, with saving, autosave and unsaved-change tracking (§20).

use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::format::{self, EXTENSION, LoadError};
use crate::history::Editor;
use crate::project::Project;
use crate::store;

/// How long after a change autosave waits (§20).
pub const AUTOSAVE_DELAY: Duration = Duration::from_secs(1);

#[derive(Debug)]
pub enum SessionError {
    Io(io::Error),
    Load(LoadError),
    /// A different project already has this name, so saving would destroy it (§20).
    NameTaken(PathBuf),
    /// Nothing to save to yet: use Save As.
    NoPath,
}

impl fmt::Display for SessionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "The file couldn't be written: {error}"),
            Self::Load(error) => write!(f, "{error}"),
            Self::NameTaken(path) => {
                write!(
                    f,
                    "A different project is already saved as {}.",
                    path.display()
                )
            }
            Self::NoPath => write!(f, "This project hasn't been saved yet."),
        }
    }
}

impl std::error::Error for SessionError {}

impl From<io::Error> for SessionError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<LoadError> for SessionError {
    fn from(error: LoadError) -> Self {
        Self::Load(error)
    }
}

pub struct Session {
    pub editor: Editor,
    path: Option<PathBuf>,
    /// The project as it was last saved or opened.
    saved: Option<Project>,
    last_change: Option<Instant>,
}

impl Session {
    /// A new recording, which has no file yet.
    #[must_use]
    pub fn new(project: Project) -> Self {
        Self {
            editor: Editor::new(project),
            path: None,
            saved: None,
            last_change: None,
        }
    }

    /// Opens a project. This reads the file and nothing else: in Recordly, opening a project marks
    /// it dirty and rewrites it a second later (§58).
    ///
    /// # Errors
    /// Returns why the file can't be read.
    pub fn open(path: &Path) -> Result<Self, SessionError> {
        let text = fs::read_to_string(path)?;
        let project = format::from_str(&text)?;
        Ok(Self {
            editor: Editor::new(project.clone()),
            path: Some(path.to_path_buf()),
            saved: Some(project),
            last_change: None,
        })
    }

    #[must_use]
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// The folder relative media paths start from.
    #[must_use]
    pub fn folder(&self) -> Option<&Path> {
        self.path.as_deref().and_then(Path::parent)
    }

    /// Whether anything would be lost by closing now. A new recording counts until it's saved.
    #[must_use]
    pub fn is_dirty(&self) -> bool {
        self.saved.as_ref() != Some(self.editor.project())
    }

    /// Call after anything changes the project, to time autosave from.
    pub fn touch(&mut self, now: Instant) {
        self.last_change = Some(now);
    }

    /// Whether it's time to autosave: unsaved changes, and a second since the last one.
    #[must_use]
    pub fn autosave_due(&self, now: Instant) -> bool {
        self.is_dirty()
            && self
                .last_change
                .is_none_or(|changed| now.duration_since(changed) >= AUTOSAVE_DELAY)
    }

    /// Saves to the project's file, or for a new recording, creates one in this folder under a name
    /// that isn't taken. Recordly never autosaves a new recording, so a crash loses all edits (§20).
    ///
    /// # Errors
    /// Returns any error from writing the file.
    pub fn autosave(
        &mut self,
        projects_folder: &Path,
        default_name: &str,
    ) -> Result<(), SessionError> {
        if self.path.is_none() {
            self.path = Some(unused_path(projects_folder, default_name));
        }
        self.save()
    }

    /// Saves to the project's file.
    ///
    /// # Errors
    /// Returns `NoPath` if it has no file yet, or any error from writing it.
    pub fn save(&mut self) -> Result<(), SessionError> {
        let path = self.path.clone().ok_or(SessionError::NoPath)?;
        self.write(&path)?;
        Ok(())
    }

    /// Saves under a new path, refusing to overwrite a different project.
    ///
    /// # Errors
    /// Returns `NameTaken` if a different project is there, or any error from writing the file.
    pub fn save_as(&mut self, path: &Path) -> Result<(), SessionError> {
        if self.path.as_deref() != Some(path) && path.exists() {
            let same = fs::read_to_string(path)
                .ok()
                .and_then(|text| format::from_str(&text).ok())
                .is_some_and(|other| other.id == self.editor.project().id);
            if !same {
                return Err(SessionError::NameTaken(path.to_path_buf()));
            }
        }
        self.write(path)?;
        self.path = Some(path.to_path_buf());
        Ok(())
    }

    /// Renames the project's file, removing the old file and its backup (§20).
    ///
    /// # Errors
    /// Returns `NameTaken` if a different project has the name, or any error from the file system.
    pub fn rename(&mut self, name: &str) -> Result<(), SessionError> {
        let old = self.path.clone().ok_or(SessionError::NoPath)?;
        let folder = old.parent().unwrap_or(Path::new("."));
        let new = folder.join(format!("{}.{EXTENSION}", store::safe_file_name(name)));
        if new == old {
            return Ok(());
        }
        self.save_as(&new)?;
        let _ = fs::remove_file(&old);
        let _ = fs::remove_file(store::backup_path(&old));
        Ok(())
    }

    fn write(&mut self, path: &Path) -> Result<(), SessionError> {
        let mut file = self.editor.project().clone();
        if let Some(folder) = path.parent() {
            let absolute = fs::canonicalize(folder).unwrap_or_else(|_| folder.to_path_buf());
            file.rebase_media(&absolute);
        }
        store::atomic_write(path, format::to_string(&file).as_bytes(), true)?;
        self.saved = Some(self.editor.project().clone());
        self.last_change = None;
        Ok(())
    }
}

/// A path in this folder with this name, adding a number if it's taken.
fn unused_path(folder: &Path, name: &str) -> PathBuf {
    let name = store::safe_file_name(name);
    let first = folder.join(format!("{name}.{EXTENSION}"));
    if !first.exists() {
        return first;
    }
    (2..)
        .map(|n| folder.join(format!("{name} {n}.{EXTENSION}")))
        .find(|path| !path.exists())
        .expect("an unused name always exists")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::MediaRef;
    use crate::time::Micros;
    use crate::timeline::Timeline;

    fn project(id: &str) -> Project {
        let mut project = Project {
            id: id.to_string(),
            timeline: Timeline::for_recording(Micros::from_secs(30)),
            ..Project::default()
        };
        project.media.video = MediaRef::absolute("/somewhere/video.mp4");
        project
    }

    #[test]
    fn opening_a_project_leaves_it_clean_and_the_file_untouched() {
        // Recordly reads as dirty and rewrites the file a second after opening (§58).
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.reel");
        let mut session = Session::new(project("a"));
        session.save_as(&path).unwrap();
        let written = fs::read(&path).unwrap();
        let modified = fs::metadata(&path).unwrap().modified().unwrap();

        let session = Session::open(&path).unwrap();
        assert!(!session.is_dirty());
        assert!(!session.autosave_due(Instant::now() + Duration::from_secs(60)));
        assert_eq!(fs::read(&path).unwrap(), written);
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
    }

    #[test]
    fn changing_something_makes_it_dirty_and_saving_clears_that() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.reel");
        let mut session = Session::new(project("a"));
        session.save_as(&path).unwrap();
        assert!(!session.is_dirty());
        session
            .editor
            .change(None, |project| project.look.shadow = 0.1);
        assert!(session.is_dirty());
        session.save().unwrap();
        assert!(!session.is_dirty());
        assert_eq!(
            Session::open(&path).unwrap().editor.project().look.shadow,
            0.1
        );
    }

    #[test]
    fn undoing_back_to_the_saved_state_is_clean_again() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.reel");
        let mut session = Session::new(project("a"));
        session.save_as(&path).unwrap();
        session
            .editor
            .change(None, |project| project.look.shadow = 0.1);
        session.editor.undo();
        assert!(!session.is_dirty());
    }

    #[test]
    fn autosave_waits_a_second_after_the_last_change() {
        let mut session = Session::new(project("a"));
        let start = Instant::now();
        session
            .editor
            .change(None, |project| project.look.shadow = 0.1);
        session.touch(start);
        assert!(!session.autosave_due(start + Duration::from_millis(500)));
        assert!(session.autosave_due(start + Duration::from_millis(1000)));
    }

    #[test]
    fn a_new_recording_autosaves_too() {
        // Recordly only autosaves projects that already have a file (§20).
        let dir = tempfile::tempdir().unwrap();
        let mut session = Session::new(project("a"));
        assert!(session.is_dirty());
        session.autosave(dir.path(), "Recording").unwrap();
        let path = session.path().unwrap().to_path_buf();
        assert_eq!(path, dir.path().join("Recording.reel"));
        assert!(!session.is_dirty());
        assert_eq!(Session::open(&path).unwrap().editor.project().id, "a");
    }

    #[test]
    fn an_autosaved_recording_never_takes_another_projects_name() {
        let dir = tempfile::tempdir().unwrap();
        let mut first = Session::new(project("a"));
        first.autosave(dir.path(), "Recording").unwrap();
        let mut second = Session::new(project("b"));
        second.autosave(dir.path(), "Recording").unwrap();
        assert_eq!(second.path().unwrap(), dir.path().join("Recording 2.reel"));
        assert_eq!(
            Session::open(first.path().unwrap())
                .unwrap()
                .editor
                .project()
                .id,
            "a"
        );
    }

    #[test]
    fn save_as_refuses_to_overwrite_a_different_project() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.reel");
        Session::new(project("a")).save_as(&path).unwrap();
        let result = Session::new(project("b")).save_as(&path);
        assert!(matches!(result, Err(SessionError::NameTaken(_))));
        assert_eq!(Session::open(&path).unwrap().editor.project().id, "a");
    }

    #[test]
    fn renaming_moves_the_file_and_removes_the_old_one_and_its_backup() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.reel");
        let mut session = Session::new(project("a"));
        session.save_as(&path).unwrap();
        session.save().unwrap();
        assert!(store::backup_path(&path).exists());
        session.rename("b/c").unwrap();
        assert_eq!(session.path().unwrap(), dir.path().join("bc.reel"));
        assert!(!path.exists());
        assert!(!store::backup_path(&path).exists());
    }

    #[test]
    fn media_is_found_after_the_project_folder_moves() {
        let dir = tempfile::tempdir().unwrap();
        let old = dir.path().join("old");
        fs::create_dir_all(old.join("media")).unwrap();
        let canonical = fs::canonicalize(&old).unwrap();
        fs::write(canonical.join("media/v.mp4"), b"x").unwrap();
        let mut original = project("a");
        original.media.video = MediaRef::absolute(canonical.join("media/v.mp4"));
        Session::new(original)
            .save_as(&canonical.join("a.reel"))
            .unwrap();

        let new = dir.path().join("new");
        fs::rename(&canonical, &new).unwrap();
        let session = Session::open(&new.join("a.reel")).unwrap();
        let video = session
            .editor
            .project()
            .media
            .video
            .resolve(session.folder());
        assert_eq!(video, new.join("media/v.mp4"));
        assert!(video.exists());
    }
}
