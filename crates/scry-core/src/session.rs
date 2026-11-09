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
use crate::highlight::{Highlighter, Span};
use crate::indent::{self, Newline};
use crate::resolution::{self, Analysis, Candidate, CandidateKind};
use crate::syntax::{self, Language, ParseCheck};

/// The three panes of the merge editor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Pane {
    Ours,
    Result,
    Theirs,
}

/// The four files `git mergetool` passes to a merge tool.
pub struct MergetoolFiles {
    pub base: PathBuf,
    pub local: PathBuf,
    pub remote: PathBuf,
    pub merged: PathBuf,
}

/// How a conflict is resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Choice {
    /// One of the candidates the core offers.
    Candidate(CandidateKind),
    /// Text the user wrote by hand.
    Edited(Edit),
}

/// A conflict's text written by hand, with its check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edit {
    pub text: String,
    /// Checked like a candidate: the whole file parsed with this text in
    /// place.
    pub parse: ParseCheck,
}

/// One step of the history: the conflicts it changed, each with the choice
/// it had before.
type Change = Vec<(usize, Option<Choice>)>;

pub struct MergeSession {
    segments: Vec<Segment>,
    conflicts: Vec<Conflict>,
    analyses: Vec<Analysis>,
    choices: Vec<Option<Choice>>,
    /// Changes to undo, the latest last, and changes undone that can be
    /// redone. A new change clears the redo history, as in any editor.
    undo_history: Vec<Change>,
    redo_history: Vec<Change>,
    rebuilt: bool,
    language: Option<Language>,
    /// Whether a reference version of the file parses. Grammars reject some
    /// valid code; without such a reference, a parse failure says nothing
    /// about the merge, so nothing is checked.
    checkable: bool,
    highlighter: Option<Highlighter>,
    /// Highlight spans of the ours and theirs versions of the file, by line;
    /// they never change during the session.
    ours_spans: Vec<Vec<Span>>,
    theirs_spans: Vec<Vec<Span>>,
    /// The file's indent unit, for text typed by hand.
    indent_unit: String,
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
        let highlighter = language.and_then(Highlighter::new);
        let mut session = MergeSession {
            segments,
            conflicts,
            analyses,
            choices,
            undo_history: Vec::new(),
            redo_history: Vec::new(),
            rebuilt,
            language,
            checkable,
            highlighter,
            ours_spans: Vec::new(),
            theirs_spans: Vec::new(),
            indent_unit: String::new(),
        };
        session.indent_unit = indent::indent_unit(&session.side_text(Pane::Ours));
        session.ours_spans = session.highlight(&session.side_text(Pane::Ours));
        session.theirs_spans = session.highlight(&session.side_text(Pane::Theirs));
        session
    }

    /// Spans per line of `text`; none without a highlighter.
    pub(crate) fn highlight(&self, text: &str) -> Vec<Vec<Span>> {
        match &self.highlighter {
            Some(highlighter) => highlighter.lines(text),
            None => Vec::new(),
        }
    }

    /// The text a pane shows, as one file: the common text with each
    /// conflict's ours side, theirs side, or chosen candidate (an
    /// unresolved conflict adds nothing to the result).
    pub(crate) fn side_text(&self, pane: Pane) -> String {
        let mut text = String::new();
        let mut index = 0;
        for segment in &self.segments {
            match segment {
                Segment::Common(common) => text.push_str(common),
                Segment::Conflict(conflict) => {
                    match pane {
                        Pane::Ours => text.push_str(&conflict.ours.text),
                        Pane::Theirs => text.push_str(&conflict.theirs.text),
                        Pane::Result => {
                            if let Some(chosen) = self.chosen_text(index) {
                                text.push_str(chosen);
                            }
                        }
                    }
                    index += 1;
                }
            }
        }
        text
    }

    /// The first and last lines (from 1) of a conflict's side in that
    /// side's own file; `None` when the side has no line there.
    pub(crate) fn side_lines(&self, index: usize, pane: Pane) -> Option<(usize, usize)> {
        let mut line = 1;
        let mut conflict = 0;
        for segment in &self.segments {
            match segment {
                Segment::Common(common) => line += common.lines().count(),
                Segment::Conflict(found) => {
                    let side = match pane {
                        Pane::Theirs => &found.theirs.text,
                        _ => &found.ours.text,
                    };
                    let count = side.lines().count();
                    if conflict == index {
                        return (count > 0).then(|| (line, line + count - 1));
                    }
                    line += count;
                    conflict += 1;
                }
            }
        }
        None
    }

    pub(crate) fn ours_spans(&self) -> &[Vec<Span>] {
        &self.ours_spans
    }

    pub(crate) fn theirs_spans(&self) -> &[Vec<Span>] {
        &self.theirs_spans
    }

    /// The text that resolves a conflict: its chosen candidate's, or the
    /// text written by hand; `None` while unresolved.
    pub fn chosen_text(&self, index: usize) -> Option<&str> {
        match self.choices[index].as_ref()? {
            Choice::Candidate(kind) => self.analyses[index]
                .candidates
                .iter()
                .find(|candidate| candidate.kind == *kind)
                .map(|candidate| candidate.text.as_str()),
            Choice::Edited(edit) => Some(edit.text.as_str()),
        }
    }

    pub(crate) fn segments(&self) -> &[Segment] {
        &self.segments
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

    pub fn choice(&self, index: usize) -> Option<&Choice> {
        self.choices[index].as_ref()
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
        self.apply(vec![(index, Some(Choice::Candidate(kind)))]);
        Ok(())
    }

    /// Resolves a conflict with text written by hand, checked by parsing
    /// like a candidate. A text without a final line break gets one, so the
    /// line after the conflict stays on its own line.
    pub fn edit(&mut self, index: usize, text: &str) -> Result<(), ChooseError> {
        let Some(conflict) = self.conflicts.get(index) else {
            return Err(ChooseError::NoSuchConflict(index + 1));
        };
        let mut text = text.to_string();
        if !text.is_empty() && !text.ends_with('\n') {
            // Follow the file's line endings.
            let crlf = conflict.ours.text.contains("\r\n") || conflict.theirs.text.contains("\r\n");
            text.push_str(if crlf { "\r\n" } else { "\n" });
        }
        let parse = self.check_in_place(index, &text);
        self.apply(vec![(index, Some(Choice::Edited(Edit { text, parse })))]);
        Ok(())
    }

    /// What Return inserts while editing a conflict by hand, with `before`
    /// and `after` the current line's text on each side of the cursor.
    pub fn newline(&self, before: &str, after: &str) -> Newline {
        indent::newline(before, after, &self.indent_unit, self.language)
    }

    /// The indentation for the current line when `closer` is typed while
    /// editing conflict `index`, with `before` the draft's text before the
    /// cursor; `None` to type it as it is. The file before the conflict is
    /// searched too, for a block opened outside it.
    pub fn closing_indent(&self, index: usize, before: &str, closer: char) -> Option<String> {
        let mut text = self.text_before_conflict(index);
        text.push_str(before);
        indent::closing_indent(&text, closer, &self.indent_unit)
    }

    /// The file before conflict `index`: common text, and earlier conflicts
    /// as chosen, or with ours while unresolved.
    fn text_before_conflict(&self, index: usize) -> String {
        let mut text = String::new();
        let mut conflict = 0;
        for segment in &self.segments {
            match segment {
                Segment::Common(common) => text.push_str(common),
                Segment::Conflict(found) => {
                    if conflict == index {
                        break;
                    }
                    text.push_str(self.chosen_text(conflict).unwrap_or(&found.ours.text));
                    conflict += 1;
                }
            }
        }
        text
    }

    /// Highlight spans of `text` being typed for conflict `index`, one list
    /// per line of `text`. It is highlighted in place in the file (the other
    /// conflicts resolved as chosen, or with ours), so a string or comment
    /// that spans lines is coloured as it will be once applied.
    pub fn highlight_draft(&self, index: usize, text: &str) -> Vec<Vec<Span>> {
        if self.highlighter.is_none() || index >= self.conflicts.len() {
            return Vec::new();
        }
        let mut draft = text.to_string();
        if !draft.is_empty() && !draft.ends_with('\n') {
            draft.push('\n');
        }
        let mut texts: Vec<&str> = (0..self.conflicts.len())
            .map(|other| {
                self.chosen_text(other)
                    .unwrap_or(&self.conflicts[other].ours.text)
            })
            .collect();
        texts[index] = &draft;
        // The draft's first line in the assembled file.
        let mut first = 0;
        let mut conflict = 0;
        for segment in &self.segments {
            match segment {
                Segment::Common(common) => first += common.lines().count(),
                Segment::Conflict(_) => {
                    if conflict == index {
                        break;
                    }
                    first += texts[conflict].lines().count();
                    conflict += 1;
                }
            }
        }
        let count = draft.lines().count();
        self.highlight(&assemble(&self.segments, &texts))
            .into_iter()
            .skip(first)
            .take(count)
            .collect()
    }

    /// Whether the file parses with `text` resolving conflict `index` and
    /// the other conflicts filled with ours, as candidates are checked.
    fn check_in_place(&self, index: usize, text: &str) -> ParseCheck {
        let Some(language) = self.language else {
            return ParseCheck::NotChecked;
        };
        if !self.checkable {
            return ParseCheck::NotChecked;
        }
        let mut texts: Vec<&str> = self
            .conflicts
            .iter()
            .map(|c| c.ours.text.as_str())
            .collect();
        texts[index] = text;
        syntax::check(language, &assemble(&self.segments, &texts))
    }

    /// Chooses the rule suggestion for every conflict that has one and no
    /// choice yet, as one step of the history. Returns how many were chosen.
    pub fn apply_suggestions(&mut self) -> usize {
        let mut changes = Vec::new();
        for (index, analysis) in self.analyses.iter().enumerate() {
            if self.choices[index].is_none()
                && let Some(suggestion) = analysis.suggestion
            {
                changes.push((index, Some(Choice::Candidate(suggestion))));
            }
        }
        let applied = changes.len();
        if applied > 0 {
            self.apply(changes);
        }
        applied
    }

    /// Sets choices as one step of the history.
    fn apply(&mut self, changes: Change) {
        let previous = self.swap(changes);
        self.undo_history.push(previous);
        self.redo_history.clear();
    }

    /// Puts each choice in place and returns the ones it replaced.
    fn swap(&mut self, changes: Change) -> Change {
        changes
            .into_iter()
            .map(|(index, choice)| {
                let previous = std::mem::replace(&mut self.choices[index], choice);
                (index, previous)
            })
            .collect()
    }

    /// Undoes the last step; false when there is none.
    pub fn undo(&mut self) -> bool {
        let Some(change) = self.undo_history.pop() else {
            return false;
        };
        let redo = self.swap(change);
        self.redo_history.push(redo);
        true
    }

    /// Redoes the last step undone; false when there is none.
    pub fn redo(&mut self) -> bool {
        let Some(change) = self.redo_history.pop() else {
            return false;
        };
        let undo = self.swap(change);
        self.undo_history.push(undo);
        true
    }

    pub fn can_undo(&self) -> bool {
        !self.undo_history.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo_history.is_empty()
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
                    text.push_str(self.chosen_text(index)?);
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
    use crate::highlight::SyntaxRole;

    const TEXT: &str = "start\n\
        <<<<<<< ours\na\n||||||| base\nx\n=======\nb\n>>>>>>> theirs\n\
        middle\n\
        <<<<<<< ours\nsame\n||||||| base\nold\n=======\nsame\n>>>>>>> theirs\n\
        end\n";

    /// No merged text exists while a conflict is open, so a half-resolved file can never be
    /// written.
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

    /// A conflict resolved by hand merges its text as written, with a final
    /// line break added so the next line stays on its own line.
    #[test]
    fn edited_text_resolves_the_conflict() {
        let mut session = MergeSession::from_text(TEXT, None).unwrap();
        session.choose(1, CandidateKind::Ours).unwrap();
        session.edit(0, "a and b").unwrap();
        assert_eq!(session.chosen_text(0), Some("a and b\n"));
        assert_eq!(
            session.merged_text().as_deref(),
            Some("start\na and b\nmiddle\nsame\nend\n")
        );
        // An empty text removes the block, like "none".
        session.edit(0, "").unwrap();
        assert_eq!(session.chosen_text(0), Some(""));
        assert_eq!(session.edit(7, "x"), Err(ChooseError::NoSuchConflict(8)));
    }

    /// A file with Windows line endings keeps them in the line break added to
    /// an edit.
    #[test]
    fn edit_follows_crlf_line_endings() {
        let text = "a\r\n<<<<<<< ours\r\no\r\n=======\r\nt\r\n>>>>>>> theirs\r\n";
        let mut session = MergeSession::from_text(text, None).unwrap();
        session.edit(0, "x").unwrap();
        assert_eq!(session.chosen_text(0), Some("x\r\n"));
    }

    /// Text written by hand is checked by parsing like any candidate, so a
    /// broken edit is caught before the file is written.
    #[test]
    fn edited_text_is_checked_by_parsing() {
        let mut session = MergeSession::from_text(SWIFT, Some(Language::Swift)).unwrap();
        session
            .edit(
                0,
                "func cancel() {\n    queue.cancelAll()\n}\nfunc pause() {\n    queue.suspend()",
            )
            .unwrap();
        let Some(Choice::Edited(edit)) = session.choice(0) else {
            panic!("expected an edit");
        };
        assert_eq!(edit.parse, ParseCheck::Parses);
        session.edit(0, "let = 1").unwrap();
        let Some(Choice::Edited(edit)) = session.choice(0) else {
            panic!("expected an edit");
        };
        assert!(matches!(edit.parse, ParseCheck::DoesNotParse { .. }));
    }

    /// Undo and redo walk the history of choices and edits, and a new choice
    /// after an undo drops what could be redone, as in any editor.
    #[test]
    fn undo_and_redo_walk_the_history() {
        let mut session = MergeSession::from_text(TEXT, None).unwrap();
        assert!(!session.can_undo() && !session.undo());
        session.choose(0, CandidateKind::Ours).unwrap();
        session.edit(0, "mine").unwrap();
        assert!(session.undo());
        assert_eq!(session.chosen_text(0), Some("a\n"));
        assert!(session.undo());
        assert_eq!(session.choice(0), None);
        assert!(session.can_redo());
        assert!(session.redo());
        assert_eq!(session.chosen_text(0), Some("a\n"));
        session.choose(0, CandidateKind::Theirs).unwrap();
        assert!(!session.can_redo());
        assert!(session.undo());
        assert_eq!(session.chosen_text(0), Some("a\n"));
    }

    /// Text typed by hand is highlighted in place in the file: the lines of
    /// the draft get the spans of their own text, whatever comes before.
    #[test]
    fn draft_is_highlighted_in_place() {
        let session = MergeSession::from_text(SWIFT, Some(Language::Swift)).unwrap();
        let spans = session.highlight_draft(0, "func stop() {\n    // done");
        assert_eq!(spans.len(), 2);
        assert_eq!(spans[0][0].role, SyntaxRole::Keyword);
        assert_eq!((spans[0][0].start, spans[0][0].end), (0, 4));
        assert!(spans[1].iter().any(|span| span.role == SyntaxRole::Comment));
        // Without a grammar, nothing to colour.
        let plain = MergeSession::from_text(SWIFT, None).unwrap();
        assert!(plain.highlight_draft(0, "func a() {").is_empty());
    }

    /// Return keeps the file's own indent unit: the demo indents by four
    /// spaces, so a new line after an opening brace goes four deeper.
    #[test]
    fn newline_uses_the_files_indent_unit() {
        let session = MergeSession::from_text(SWIFT, Some(Language::Swift)).unwrap();
        assert_eq!(session.newline("func stop() {", "}").text, "\n    \n");
    }

    /// A closing brace typed in a conflict finds its block's opening line
    /// in the file before the conflict too: here the function around it.
    #[test]
    fn closing_brace_finds_a_block_opened_before_the_conflict() {
        let text = "func a() {\n    if b {\n<<<<<<< ours\n        c()\n=======\n        d()\n>>>>>>> theirs\n    }\n}\n";
        let session = MergeSession::from_text(text, Some(Language::Swift)).unwrap();
        assert_eq!(
            session
                .closing_indent(0, "        c()\n        ", '}')
                .as_deref(),
            Some("    ")
        );
    }

    /// Applying suggestions is one step: one undo takes them all back.
    #[test]
    fn suggestions_undo_in_one_step() {
        let mut session = MergeSession::from_text(TEXT, None).unwrap();
        assert_eq!(session.apply_suggestions(), 1);
        assert!(session.undo());
        assert_eq!(session.unresolved(), 2);
        assert!(!session.can_undo());
        // Nothing to suggest: no empty step in the history.
        session.choose(1, CandidateKind::Ours).unwrap();
        assert_eq!(session.apply_suggestions(), 0);
        assert!(session.undo());
        assert!(!session.can_undo());
    }

    /// The lens rows blame each side's lines in that side's own file, so the
    /// line numbers must count only that side's text before the conflict.
    #[test]
    fn side_lines_count_each_side_on_its_own() {
        let session = MergeSession::from_text(TEXT, None).unwrap();
        assert_eq!(session.side_lines(0, Pane::Ours), Some((2, 2)));
        assert_eq!(session.side_lines(1, Pane::Ours), Some((4, 4)));
        let text = "a\n<<<<<<< ours\n||||||| base\nx\n=======\nt1\nt2\n>>>>>>> theirs\nz\n<<<<<<< ours\no\n||||||| base\n=======\nt\n>>>>>>> theirs\n";
        let session = MergeSession::from_text(text, None).unwrap();
        // Ours deleted the first block: no line to blame there.
        assert_eq!(session.side_lines(0, Pane::Ours), None);
        assert_eq!(session.side_lines(0, Pane::Theirs), Some((2, 3)));
        assert_eq!(session.side_lines(1, Pane::Ours), Some((3, 3)));
        assert_eq!(session.side_lines(1, Pane::Theirs), Some((5, 5)));
    }

    /// Choosing a candidate the conflict does not offer, or a conflict that does not exist,
    /// is an error rather than a silent no-op.
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

    /// Applying suggestions fills only the conflicts a rule settles and leaves the others
    /// open.
    #[test]
    fn suggestions_fill_only_open_conflicts_that_have_one() {
        let mut session = MergeSession::from_text(TEXT, None).unwrap();
        assert_eq!(session.apply_suggestions(), 1);
        assert_eq!(session.choice(0), None);
        assert_eq!(
            session.choice(1),
            Some(&Choice::Candidate(CandidateKind::Ours))
        );
    }

    /// The CLI's `--take` refers to conflicts by number or ID; both must resolve, and unknown
    /// ones must not.
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

    /// Writing replaces the file in one step and leaves no temporary file behind.
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

    /// Each candidate is checked by parsing the whole file with it; the demo's plain "both"
    /// does not parse because git moved the shared brace.
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

    /// The closed variants of "both" repair the brace git moved out of the conflict, and the
    /// merged file parses.
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

    /// Closed variants are offered only when they repair something: not when plain "both"
    /// already parses.
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

    /// Without a grammar nothing is checked, and no closed variant is offered.
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

    /// When the grammar rejects the file on both sides, a parse failure says nothing about
    /// the merge, so nothing is checked and nothing is blocked.
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
