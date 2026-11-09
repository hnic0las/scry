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
use crate::resolution::{self, Analysis, Candidate, CandidateKind};
use crate::syntax::{self, Language, ParseCheck};

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
    language: Option<Language>,
    /// Whether a reference version of the file parses. Grammars reject some
    /// valid code; without such a reference, a parse failure says nothing
    /// about the merge, so nothing is checked.
    checkable: bool,
}

impl MergeSession {
    /// A session on a text with conflict markers. With a language, every
    /// candidate is checked by parsing.
    pub fn from_text(text: &str, language: Option<Language>) -> Result<MergeSession, ParseError> {
        Ok(MergeSession::from_segments(
            conflict::parse(text)?,
            false,
            language,
        ))
    }

    fn from_segments(
        segments: Vec<Segment>,
        rebuilt: bool,
        language: Option<Language>,
    ) -> MergeSession {
        let conflicts: Vec<Conflict> = conflict::conflicts(&segments).cloned().collect();
        let mut analyses = resolution::analyze(&segments);
        let checkable = match language {
            Some(language) => verify(&segments, &conflicts, &mut analyses, language),
            None => false,
        };
        let choices = vec![None; conflicts.len()];
        MergeSession {
            segments,
            conflicts,
            analyses,
            choices,
            rebuilt,
            language,
            checkable,
        }
    }

    pub fn language(&self) -> Option<Language> {
        self.language
    }

    /// Whether the merged text parses; `NotChecked` without a language,
    /// when no reference version of the file parses, or while a conflict has
    /// no choice.
    pub fn merged_check(&self) -> ParseCheck {
        match (self.language, self.merged_text()) {
            (Some(language), Some(text)) if self.checkable => syntax::check(language, &text),
            _ => ParseCheck::NotChecked,
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

/// Checks every candidate by parsing the whole file with it in place, and
/// adds the "closed" variants of "both" where they repair it.
///
/// Only when the file parses both with every conflict taken from ours and
/// with every conflict taken from theirs: grammars reject some valid code
/// (newer syntax, macros), and a failure must never be blamed on a candidate
/// for that. The other conflicts are filled with ours while one is checked.
/// Returns whether the file could be checked.
fn verify(
    segments: &[Segment],
    conflicts: &[Conflict],
    analyses: &mut [Analysis],
    language: Language,
) -> bool {
    let ours: Vec<&str> = conflicts.iter().map(|c| c.ours.text.as_str()).collect();
    let theirs: Vec<&str> = conflicts.iter().map(|c| c.theirs.text.as_str()).collect();
    if !parses(language, &assemble(segments, &ours))
        || !parses(language, &assemble(segments, &theirs))
    {
        return false;
    }
    let reference = ours;

    for (index, analysis) in analyses.iter_mut().enumerate() {
        add_closed_candidates(
            segments,
            &conflicts[index],
            &reference,
            index,
            analysis,
            language,
        );
        for candidate in &mut analysis.candidates {
            let check = {
                let mut texts = reference.clone();
                texts[index] = candidate.text.as_str();
                syntax::check(language, &assemble(segments, &texts))
            };
            candidate.parse = check;
        }
    }
    true
}

/// Adds "both, closed" when plain "both" does not parse: each side followed
/// by the first one, two or three lines after the conflict, whichever count
/// first makes the file parse.
fn add_closed_candidates(
    segments: &[Segment],
    conflict: &Conflict,
    reference: &[&str],
    index: usize,
    analysis: &mut Analysis,
    language: Language,
) {
    let ours = conflict.ours.text.as_str();
    let theirs = conflict.theirs.text.as_str();
    if ours.is_empty() || theirs.is_empty() {
        return;
    }
    let Some(following) = text_after_conflict(segments, index) else {
        return;
    };
    let lines: Vec<&str> = following.split_inclusive('\n').take(3).collect();
    let orders = [
        (CandidateKind::OursThenTheirsClosed, ours, theirs),
        (CandidateKind::TheirsThenOursClosed, theirs, ours),
    ];
    let parses_with = |text: &str| {
        let mut texts = reference.to_vec();
        texts[index] = text;
        parses(language, &assemble(segments, &texts))
    };

    for (kind, first, second) in orders {
        if parses_with(&format!("{first}{second}")) {
            continue;
        }
        for count in 1..=lines.len() {
            let text = format!("{first}{}{second}", lines[..count].concat());
            if parses_with(&text) {
                if !analysis.candidates.iter().any(|c| c.text == text) {
                    analysis.candidates.push(Candidate {
                        kind,
                        text,
                        parse: ParseCheck::NotChecked,
                    });
                }
                break;
            }
        }
    }
}

/// The common text right after the conflict at `index`, if any.
fn text_after_conflict(segments: &[Segment], index: usize) -> Option<&str> {
    let mut seen = 0;
    let mut segments = segments.iter();
    for segment in segments.by_ref() {
        if let Segment::Conflict(_) = segment {
            if seen == index {
                break;
            }
            seen += 1;
        }
    }
    match segments.next()? {
        Segment::Common(text) => Some(text.as_str()),
        Segment::Conflict(_) => None,
    }
}

/// The file with conflict `i` replaced by `texts[i]`.
fn assemble(segments: &[Segment], texts: &[&str]) -> String {
    let mut result = String::new();
    let mut index = 0;
    for segment in segments {
        match segment {
            Segment::Common(text) => result.push_str(text),
            Segment::Conflict(_) => {
                result.push_str(texts[index]);
                index += 1;
            }
        }
    }
    result
}

fn parses(language: Language, text: &str) -> bool {
    syntax::check(language, text) == ParseCheck::Parses
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
    let language = Language::from_path(&files.merged);
    let merged = read_text(&files.merged)?;
    if let Ok(segments) = conflict::parse(&merged) {
        // Computed first: the borrow of `segments` ends before it moves into
        // the session.
        let all_with_base = {
            let mut conflicts = conflict::conflicts(&segments).peekable();
            conflicts.peek().is_some() && conflicts.all(|conflict| conflict.base.is_some())
        };
        if all_with_base {
            return Ok(MergeSession::from_segments(segments, false, language));
        }
    }

    let text = merge_file(git, files, ours, theirs)?;
    let segments = conflict::parse(&text).map_err(LoadError::Parse)?;
    Ok(MergeSession::from_segments(segments, true, language))
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
        let mut session = MergeSession::from_text(TEXT, None).unwrap();
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
        let mut session = MergeSession::from_text(TEXT, None).unwrap();
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
        let mut session = MergeSession::from_text(TEXT, None).unwrap();
        assert_eq!(session.apply_suggestions(), 1);
        assert_eq!(session.choice(0), None);
        assert_eq!(session.choice(1), Some(CandidateKind::Ours));
    }

    #[test]
    fn find_by_number_or_id() {
        let session = MergeSession::from_text(TEXT, None).unwrap();
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

    /// The demo's second conflict: zdiff3 moved the closing brace both added
    /// functions end with below the conflict.
    const SWIFT: &str = "func upload() {\n    send()\n}\n\n\
        <<<<<<< ours\nfunc cancel() {\n    queue.cancelAll()\n\
        ||||||| base\n=======\nfunc pause() {\n    queue.suspend()\n>>>>>>> theirs\n}\n";

    fn candidate(session: &MergeSession, kind: CandidateKind) -> Option<&Candidate> {
        session
            .analysis(0)
            .candidates
            .iter()
            .find(|c| c.kind == kind)
    }

    #[test]
    fn candidates_are_checked_by_parsing() {
        let session = MergeSession::from_text(SWIFT, Some(Language::Swift)).unwrap();
        let parse = |kind| candidate(&session, kind).unwrap().parse;
        assert_eq!(parse(CandidateKind::Ours), ParseCheck::Parses);
        assert_eq!(parse(CandidateKind::Theirs), ParseCheck::Parses);
        assert!(matches!(
            parse(CandidateKind::OursThenTheirs),
            ParseCheck::DoesNotParse { .. }
        ));
    }

    #[test]
    fn closed_both_repairs_the_moved_brace() {
        let mut session = MergeSession::from_text(SWIFT, Some(Language::Swift)).unwrap();
        let closed = candidate(&session, CandidateKind::OursThenTheirsClosed).unwrap();
        assert_eq!(closed.parse, ParseCheck::Parses);
        assert!(candidate(&session, CandidateKind::TheirsThenOursClosed).is_some());

        session
            .choose(0, CandidateKind::OursThenTheirsClosed)
            .unwrap();
        assert_eq!(
            session.merged_text().as_deref(),
            Some(
                "func upload() {\n    send()\n}\n\nfunc cancel() {\n    queue.cancelAll()\n}\nfunc pause() {\n    queue.suspend()\n}\n"
            )
        );
        assert_eq!(session.merged_check(), ParseCheck::Parses);
    }

    #[test]
    fn no_closed_candidate_when_plain_both_parses() {
        let text = "<<<<<<< ours\nlet a = 1\n||||||| base\n=======\nlet b = 2\n>>>>>>> theirs\n";
        let session = MergeSession::from_text(text, Some(Language::Swift)).unwrap();
        assert!(candidate(&session, CandidateKind::OursThenTheirsClosed).is_none());
        assert_eq!(
            candidate(&session, CandidateKind::OursThenTheirs)
                .unwrap()
                .parse,
            ParseCheck::Parses
        );
    }

    #[test]
    fn nothing_checked_without_a_language() {
        let session = MergeSession::from_text(SWIFT, None).unwrap();
        assert!(
            session
                .analysis(0)
                .candidates
                .iter()
                .all(|c| c.parse == ParseCheck::NotChecked)
        );
        assert!(candidate(&session, CandidateKind::OursThenTheirsClosed).is_none());
    }

    #[test]
    fn nothing_checked_when_the_grammar_rejects_both_sides() {
        // Valid for the user's compiler, rejected by the grammar: here, plain
        // garbage stands in for it on both sides.
        let text = "@@@ {\n<<<<<<< ours\na()\n||||||| base\n=======\nb()\n>>>>>>> theirs\n";
        let mut session = MergeSession::from_text(text, Some(Language::Swift)).unwrap();
        assert!(
            session
                .analysis(0)
                .candidates
                .iter()
                .all(|c| c.parse == ParseCheck::NotChecked)
        );
        session.choose(0, CandidateKind::Ours).unwrap();
        assert_eq!(session.merged_check(), ParseCheck::NotChecked);
    }
}
