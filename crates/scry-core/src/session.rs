//! A merge session on one file: its conflicts, the candidate chosen for each,
//! and the merged text once every conflict has one.
//!
//! Nothing is written while a conflict is left: `git mergetool` then keeps
//! the file as conflicted, and the user's work in it is untouched.

use std::ffi::OsString;
use std::fmt;
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use crate::conflict::{self, Conflict, ParseError, Segment};
use crate::git::{Git, GitError};
use crate::resolution::{self, Analysis, CandidateKind};

/// The four files `git mergetool` passes to a merge tool.
pub struct MergetoolFiles {
    pub base: PathBuf,
    pub local: PathBuf,
    pub remote: PathBuf,
    pub merged: PathBuf,
}

pub struct MergeSession {
    segments: Vec<Segment>,
    conflicts: Vec<Conflict>,
    analyses: Vec<Analysis>,
    choices: Vec<Option<CandidateKind>>,
    rebuilt: bool,
}

impl MergeSession {
    /// A session on a text with conflict markers.
    pub fn from_text(text: &str) -> Result<MergeSession, ParseError> {
        Ok(MergeSession::from_segments(conflict::parse(text)?, false))
    }

    fn from_segments(segments: Vec<Segment>, rebuilt: bool) -> MergeSession {
        let conflicts: Vec<Conflict> = conflict::conflicts(&segments).cloned().collect();
        let analyses = resolution::analyze(&segments);
        let choices = vec![None; conflicts.len()];
        MergeSession {
            segments,
            conflicts,
            analyses,
            choices,
            rebuilt,
        }
    }

    /// Whether the conflicts were rebuilt from the three versions because
    /// the file had no base in its markers.
    pub fn rebuilt(&self) -> bool {
        self.rebuilt
    }

    pub fn conflict_count(&self) -> usize {
        self.conflicts.len()
    }

    /// The conflict at `index`, from 0. Panics when out of range, like a
    /// slice.
    pub fn conflict(&self, index: usize) -> &Conflict {
        &self.conflicts[index]
    }

    pub fn analysis(&self, index: usize) -> &Analysis {
        &self.analyses[index]
    }

    pub fn choice(&self, index: usize) -> Option<CandidateKind> {
        self.choices[index]
    }

    /// The index of a conflict given by its number ("2") or its ID
    /// ("cb5d1a4").
    pub fn find(&self, reference: &str) -> Option<usize> {
        if let Ok(number) = reference.parse::<usize>() {
            return (1..=self.conflicts.len())
                .contains(&number)
                .then(|| number - 1);
        }
        self.analyses
            .iter()
            .position(|analysis| analysis.id == reference)
    }

    /// Chooses a candidate for a conflict; it must be one the conflict
    /// offers.
    pub fn choose(&mut self, index: usize, kind: CandidateKind) -> Result<(), ChooseError> {
        let Some(analysis) = self.analyses.get(index) else {
            return Err(ChooseError::NoSuchConflict(index + 1));
        };
        if !analysis
            .candidates
            .iter()
            .any(|candidate| candidate.kind == kind)
        {
            return Err(ChooseError::NotOffered {
                conflict: analysis.id.clone(),
                candidate: kind.id(),
            });
        }
        self.choices[index] = Some(kind);
        Ok(())
    }

    /// Chooses the rule suggestion for every conflict that has one and no
    /// choice yet. Returns how many were chosen.
    pub fn apply_suggestions(&mut self) -> usize {
        let mut applied = 0;
        for (choice, analysis) in self.choices.iter_mut().zip(&self.analyses) {
            if choice.is_none()
                && let Some(suggestion) = analysis.suggestion
            {
                *choice = Some(suggestion);
                applied += 1;
            }
        }
        applied
    }

    pub fn unresolved(&self) -> usize {
        self.choices
            .iter()
            .filter(|choice| choice.is_none())
            .count()
    }

    /// The file with every conflict replaced by its chosen candidate, or
    /// `None` while a conflict has no choice.
    pub fn merged_text(&self) -> Option<String> {
        let mut text = String::new();
        let mut index = 0;
        for segment in &self.segments {
            match segment {
                Segment::Common(common) => text.push_str(common),
                Segment::Conflict(_) => {
                    let kind = self.choices[index]?;
                    let candidate = self.analyses[index]
                        .candidates
                        .iter()
                        .find(|candidate| candidate.kind == kind)?;
                    text.push_str(&candidate.text);
                    index += 1;
                }
            }
        }
        Some(text)
    }
}

/// Loads the session for the files `git mergetool` passed.
///
/// Uses the merged file as it is when every conflict in it shows the base
/// (`diff3` or `zdiff3` style), so edits made before launching are kept.
/// Otherwise rebuilds the conflicts from the three versions with
/// `git merge-file --zdiff3`, labelled `ours` and `theirs`.
pub fn load(
    git: &dyn Git,
    files: &MergetoolFiles,
    ours: &str,
    theirs: &str,
) -> Result<MergeSession, LoadError> {
    let merged = read_text(&files.merged)?;
    if let Ok(segments) = conflict::parse(&merged) {
        // Computed first: the borrow of `segments` ends before it moves into
        // the session.
        let all_with_base = {
            let mut conflicts = conflict::conflicts(&segments).peekable();
            conflicts.peek().is_some() && conflicts.all(|conflict| conflict.base.is_some())
        };
        if all_with_base {
            return Ok(MergeSession::from_segments(segments, false));
        }
    }

    let text = merge_file(git, files, ours, theirs)?;
    let segments = conflict::parse(&text).map_err(LoadError::Parse)?;
    Ok(MergeSession::from_segments(segments, true))
}

fn merge_file(
    git: &dyn Git,
    files: &MergetoolFiles,
    ours: &str,
    theirs: &str,
) -> Result<String, LoadError> {
    let local = path_text(&files.local)?;
    let base = path_text(&files.base)?;
    let remote = path_text(&files.remote)?;
    let args = [
        "merge-file",
        "--stdout",
        "--zdiff3",
        "-L",
        ours,
        "-L",
        "base",
        "-L",
        theirs,
        local,
        base,
        remote,
    ];
    let output = git.output(&args)?;
    // merge-file exits with the number of conflicts (at most 127), or with a
    // negative value on error, such as for binary files.
    match output.status {
        Some(code) if (0..=127).contains(&code) => {}
        _ => {
            return Err(LoadError::Git(GitError::Failed {
                command: args.join(" "),
                stderr: output.stderr,
            }));
        }
    }
    String::from_utf8(output.stdout).map_err(|_| LoadError::NotText(files.merged.clone()))
}

fn read_text(path: &Path) -> Result<String, LoadError> {
    let bytes = fs::read(path).map_err(|error| LoadError::Io(path.to_path_buf(), error))?;
    String::from_utf8(bytes).map_err(|_| LoadError::NotText(path.to_path_buf()))
}

fn path_text(path: &Path) -> Result<&str, LoadError> {
    path.to_str()
        .ok_or_else(|| LoadError::NotText(path.to_path_buf()))
}

/// Replaces the file at `path` with `text` in one step: written to a
/// temporary file next to it, flushed to disk, then renamed over it, so a
/// crash never leaves a half-written file. Keeps the file's permissions.
pub fn write_atomically(path: &Path, text: &str) -> io::Result<()> {
    let Some(name) = path.file_name() else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "not a file path",
        ));
    };
    let mut temporary_name = OsString::from(".");
    temporary_name.push(name);
    temporary_name.push(".scry-tmp");
    let temporary = path.with_file_name(temporary_name);

    let result = write_then_rename(path, &temporary, text);
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn write_then_rename(path: &Path, temporary: &Path, text: &str) -> io::Result<()> {
    let mut file = File::create(temporary)?;
    file.write_all(text.as_bytes())?;
    file.sync_all()?;
    drop(file);
    if let Ok(metadata) = fs::metadata(path) {
        fs::set_permissions(temporary, metadata.permissions())?;
    }
    fs::rename(temporary, path)
}

#[derive(Debug, PartialEq, Eq)]
pub enum ChooseError {
    /// No conflict with this number.
    NoSuchConflict(usize),
    NotOffered {
        conflict: String,
        candidate: &'static str,
    },
}

impl fmt::Display for ChooseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ChooseError::NoSuchConflict(number) => write!(f, "no conflict {number}"),
            ChooseError::NotOffered {
                conflict,
                candidate,
            } => write!(f, "conflict {conflict} has no candidate {candidate:?}"),
        }
    }
}

impl std::error::Error for ChooseError {}

#[derive(Debug)]
pub enum LoadError {
    Io(PathBuf, io::Error),
    /// Not UTF-8: Scry does not merge such files yet, rather than corrupt them.
    NotText(PathBuf),
    Parse(ParseError),
    Git(GitError),
}

impl From<GitError> for LoadError {
    fn from(error: GitError) -> LoadError {
        LoadError::Git(error)
    }
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LoadError::Io(path, error) => write!(f, "cannot read {}: {error}", path.display()),
            LoadError::NotText(path) => {
                write!(
                    f,
                    "{} is not UTF-8 text; Scry cannot merge it yet",
                    path.display()
                )
            }
            LoadError::Parse(error) => write!(f, "{error}"),
            LoadError::Git(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for LoadError {}

#[cfg(test)]
mod tests {
    use super::*;

    const TEXT: &str = "start\n\
        <<<<<<< ours\na\n||||||| base\nx\n=======\nb\n>>>>>>> theirs\n\
        middle\n\
        <<<<<<< ours\nsame\n||||||| base\nold\n=======\nsame\n>>>>>>> theirs\n\
        end\n";

    #[test]
    fn nothing_merged_until_every_conflict_has_a_choice() {
        let mut session = MergeSession::from_text(TEXT).unwrap();
        assert_eq!(session.conflict_count(), 2);
        assert_eq!(session.unresolved(), 2);
        assert_eq!(session.merged_text(), None);

        session.choose(0, CandidateKind::OursThenTheirs).unwrap();
        assert_eq!(session.merged_text(), None);
        session.choose(1, CandidateKind::Ours).unwrap();
        assert_eq!(session.unresolved(), 0);
        assert_eq!(
            session.merged_text().as_deref(),
            Some("start\na\nb\nmiddle\nsame\nend\n")
        );
    }

    #[test]
    fn only_offered_candidates_can_be_chosen() {
        let mut session = MergeSession::from_text(TEXT).unwrap();
        // Conflict 2 has the same text on both sides: "theirs" is not offered.
        assert_eq!(
            session.choose(1, CandidateKind::Theirs),
            Err(ChooseError::NotOffered {
                conflict: session.analysis(1).id.clone(),
                candidate: "theirs",
            })
        );
        assert_eq!(
            session.choose(5, CandidateKind::Ours),
            Err(ChooseError::NoSuchConflict(6))
        );
    }

    #[test]
    fn suggestions_fill_only_open_conflicts_that_have_one() {
        let mut session = MergeSession::from_text(TEXT).unwrap();
        assert_eq!(session.apply_suggestions(), 1);
        assert_eq!(session.choice(0), None);
        assert_eq!(session.choice(1), Some(CandidateKind::Ours));
    }

    #[test]
    fn find_by_number_or_id() {
        let session = MergeSession::from_text(TEXT).unwrap();
        assert_eq!(session.find("1"), Some(0));
        assert_eq!(session.find("2"), Some(1));
        assert_eq!(session.find("3"), None);
        assert_eq!(session.find("0"), None);
        assert_eq!(session.find(&session.analysis(1).id.clone()), Some(1));
        assert_eq!(session.find("cffffff"), None);
    }

    #[test]
    fn atomic_write_replaces_the_file_and_leaves_no_temporary() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("file.txt");
        fs::write(&path, "old\n").unwrap();
        write_atomically(&path, "new\n").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "new\n");
        let names: Vec<_> = fs::read_dir(dir.path()).unwrap().collect();
        assert_eq!(names.len(), 1);
    }
}
