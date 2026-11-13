//! A merge session on one file: its conflicts, the candidate chosen for each,
//! and the merged text once every conflict has one.
//!
//! Nothing is written while a conflict is left: `git mergetool` then keeps
//! the file as conflicted, and the user's work in it is untouched.

use std::ffi::OsString;
use std::fmt;
use std::fs::{self, File};
use std::io::{self, Write};
use std::ops::Range;
use std::path::{Path, PathBuf};

use crate::command::{Command, CommandError, Event};
use crate::conflict::{self, Conflict, ParseError, Segment};
use crate::git::{Git, GitError};
use crate::highlight::{Highlighter, Span};
use crate::indent::{self, Newline};
use crate::operation::{self, Operation};
use crate::presentation;
use crate::resolution::{self, Analysis, Candidate, CandidateKind};
use crate::syntax::{Language, ParseCheck, ParsedFile};

/// The side panes of the merge editor; the result pane has its own
/// methods, since it depends on the choices.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Pane {
    Ours,
    Theirs,
}

/// The four files `git mergetool` passes to a merge tool.
pub struct MergetoolFiles {
    pub base: PathBuf,
    pub local: PathBuf,
    pub remote: PathBuf,
    pub merged: PathBuf,
}

/// Where a merge stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Status {
    pub resolved: usize,
    pub total: usize,
    /// The merged file's parse check, once every conflict has a choice.
    pub merged: ParseCheck,
    pub language: Option<Language>,
    pub can_undo: bool,
    pub can_redo: bool,
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

/// Conflicts with a choice each: the choices a command sets, or the ones
/// they replaced.
type Change = Vec<(usize, Option<Choice>)>;

/// One step of the history: the command that made it, so undo can say
/// what it undoes, and the choices it replaced.
struct Step {
    command: Command,
    previous: Change,
}

/// Where a session's conflicts come from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    /// The file as it is on disk.
    File,
    /// Rebuilt from git's three versions to show the base, since the file
    /// had none in its markers and was still as git wrote it.
    Rebuilt,
    /// The file as it is on disk, without the base: it was edited since git
    /// wrote it, and rebuilding the conflicts would drop those edits.
    EditedFile,
}

pub struct MergeSession {
    segments: Vec<Segment>,
    conflicts: Vec<Conflict>,
    analyses: Vec<Analysis>,
    choices: Vec<Option<Choice>>,
    /// Changes to undo, the latest last, and changes undone that can be
    /// redone. A new change clears the redo history, as in any editor.
    undo_history: Vec<Step>,
    redo_history: Vec<Step>,
    origin: Origin,
    /// Where `Command::Save` writes: the file the session was read from.
    file: Option<PathBuf>,
    /// The file's text as last read or written, to tell whether something
    /// else changed it before writing over it.
    on_disk: String,
    language: Option<Language>,
    /// Whether a reference version of the file parses. Grammars reject some
    /// valid code; without such a reference, a parse failure says nothing
    /// about the merge, so nothing is checked.
    checkable: bool,
    /// The file with every conflict taken from ours, parsed, and each
    /// conflict's byte range in it. Every other version of the file
    /// (candidates, the result, a draft) is parsed from it incrementally.
    reference: Option<ParsedFile>,
    ranges: Vec<Range<usize>>,
    highlighter: Option<Highlighter>,
    /// Highlight spans of the ours and theirs versions of the file, by line;
    /// they never change during the session.
    ours_spans: Vec<Vec<Span>>,
    theirs_spans: Vec<Vec<Span>>,
    /// The file's indent unit, for text typed by hand.
    indent_unit: String,
}

impl MergeSession {
    /// A session on a text with conflict markers, read from the file it
    /// will be saved to. With a language, every candidate is checked by
    /// parsing.
    pub fn from_text(text: &str, language: Option<Language>) -> Result<MergeSession, ParseError> {
        MergeSession::from_text_with_marker_size(text, language, conflict::DEFAULT_MARKER_SIZE)
    }

    /// Like `from_text`, for a file whose markers have `marker_size`
    /// characters (`conflict::marker_size`).
    pub fn from_text_with_marker_size(
        text: &str,
        language: Option<Language>,
        marker_size: usize,
    ) -> Result<MergeSession, ParseError> {
        Ok(MergeSession::from_segments(
            conflict::parse_with_marker_size(text, marker_size)?,
            Origin::File,
            text.to_string(),
            language,
        ))
    }

    /// The file the session was read from, where `Command::Save` writes.
    pub fn with_file(mut self, path: PathBuf) -> MergeSession {
        self.file = Some(path);
        self
    }

    fn from_segments(
        segments: Vec<Segment>,
        origin: Origin,
        on_disk: String,
        language: Option<Language>,
    ) -> MergeSession {
        let conflicts: Vec<Conflict> = conflict::conflicts(&segments).cloned().collect();
        let mut analyses = resolution::analyze(&segments);
        let (ours_text, ranges) = ours_file(&segments);
        let reference = language.and_then(|language| ParsedFile::new(language, ours_text));
        let checkable = match &reference {
            Some(reference) => verify(&segments, &conflicts, &mut analyses, reference, &ranges),
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
            origin,
            file: None,
            on_disk,
            language,
            checkable,
            reference,
            ranges,
            highlighter,
            ours_spans: Vec::new(),
            theirs_spans: Vec::new(),
            indent_unit: String::new(),
        };
        session.indent_unit = indent::indent_unit(&session.side_text(Pane::Ours));
        let ours: Vec<&str> = session
            .conflicts
            .iter()
            .map(|c| c.ours.text.as_str())
            .collect();
        let theirs: Vec<&str> = session
            .conflicts
            .iter()
            .map(|c| c.theirs.text.as_str())
            .collect();
        session.ours_spans = session.highlight_variant(&ours);
        session.theirs_spans = session.highlight_variant(&theirs);
        session
    }

    /// Spans per line of `text`; none without a highlighter.
    /// The reference file with conflict `i` replaced by `texts[i]`, parsed
    /// incrementally.
    fn variant(&self, texts: &[&str]) -> Option<ParsedFile> {
        let replacements: Vec<(Range<usize>, &str)> = self
            .ranges
            .iter()
            .zip(texts)
            .zip(&self.conflicts)
            // Conflicts left as ours need no edit.
            .filter(|((_, text), conflict)| **text != conflict.ours.text)
            .map(|((range, text), _)| (range.clone(), *text))
            .collect();
        self.reference.as_ref()?.with(&replacements)
    }

    /// The spans of every line of the file with conflict `i` replaced by
    /// `texts[i]`.
    fn highlight_variant(&self, texts: &[&str]) -> Vec<Vec<Span>> {
        let (Some(highlighter), Some(parsed)) = (&self.highlighter, self.variant(texts)) else {
            return Vec::new();
        };
        highlighter.lines_in(&parsed, 0..parsed.text().len())
    }

    /// The spans of the result pane: chosen candidates, nothing for
    /// unresolved conflicts.
    pub(crate) fn result_spans(&self) -> Vec<Vec<Span>> {
        let texts: Vec<&str> = (0..self.conflicts.len())
            .map(|index| self.chosen_text(index).unwrap_or(""))
            .collect();
        self.highlight_variant(&texts)
    }

    /// The text a side pane shows, as one file: the common text with each
    /// conflict's side.
    pub(crate) fn side_text(&self, pane: Pane) -> String {
        let mut text = String::new();
        for segment in &self.segments {
            match segment {
                Segment::Common(common) => text.push_str(common),
                Segment::Conflict(conflict) => match pane {
                    Pane::Ours => text.push_str(&conflict.ours.text),
                    Pane::Theirs => text.push_str(&conflict.theirs.text),
                },
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
                        Pane::Ours => &found.ours.text,
                        Pane::Theirs => &found.theirs.text,
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
        if !self.checkable {
            return ParseCheck::NotChecked;
        }
        let texts: Option<Vec<&str>> = (0..self.conflicts.len())
            .map(|index| self.chosen_text(index))
            .collect();
        match texts.and_then(|texts| self.variant(&texts)) {
            Some(parsed) => parsed.check(),
            None => ParseCheck::NotChecked,
        }
    }

    /// Whether the conflicts were rebuilt from the three versions because
    /// the file had no base in its markers.
    pub fn rebuilt(&self) -> bool {
        self.origin == Origin::Rebuilt
    }

    pub fn origin(&self) -> Origin {
        self.origin
    }

    /// Runs a command: the only way a UI changes the session. Returns what
    /// changed, nothing when the command changes nothing; a refused command
    /// leaves the session as it was.
    pub fn apply(&mut self, command: Command) -> Result<Vec<Event>, CommandError> {
        Ok(match command {
            Command::Choose {
                conflict,
                candidate,
            } => self.choose(conflict, candidate)?,
            Command::Edit { conflict, text } => self.edit(conflict, &text)?,
            Command::ApplySuggestions => self.apply_suggestions(),
            Command::Undo => self.undo(),
            Command::Redo => self.redo(),
            Command::Save { overwrite } => {
                self.save(overwrite)?;
                vec![Event::Saved]
            }
        })
    }

    /// Writes the merged text to the file the session was read from.
    /// Refuses while a conflict is left, and when the file changed on disk
    /// since it was read (resolved in another editor, `git checkout`, an
    /// aborted rebase) unless `overwrite` says to write over it anyway.
    fn save(&mut self, overwrite: bool) -> Result<(), SaveError> {
        let path = self.file.clone().ok_or(SaveError::NoFile)?;
        let path = path.as_path();
        let text = self
            .merged_text()
            .ok_or(SaveError::Unresolved(self.unresolved()))?;
        if !overwrite {
            match fs::read(path) {
                Ok(bytes) if bytes == self.on_disk.as_bytes() => {}
                Ok(_) => return Err(SaveError::ChangedOnDisk(path.to_path_buf())),
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    return Err(SaveError::ChangedOnDisk(path.to_path_buf()));
                }
                Err(error) => return Err(SaveError::Io(path.to_path_buf(), error)),
            }
        }
        write_atomically(path, &text).map_err(|error| SaveError::Io(path.to_path_buf(), error))?;
        self.on_disk = text;
        Ok(())
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
    /// ("cb5d1a4e2").
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
    pub(crate) fn choose(
        &mut self,
        index: usize,
        kind: CandidateKind,
    ) -> Result<Vec<Event>, ChooseError> {
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
        let command = Command::Choose {
            conflict: index,
            candidate: kind,
        };
        Ok(self.record(command, vec![(index, Some(Choice::Candidate(kind)))]))
    }

    /// Resolves a conflict with text written by hand, checked by parsing
    /// like a candidate. A text without a final line break gets one, so the
    /// line after the conflict stays on its own line.
    pub(crate) fn edit(&mut self, index: usize, text: &str) -> Result<Vec<Event>, ChooseError> {
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
        let command = Command::Edit {
            conflict: index,
            text: text.clone(),
        };
        Ok(self.record(
            command,
            vec![(index, Some(Choice::Edited(Edit { text, parse })))],
        ))
    }

    /// What Return does while editing conflict `index` by hand, with
    /// `before` the draft's text before the cursor and `after` the rest of
    /// the cursor's line. `Newline::text` replaces the current line up to
    /// the cursor. The file before the conflict is searched too, for a
    /// block opened outside it.
    pub fn newline(&self, index: usize, before: &str, after: &str) -> Newline {
        let mut text = self.text_before_conflict(index);
        text.push_str(before);
        indent::newline(&text, after, &self.indent_unit, self.language)
    }

    /// The indentation for the current line when `typed` is typed while
    /// editing conflict `index`, with `before` the draft's text before the
    /// cursor; `None` to type it as it is. The file before the conflict is
    /// searched too, for a block opened outside it.
    pub fn typed_indent(&self, index: usize, before: &str, typed: char) -> Option<String> {
        let mut text = self.text_before_conflict(index);
        text.push_str(before);
        indent::typed_indent(&text, typed, &self.indent_unit, self.language)
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
        let (Some(highlighter), Some(parsed)) = (&self.highlighter, self.variant(&texts)) else {
            return Vec::new();
        };
        // Only the draft's lines are painted: where it starts once the
        // conflicts before it have their own length.
        let shift: isize = (0..index)
            .map(|other| {
                texts[other].len() as isize - self.conflicts[other].ours.text.len() as isize
            })
            .sum();
        let start = self.ranges[index].start.saturating_add_signed(shift);
        highlighter.lines_in(&parsed, start..start + draft.len())
    }

    /// Whether the file parses with `text` resolving conflict `index` and
    /// the other conflicts filled with ours, as candidates are checked.
    fn check_in_place(&self, index: usize, text: &str) -> ParseCheck {
        if !self.checkable {
            return ParseCheck::NotChecked;
        }
        let mut texts: Vec<&str> = self
            .conflicts
            .iter()
            .map(|c| c.ours.text.as_str())
            .collect();
        texts[index] = text;
        match self.variant(&texts) {
            Some(parsed) => parsed.check(),
            None => ParseCheck::NotChecked,
        }
    }

    /// Chooses the rule suggestion for every conflict that has one and no
    /// choice yet, as one step of the history.
    fn apply_suggestions(&mut self) -> Vec<Event> {
        let mut changes = Vec::new();
        for (index, analysis) in self.analyses.iter().enumerate() {
            if self.choices[index].is_none()
                && let Some(suggestion) = analysis.suggestion
            {
                changes.push((index, Some(Choice::Candidate(suggestion))));
            }
        }
        self.record(Command::ApplySuggestions, changes)
    }

    /// Sets choices as one step of the history, made by `command`. Choices
    /// already in place are left out, so choosing the chosen candidate
    /// again adds no empty step to undo.
    fn record(&mut self, command: Command, changes: Change) -> Vec<Event> {
        let changes: Change = changes
            .into_iter()
            .filter(|(index, choice)| self.choices[*index] != *choice)
            .collect();
        if changes.is_empty() {
            return Vec::new();
        }
        let conflicts = changes.iter().map(|(index, _)| *index).collect();
        let previous = self.swap(changes);
        self.undo_history.push(Step { command, previous });
        self.redo_history.clear();
        vec![Event::ChoicesChanged { conflicts }, self.history_changed()]
    }

    fn history_changed(&self) -> Event {
        Event::HistoryChanged {
            can_undo: self.can_undo(),
            can_redo: self.can_redo(),
        }
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

    /// Undoes the last step; no event when there is none.
    fn undo(&mut self) -> Vec<Event> {
        let Some(step) = self.undo_history.pop() else {
            return Vec::new();
        };
        let (command, conflicts, previous) = self.revert(step);
        self.redo_history.push(Step {
            command: command.clone(),
            previous,
        });
        vec![
            Event::Undone { command },
            Event::ChoicesChanged { conflicts },
            self.history_changed(),
        ]
    }

    /// Redoes the last step undone; no event when there is none.
    fn redo(&mut self) -> Vec<Event> {
        let Some(step) = self.redo_history.pop() else {
            return Vec::new();
        };
        let (command, conflicts, previous) = self.revert(step);
        self.undo_history.push(Step {
            command: command.clone(),
            previous,
        });
        vec![
            Event::Redone { command },
            Event::ChoicesChanged { conflicts },
            self.history_changed(),
        ]
    }

    /// Puts a step's choices back: its command, the conflicts it touches,
    /// and the choices it replaced, which make the opposite step.
    fn revert(&mut self, step: Step) -> (Command, Vec<usize>, Change) {
        let conflicts = step.previous.iter().map(|(index, _)| *index).collect();
        let previous = self.swap(step.previous);
        (step.command, conflicts, previous)
    }

    /// Where the merge stands, as a status bar shows it.
    pub fn status(&self) -> Status {
        Status {
            resolved: self.conflict_count() - self.unresolved(),
            total: self.conflict_count(),
            merged: self.merged_check(),
            language: self.language,
            can_undo: self.can_undo(),
            can_redo: self.can_redo(),
        }
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
    reference: &ParsedFile,
    ranges: &[Range<usize>],
) -> bool {
    let theirs: Vec<(Range<usize>, &str)> = ranges
        .iter()
        .cloned()
        .zip(conflicts.iter().map(|c| c.theirs.text.as_str()))
        .collect();
    let theirs_parses = reference
        .with(&theirs)
        .is_some_and(|parsed| parsed.check() == ParseCheck::Parses);
    if reference.check() != ParseCheck::Parses || !theirs_parses {
        return false;
    }

    // The other conflicts stay as ours while one is checked.
    let check = |index: usize, text: &str| match reference.with(&[(ranges[index].clone(), text)]) {
        Some(parsed) => parsed.check(),
        None => ParseCheck::NotChecked,
    };
    let verify_conflict = |index: usize, analysis: &mut Analysis| {
        add_closed_candidates(segments, &conflicts[index], index, analysis, &|text| {
            check(index, text) == ParseCheck::Parses
        });
        for candidate in &mut analysis.candidates {
            candidate.parse = check(index, &candidate.text);
        }
    };

    // Each conflict is checked on its own, so they are shared between the
    // processor's cores: a large file with many conflicts (a string
    // catalog) opens several times faster.
    let threads = std::thread::available_parallelism().map_or(1, |count| count.get());
    let per_thread = analyses.len().div_ceil(threads).max(1);
    std::thread::scope(|scope| {
        for (chunk_index, chunk) in analyses.chunks_mut(per_thread).enumerate() {
            let verify_conflict = &verify_conflict;
            scope.spawn(move || {
                for (offset, analysis) in chunk.iter_mut().enumerate() {
                    verify_conflict(chunk_index * per_thread + offset, analysis);
                }
            });
        }
    });
    true
}

/// The file with every conflict taken from ours, and each conflict's byte
/// range in it.
fn ours_file(segments: &[Segment]) -> (String, Vec<Range<usize>>) {
    let mut text = String::new();
    let mut ranges = Vec::new();
    for segment in segments {
        match segment {
            Segment::Common(common) => text.push_str(common),
            Segment::Conflict(conflict) => {
                let start = text.len();
                text.push_str(&conflict.ours.text);
                ranges.push(start..text.len());
            }
        }
    }
    (text, ranges)
}

/// Adds "both, closed" when plain "both" does not parse: each side followed
/// by the first one, two or three lines after the conflict, whichever count
/// first makes the file parse.
fn add_closed_candidates(
    segments: &[Segment],
    conflict: &Conflict,
    index: usize,
    analysis: &mut Analysis,
    parses_with: &dyn Fn(&str) -> bool,
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

/// Loads the session for the files `git mergetool` passed.
///
/// Uses the merged file as it is when it has no conflict left, or when
/// every conflict shows the base (`diff3` or `zdiff3` style), so edits made
/// before launching are kept. A file in the `merge` style has no base: its
/// conflicts are rebuilt from the three versions with
/// `git merge-file --zdiff3`, labelled `ours` and `theirs`, but only while
/// the file is still what git wrote. Once edited, it is used as it is,
/// without the base, rather than losing the edits.
pub fn load(
    git: &dyn Git,
    files: &MergetoolFiles,
    ours: &str,
    theirs: &str,
) -> Result<MergeSession, LoadError> {
    let language = Language::from_path(&files.merged);
    let merged = read_text(&files.merged)?;
    let marker_size = conflict::marker_size(git, &files.merged);
    // Markers git did not write are refused, rather than rebuilt over
    // whatever the user did to the file.
    let segments = conflict::parse_with_marker_size(&merged, marker_size)
        .map_err(|error| LoadError::Malformed(files.merged.clone(), error))?;
    let has_base = {
        let mut conflicts = conflict::conflicts(&segments).peekable();
        conflicts.peek().is_none() || conflicts.all(|conflict| conflict.base.is_some())
    };
    if has_base {
        return Ok(
            MergeSession::from_segments(segments, Origin::File, merged, language)
                .with_file(files.merged.clone()),
        );
    }

    let as_git_wrote = merge_file(git, files, ours, theirs, marker_size, None)?;
    let unchanged = conflict::parse_with_marker_size(&as_git_wrote, marker_size)
        .is_ok_and(|written| same_ignoring_labels(&segments, &written));
    if !unchanged {
        return Ok(
            MergeSession::from_segments(segments, Origin::EditedFile, merged, language)
                .with_file(files.merged.clone()),
        );
    }
    let text = merge_file(git, files, ours, theirs, marker_size, Some("--zdiff3"))?;
    let rebuilt = conflict::parse_with_marker_size(&text, marker_size).map_err(LoadError::Parse)?;
    Ok(
        MergeSession::from_segments(rebuilt, Origin::Rebuilt, merged, language)
            .with_file(files.merged.clone()),
    )
}

/// Whether two parsed files have the same text and the same conflict
/// sides, whatever their markers' labels.
pub(crate) fn same_ignoring_labels(a: &[Segment], b: &[Segment]) -> bool {
    a.len() == b.len()
        && a.iter().zip(b).all(|pair| match pair {
            (Segment::Common(a), Segment::Common(b)) => a == b,
            (Segment::Conflict(a), Segment::Conflict(b)) => {
                a.ours.text == b.ours.text && a.theirs.text == b.theirs.text
            }
            _ => false,
        })
}

/// The three versions merged by `git merge-file`, in the `merge` style
/// unless `style` says otherwise (`--zdiff3`).
pub(crate) fn merge_file(
    git: &dyn Git,
    files: &MergetoolFiles,
    ours: &str,
    theirs: &str,
    marker_size: usize,
    style: Option<&str>,
) -> Result<String, LoadError> {
    let local = path_text(&files.local)?;
    let base = path_text(&files.base)?;
    let remote = path_text(&files.remote)?;
    let marker_size = marker_size.to_string();
    let mut args = vec!["merge-file", "--stdout", "--marker-size", &marker_size];
    args.extend(style);
    args.extend(["-L", ours, "-L", "base", "-L", theirs, local, base, remote]);
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

/// Opens a file with conflict markers, saved back where it was read.
/// `git` runs in the file's folder (`GitCli::beside`): it reads the
/// marker size `.gitattributes` gives the file.
pub fn open_file(git: &dyn Git, path: &Path) -> Result<MergeSession, LoadError> {
    let text = read_text(path)?;
    let marker_size = match path.file_name() {
        Some(name) => conflict::marker_size(git, Path::new(name)),
        None => conflict::DEFAULT_MARKER_SIZE,
    };
    Ok(
        MergeSession::from_text_with_marker_size(&text, Language::from_path(path), marker_size)
            .map_err(LoadError::Parse)?
            .with_file(path.to_path_buf()),
    )
}

/// Opens the files `git mergetool` passes (`load`), with the sides named
/// after the operation git has in progress, which it returns too.
pub fn open_mergetool(
    git: &dyn Git,
    files: &MergetoolFiles,
) -> Result<(MergeSession, Option<Operation>), LoadError> {
    let operation = operation::current(git).ok().flatten();
    let (ours, theirs) = presentation::side_labels(operation.as_ref());
    Ok((load(git, files, &ours, &theirs)?, operation))
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
///
/// A symbolic link is followed: the file it points to is replaced, and the
/// link stays a link. The temporary file has a name of its own, never an
/// existing file's, and the directory is flushed once the rename is done,
/// so the new name survives a crash too.
pub fn write_atomically(path: &Path, text: &str) -> io::Result<()> {
    let is_link =
        fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_symlink());
    let target = if is_link {
        fs::canonicalize(path)?
    } else {
        path.to_path_buf()
    };
    let (mut file, temporary) = create_temporary(&target)?;
    let result = write_then_rename(&mut file, &target, &temporary, text);
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

/// A new file next to `path`, named after it, that did not exist before.
fn create_temporary(path: &Path) -> io::Result<(File, PathBuf)> {
    let Some(name) = path.file_name() else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "not a file path",
        ));
    };
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or(0);
    for attempt in 0..100 {
        let mut temporary_name = OsString::from(".");
        temporary_name.push(name);
        temporary_name.push(format!(
            ".scry-{}-{stamp}-{attempt}.tmp",
            std::process::id()
        ));
        let temporary = path.with_file_name(temporary_name);
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
        {
            Ok(file) => return Ok((file, temporary)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "no free temporary name",
    ))
}

fn write_then_rename(file: &mut File, path: &Path, temporary: &Path, text: &str) -> io::Result<()> {
    file.write_all(text.as_bytes())?;
    file.sync_all()?;
    if let Ok(metadata) = fs::metadata(path) {
        fs::set_permissions(temporary, metadata.permissions())?;
    }
    fs::rename(temporary, path)?;
    // Some systems (Windows) cannot open a directory: the rename is then
    // as durable as they make it.
    if let Some(directory) = path.parent()
        && let Ok(directory) = File::open(if directory.as_os_str().is_empty() {
            Path::new(".")
        } else {
            directory
        })
    {
        let _ = directory.sync_all();
    }
    Ok(())
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
    /// The merged file's markers are not ones git writes, usually after a
    /// partial edit.
    Malformed(PathBuf, ParseError),
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
            LoadError::Malformed(path, error) => write!(
                f,
                "{}: {error}; fix the markers, or run `git checkout --merge {}` to recreate the conflicts (this drops edits made to the file)",
                path.display(),
                path.display()
            ),
            LoadError::Git(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for LoadError {}

/// Why a session could not be saved.
#[derive(Debug)]
pub enum SaveError {
    /// The session was not read from a file.
    NoFile,
    /// Conflicts still without a resolution.
    Unresolved(usize),
    /// The file changed on disk since it was read.
    ChangedOnDisk(PathBuf),
    Io(PathBuf, io::Error),
}

impl fmt::Display for SaveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SaveError::NoFile => write!(f, "no file to write to"),
            SaveError::Unresolved(count) => write!(f, "{count} conflict(s) left"),
            SaveError::ChangedOnDisk(path) => {
                write!(f, "{} changed on disk since Scry read it", path.display())
            }
            SaveError::Io(path, error) => write!(f, "cannot write {}: {error}", path.display()),
        }
    }
}

impl std::error::Error for SaveError {}

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
        assert!(!session.can_undo() && session.undo().is_empty());
        session.choose(0, CandidateKind::Ours).unwrap();
        session.edit(0, "mine").unwrap();
        assert!(!session.undo().is_empty());
        assert_eq!(session.chosen_text(0), Some("a\n"));
        assert!(!session.undo().is_empty());
        assert_eq!(session.choice(0), None);
        assert!(session.can_redo());
        assert!(!session.redo().is_empty());
        assert_eq!(session.chosen_text(0), Some("a\n"));
        session.choose(0, CandidateKind::Theirs).unwrap();
        assert!(!session.can_redo());
        assert!(!session.undo().is_empty());
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
        assert_eq!(
            session.newline(0, "func stop() {", "}").text,
            "func stop() {\n    \n"
        );
    }

    /// A closing brace typed in a conflict finds its block's opening line
    /// in the file before the conflict too: here the function around it.
    #[test]
    fn closing_brace_finds_a_block_opened_before_the_conflict() {
        let text = "func a() {\n    if b {\n<<<<<<< ours\n        c()\n=======\n        d()\n>>>>>>> theirs\n    }\n}\n";
        let session = MergeSession::from_text(text, Some(Language::Swift)).unwrap();
        assert_eq!(
            session
                .typed_indent(0, "        c()\n        ", '}')
                .as_deref(),
            Some("    ")
        );
    }

    /// Applying suggestions is one step: one undo takes them all back.
    #[test]
    fn suggestions_undo_in_one_step() {
        let mut session = MergeSession::from_text(TEXT, None).unwrap();
        assert_eq!(
            session.apply_suggestions()[0],
            Event::ChoicesChanged { conflicts: vec![1] }
        );
        assert!(!session.undo().is_empty());
        assert_eq!(session.unresolved(), 2);
        assert!(!session.can_undo());
        // Nothing to suggest: no empty step in the history.
        session.choose(1, CandidateKind::Ours).unwrap();
        assert!(session.apply_suggestions().is_empty());
        assert!(!session.undo().is_empty());
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
        assert_eq!(
            session.apply_suggestions()[0],
            Event::ChoicesChanged { conflicts: vec![1] }
        );
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
        assert_eq!(session.find("cffffffff"), None);
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

    /// A file of the same name as the temporary one is never written over:
    /// the temporary file gets a name of its own.
    #[test]
    fn atomic_write_leaves_other_files_alone() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("file.txt");
        let old_temporary = dir.path().join(".file.txt.scry-tmp");
        fs::write(&path, "old\n").unwrap();
        fs::write(&old_temporary, "someone else's\n").unwrap();

        write_atomically(&path, "new\n").unwrap();

        assert_eq!(fs::read_to_string(&path).unwrap(), "new\n");
        assert_eq!(
            fs::read_to_string(&old_temporary).unwrap(),
            "someone else's\n"
        );
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 2);
    }

    /// A conflicted file reached through a symbolic link (a dotfiles
    /// repository, a shared config) is written where the link points, and
    /// the link stays a link.
    #[cfg(unix)]
    #[test]
    fn atomic_write_follows_a_symbolic_link() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("real.txt");
        let link = dir.path().join("link.txt");
        fs::write(&target, "old\n").unwrap();
        std::os::unix::fs::symlink(&target, &link).unwrap();

        write_atomically(&link, "new\n").unwrap();

        assert!(
            fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(fs::read_to_string(&target).unwrap(), "new\n");
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

    /// Every change returns what changed and what undo can do, so a UI
    /// refreshes from the events rather than guessing; undo names the
    /// command it undoes, since the history keeps commands.
    #[test]
    fn commands_return_what_changed() {
        let mut session = MergeSession::from_text(TEXT, None).unwrap();
        let choose = Command::Choose {
            conflict: 0,
            candidate: CandidateKind::Theirs,
        };

        assert_eq!(
            session.apply(choose.clone()).unwrap(),
            vec![
                Event::ChoicesChanged { conflicts: vec![0] },
                Event::HistoryChanged {
                    can_undo: true,
                    can_redo: false
                },
            ]
        );
        assert_eq!(
            session.apply(Command::Undo).unwrap(),
            vec![
                Event::Undone {
                    command: choose.clone()
                },
                Event::ChoicesChanged { conflicts: vec![0] },
                Event::HistoryChanged {
                    can_undo: false,
                    can_redo: true
                },
            ]
        );
        assert_eq!(
            session.apply(Command::Redo).unwrap()[0],
            Event::Redone { command: choose }
        );
    }

    /// A command that changes nothing returns no event and adds no step:
    /// choosing the chosen candidate again must not need two undos.
    #[test]
    fn choosing_the_chosen_candidate_changes_nothing() {
        let mut session = MergeSession::from_text(TEXT, None).unwrap();
        let choose = Command::Choose {
            conflict: 0,
            candidate: CandidateKind::Ours,
        };
        session.apply(choose.clone()).unwrap();

        assert!(session.apply(choose).unwrap().is_empty());
        session.apply(Command::Undo).unwrap();
        assert!(!session.can_undo());
        assert!(session.apply(Command::Undo).unwrap().is_empty());
    }

    /// A refused command leaves the session as it was, and saving a
    /// session that was not read from a file says so instead of writing
    /// somewhere.
    #[test]
    fn refused_commands_change_nothing() {
        let mut session = MergeSession::from_text(TEXT, None).unwrap();
        let refused = session.apply(Command::Choose {
            conflict: 9,
            candidate: CandidateKind::Ours,
        });
        assert!(matches!(
            refused,
            Err(CommandError::Choose(ChooseError::NoSuchConflict(10)))
        ));
        assert!(!session.can_undo());

        session.apply(Command::ApplySuggestions).unwrap();
        session
            .apply(Command::Choose {
                conflict: 0,
                candidate: CandidateKind::Ours,
            })
            .unwrap();
        assert!(matches!(
            session.apply(Command::Save { overwrite: false }),
            Err(CommandError::Save(SaveError::NoFile))
        ));
    }
}
