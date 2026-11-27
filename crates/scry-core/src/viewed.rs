//! The "Viewed" marks of the Changes window: which files of a change the
//! user has read.
//!
//! A mark is tied to the content of the file's two sides
//! ([`crate::diff::ContentId`]): when the new side changes, the mark falls
//! and the file is "changed since viewed". Marks are kept per scope (what
//! is not committed, `main..HEAD`), in one file under the git folder, so
//! nothing is written to the working folder.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::diff::ContentId;

/// Where a file stands with its mark.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    NotViewed,
    Viewed,
    /// Viewed, and its new side has changed since.
    ChangedSince,
}

/// What was viewed of one file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Mark {
    old: String,
    new: String,
}

/// The file's content: `{"scopes": {"uncommitted": {"src/a.rs": {…}}}}`.
#[derive(Debug, Default, Serialize, Deserialize)]
struct Stored {
    #[serde(default)]
    scopes: BTreeMap<String, BTreeMap<String, Mark>>,
}

/// One scope's marks in one repository.
pub struct Viewed {
    file: PathBuf,
    scope: String,
    marks: BTreeMap<String, Mark>,
}

impl Viewed {
    /// The marks of `scope` in the repository whose git folder is
    /// `git_dir`. A file missing or not understood means no mark.
    pub fn load(git_dir: &Path, scope: &str) -> Viewed {
        let file = git_dir.join("scry").join("viewed.json");
        let marks = read(&file).scopes.remove(scope).unwrap_or_default();
        Viewed {
            file,
            scope: scope.to_string(),
            marks,
        }
    }

    /// Where the file at `path`, with this content, stands.
    pub fn state(&self, path: &str, id: &ContentId) -> State {
        match self.marks.get(path) {
            Some(mark) if mark.old == id.old && mark.new == id.new => State::Viewed,
            // Another old side is another change (a commit was made
            // since): nothing of it was viewed.
            Some(mark) if mark.old == id.old => State::ChangedSince,
            _ => State::NotViewed,
        }
    }

    /// Marks the file at `path` viewed with this content, or removes its
    /// mark, and writes the marks. The file is read again first: another
    /// window may have written other marks since.
    ///
    /// `listed` holds the paths of the whole change when it is known:
    /// marks of this scope on other paths are then dropped, their files
    /// no longer differ.
    pub fn set(
        &mut self,
        path: &str,
        id: Option<&ContentId>,
        listed: Option<&[&str]>,
    ) -> io::Result<()> {
        let mut stored = read(&self.file);
        let mut marks = stored.scopes.remove(&self.scope).unwrap_or_default();
        match id {
            Some(id) => {
                marks.insert(
                    path.to_string(),
                    Mark {
                        old: id.old.clone(),
                        new: id.new.clone(),
                    },
                );
            }
            None => {
                marks.remove(path);
            }
        }
        if let Some(listed) = listed {
            marks.retain(|path, _| listed.contains(&path.as_str()));
        }
        if !marks.is_empty() {
            stored.scopes.insert(self.scope.clone(), marks.clone());
        }
        self.marks = marks;
        if let Some(folder) = self.file.parent() {
            fs::create_dir_all(folder)?;
        }
        // Written beside, then moved: a reader never sees half a file.
        let beside = self.file.with_extension("json.new");
        fs::write(&beside, serde_json::to_vec_pretty(&stored)?)?;
        fs::rename(&beside, &self.file)
    }
}

fn read(file: &Path) -> Stored {
    fs::read(file)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(old: &str, new: &str) -> ContentId {
        ContentId {
            old: old.to_string(),
            new: new.to_string(),
        }
    }

    #[test]
    fn a_mark_holds_for_the_same_content_and_falls_when_the_new_side_changes() {
        let dir = tempfile::tempdir().unwrap();
        let mut viewed = Viewed::load(dir.path(), "uncommitted");
        assert_eq!(viewed.state("a.rs", &id("1", "2")), State::NotViewed);
        viewed.set("a.rs", Some(&id("1", "2")), None).unwrap();
        assert_eq!(viewed.state("a.rs", &id("1", "2")), State::Viewed);
        assert_eq!(viewed.state("a.rs", &id("1", "3")), State::ChangedSince);
        // Another old side: a commit was made, it is another change.
        assert_eq!(viewed.state("a.rs", &id("2", "3")), State::NotViewed);
        assert_eq!(viewed.state("b.rs", &id("1", "2")), State::NotViewed);
    }

    #[test]
    fn marks_are_kept_under_the_git_folder_per_scope() {
        let dir = tempfile::tempdir().unwrap();
        let mut viewed = Viewed::load(dir.path(), "uncommitted");
        viewed.set("a.rs", Some(&id("1", "2")), None).unwrap();
        assert!(dir.path().join("scry").join("viewed.json").is_file());
        assert!(!dir.path().join("scry").join("viewed.json.new").exists());
        let again = Viewed::load(dir.path(), "uncommitted");
        assert_eq!(again.state("a.rs", &id("1", "2")), State::Viewed);
        let other = Viewed::load(dir.path(), "main..HEAD");
        assert_eq!(other.state("a.rs", &id("1", "2")), State::NotViewed);
    }

    #[test]
    fn a_mark_is_removed_and_another_windows_marks_are_kept() {
        let dir = tempfile::tempdir().unwrap();
        let mut one = Viewed::load(dir.path(), "uncommitted");
        let mut other = Viewed::load(dir.path(), "uncommitted");
        let mut staged = Viewed::load(dir.path(), "staged");
        one.set("a.rs", Some(&id("1", "2")), None).unwrap();
        staged.set("s.rs", Some(&id("1", "2")), None).unwrap();
        other.set("b.rs", Some(&id("1", "2")), None).unwrap();
        other.set("b.rs", None, None).unwrap();
        let read = Viewed::load(dir.path(), "uncommitted");
        assert_eq!(read.state("a.rs", &id("1", "2")), State::Viewed);
        assert_eq!(read.state("b.rs", &id("1", "2")), State::NotViewed);
        let staged = Viewed::load(dir.path(), "staged");
        assert_eq!(staged.state("s.rs", &id("1", "2")), State::Viewed);
    }

    #[test]
    fn marks_of_files_that_no_longer_differ_are_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let mut viewed = Viewed::load(dir.path(), "uncommitted");
        viewed.set("gone.rs", Some(&id("1", "2")), None).unwrap();
        viewed
            .set("a.rs", Some(&id("1", "2")), Some(&["a.rs", "b.rs"]))
            .unwrap();
        assert_eq!(viewed.state("gone.rs", &id("1", "2")), State::NotViewed);
        assert_eq!(viewed.state("a.rs", &id("1", "2")), State::Viewed);
        let text = fs::read_to_string(dir.path().join("scry").join("viewed.json")).unwrap();
        assert!(!text.contains("gone.rs"));
    }

    #[test]
    fn a_file_not_understood_means_no_mark() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("scry")).unwrap();
        fs::write(dir.path().join("scry").join("viewed.json"), "not json").unwrap();
        let mut viewed = Viewed::load(dir.path(), "uncommitted");
        assert_eq!(viewed.state("a.rs", &id("1", "2")), State::NotViewed);
        viewed.set("a.rs", Some(&id("1", "2")), None).unwrap();
        assert_eq!(viewed.state("a.rs", &id("1", "2")), State::Viewed);
    }
}
