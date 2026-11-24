//! A merge session on one file: its conflicts, the candidate chosen for each,
//! and the merged text once every conflict has one.
//!
//! Nothing is written while a conflict is left: `git mergetool` then keeps
//! the file as conflicted, and the user's work in it is untouched.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fmt;
use std::fs::{self, File};
use std::io::{self, Write};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use crate::command::{Command, CommandError, Event};
use crate::conflict::{self, Conflict, ParseError, Segment};
use crate::git::{Git, GitError};
use crate::highlight::{Highlighter, Span};
use crate::indent::{self, Newline};
use crate::operation::{self, Operation};
use crate::presentation;
use crate::records::{Chosen, Record, Store};
use crate::resolution::{self, Analysis, Candidate, CandidateKind};
use crate::syntax::{Language, ParseCheck, ParsedFile};
use crate::words::{self, LineChange};

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
    /// The file has a language, but its grammar cannot read the file as
    /// both sides wrote it (newer syntax, macros such as
    /// `NS_ASSUME_NONNULL_BEGIN`): nothing is checked, and the user should
    /// know it rather than take silence for a pass.
    pub checks_off: bool,
    pub can_undo: bool,
    pub can_redo: bool,
    /// Conflicts resolved outside Scry the user has not reviewed yet.
    pub unreviewed: usize,
    /// The result is not what the file holds (or a conflict is open):
    /// there is something to write. Marking a resolution reviewed leaves
    /// the file as it is.
    pub unwritten: bool,
}

/// How a conflict is resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Choice {
    /// One of the candidates the core offers.
    Candidate(CandidateKind),
    /// Text the user wrote by hand.
    Edited(Edit),
    /// What the file held when it was opened for review, resolved outside
    /// Scry: by the user's agent, another editor or a person.
    Outside(Outside),
}

/// A conflict resolved outside Scry, as the file holds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outside {
    /// The text, with the final line break git adds when the file lacks it.
    pub text: String,
    /// The candidate the text is, when it is one.
    pub matches: Option<CandidateKind>,
    /// What the agent said about it, when it resolved it through Scry's
    /// MCP server and the file still holds that text.
    pub record: Option<Record>,
    /// Checked like a candidate.
    pub parse: ParseCheck,
    /// The user looked at it and kept it.
    pub reviewed: bool,
}

/// A conflict's text written by hand, with its check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edit {
    pub text: String,
    /// Checked like a candidate: the whole file parsed with this text in
    /// place.
    pub parse: ParseCheck,
}

/// The result pane's file as last highlighted: each conflict's chosen text
/// (empty while unresolved), the file parsed, and its spans by line.
struct ResultPaint {
    texts: Vec<String>,
    parsed: ParsedFile,
    spans: Vec<Vec<Span>>,
}

/// A conflict's line changes against its base: ours, then theirs.
pub(crate) type SideChanges = (Vec<LineChange>, Vec<LineChange>);

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
    /// Rebuilt from git's three versions for a file resolved outside Scry,
    /// each conflict's choice being what the file holds, to review.
    Resolved,
}

pub struct MergeSession {
    segments: Vec<Segment>,
    conflicts: Vec<Conflict>,
    /// Each conflict's candidates as the rules give them, unchecked.
    analyses: Vec<Analysis>,
    /// Each conflict's candidates once checked by parsing, with the
    /// "closed" variants of "both" added: computed the first time they are
    /// asked for, since checking every conflict of a large file up front
    /// took seconds.
    checked: Vec<OnceLock<Analysis>>,
    /// Each checked conflict's candidate checks with the exact line of
    /// each error, for the conflict shown in detail: the quick check can
    /// stop dozens of lines past an error on a large file.
    located: Vec<OnceLock<Vec<ParseCheck>>>,
    /// Each conflict's line changes, ours then theirs, against its base:
    /// the sides never change, so they are worked out once, when first
    /// shown.
    line_changes: Vec<OnceLock<SideChanges>>,
    /// The result pane as last highlighted, so a choice repaints only the
    /// lines it changes: painting a large file took most of each refresh.
    result_paint: Mutex<Option<ResultPaint>>,
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
    /// The file's line break, `"\r\n"` or `"\n"`: text typed by hand is
    /// written with it, so the file never mixes the two.
    line_break: &'static str,
    /// Whether ours and theirs end with a line break, when the file ends
    /// with a conflict. git adds one to both sides inside the markers, so
    /// only its versions of the file tell; `None` when unknown.
    final_breaks: Option<(bool, bool)>,
    /// Whether the file ends with a line break, for a file resolved outside
    /// Scry whose last conflict keeps what the file holds.
    outside_break: Option<bool>,
    /// Where the review of a file resolved outside Scry is kept, and the
    /// file's path from the repository root.
    reviews: Option<(Store, String)>,
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

    /// Whether ours and theirs end with a line break, read from git's
    /// versions of the file, so a file whose sides had no final line break
    /// does not gain one.
    pub fn with_final_breaks(mut self, ours: bool, theirs: bool) -> MergeSession {
        self.final_breaks = Some((ours, theirs));
        if ours != theirs && self.ends_with_conflict() {
            self.split_theirs_from_ours();
        }
        self
    }

    /// Offers theirs as its own candidate in the last conflict when it has
    /// the same text as ours: the sides still differ by their final line
    /// break, which only the side taken keeps, as `git merge-file --theirs`
    /// does.
    fn split_theirs_from_ours(&mut self) {
        let Some(analysis) = self.analyses.last_mut() else {
            return;
        };
        let Some(at) = analysis
            .candidates
            .iter()
            .position(|candidate| candidate.aliases.contains(&CandidateKind::Theirs))
        else {
            return;
        };
        let ours = &mut analysis.candidates[at];
        ours.aliases.retain(|kind| *kind != CandidateKind::Theirs);
        let theirs = Candidate {
            kind: CandidateKind::Theirs,
            aliases: Vec::new(),
            ..ours.clone()
        };
        let at = at + 1;
        analysis.candidates.insert(at, theirs);
    }

    /// The common text right before and right after conflict `index`, at
    /// most `lines` lines each: what an agent reads around it. It stops at
    /// the conflicts on either side.
    pub fn context(&self, index: usize, lines: usize) -> (String, String) {
        let Some(position) = self
            .segments
            .iter()
            .enumerate()
            .filter(|(_, segment)| matches!(segment, Segment::Conflict(_)))
            .nth(index)
            .map(|(position, _)| position)
        else {
            return (String::new(), String::new());
        };
        let common = |at: Option<usize>| match at.and_then(|at| self.segments.get(at)) {
            Some(Segment::Common(text)) => text.split_inclusive('\n').collect::<Vec<_>>(),
            _ => Vec::new(),
        };
        let before = common(position.checked_sub(1));
        let after = common(Some(position + 1));
        (
            before[before.len().saturating_sub(lines)..].concat(),
            after.into_iter().take(lines).collect(),
        )
    }

    /// Whether the file ends with a conflict, where git adds a line break
    /// the sides may not have had.
    pub fn ends_with_conflict(&self) -> bool {
        matches!(self.segments.last(), Some(Segment::Conflict(_)))
    }

    /// The file the session was read from, where `Command::Save` writes.
    pub fn with_file(mut self, path: PathBuf) -> MergeSession {
        self.file = Some(path);
        self
    }

    pub(crate) fn from_segments(
        mut segments: Vec<Segment>,
        origin: Origin,
        on_disk: String,
        language: Option<Language>,
    ) -> MergeSession {
        // Rebuilt conflicts were parsed from a text that is not the file:
        // each starts where the file's own conflict does, so a line number
        // shown leads to it in another editor.
        if origin == Origin::Rebuilt {
            let lines = opening_lines(&on_disk);
            let rebuilt = conflict::conflicts(&segments).count();
            if lines.len() == rebuilt {
                let starts = segments.iter_mut().filter_map(|segment| match segment {
                    Segment::Conflict(conflict) => Some(conflict),
                    Segment::Common(_) => None,
                });
                for (conflict, line) in starts.zip(lines) {
                    conflict.start_line = line;
                }
            }
        }
        let conflicts: Vec<Conflict> = conflict::conflicts(&segments).cloned().collect();
        let analyses = resolution::analyze(&segments);
        let (ours_text, ranges) = ours_file(&segments);
        let reference = language.and_then(|language| ParsedFile::new(language, ours_text));
        let checkable = match &reference {
            Some(reference) => parses_both_ways(&conflicts, reference, &ranges),
            None => false,
        };
        let choices = vec![None; conflicts.len()];
        let highlighter = language.and_then(Highlighter::new);
        let line_break = line_break(&on_disk);
        let mut session = MergeSession {
            segments,
            conflicts,
            checked: analyses.iter().map(|_| OnceLock::new()).collect(),
            located: analyses.iter().map(|_| OnceLock::new()).collect(),
            line_changes: analyses.iter().map(|_| OnceLock::new()).collect(),
            result_paint: Mutex::new(None),
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
            line_break,
            final_breaks: None,
            outside_break: None,
            reviews: None,
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
        let Some(highlighter) = &self.highlighter else {
            return Vec::new();
        };
        let mut paint = self
            .result_paint
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(last) = paint.as_ref()
            && let Some(next) = self.repaint(highlighter, last, &texts)
        {
            let spans = next.spans.clone();
            *paint = Some(next);
            return spans;
        }
        let Some(parsed) = self.variant(&texts) else {
            return Vec::new();
        };
        let spans = highlighter.lines_in(&parsed, 0..parsed.text().len());
        *paint = Some(ResultPaint {
            texts: texts.iter().map(|text| text.to_string()).collect(),
            parsed,
            spans: spans.clone(),
        });
        spans
    }

    /// The result pane painted again from `last` for the new chosen
    /// `texts`: only the conflicts whose text changed are parsed again, and
    /// only the lines whose syntax may differ are painted; the others keep
    /// their spans. `None` when that cannot be done safely.
    fn repaint(
        &self,
        highlighter: &Highlighter,
        last: &ResultPaint,
        texts: &[&str],
    ) -> Option<ResultPaint> {
        // Each changed conflict's bytes in the last result file.
        let mut replacements: Vec<(Range<usize>, &str)> = Vec::new();
        let mut offset = 0;
        let mut index = 0;
        for segment in &self.segments {
            match segment {
                Segment::Common(common) => offset += common.len(),
                Segment::Conflict(_) => {
                    let old = &last.texts[index];
                    if old != texts[index] {
                        replacements.push((offset..offset + old.len(), texts[index]));
                    }
                    offset += old.len();
                    index += 1;
                }
            }
        }
        if replacements.is_empty() {
            return Some(ResultPaint {
                texts: last.texts.clone(),
                parsed: last.parsed.clone(),
                spans: last.spans.clone(),
            });
        }
        let (parsed, changed) = last.parsed.with_changes(&replacements)?;
        let line_count = parsed.text().lines().count();

        // The unchanged lines keep their spans, shifted by the lines each
        // replacement adds or removes; the replaced ones are painted.
        let mut spans: Vec<Option<Vec<Span>>> = Vec::with_capacity(line_count);
        let mut old_row = 0;
        let mut shift: isize = 0;
        for (range, text) in &replacements {
            let new_start = (range.start as isize + shift) as usize;
            let old_rows = last.parsed.row(range.start)..last.parsed.row(range.end);
            let new_rows = parsed.row(new_start)..parsed.row(new_start + text.len());
            while spans.len() < new_rows.start {
                spans.push(Some(last.spans.get(old_row)?.clone()));
                old_row += 1;
            }
            spans.extend(new_rows.map(|_| None));
            old_row = old_rows.end;
            shift += text.len() as isize - range.len() as isize;
        }
        while spans.len() < line_count {
            spans.push(Some(last.spans.get(old_row)?.clone()));
            old_row += 1;
        }
        if spans.len() != line_count || old_row != last.spans.len() {
            return None;
        }
        for range in changed {
            let end = parsed.row(range.end.saturating_sub(1).max(range.start));
            for row in parsed.row(range.start)..=end {
                if let Some(line) = spans.get_mut(row) {
                    *line = None;
                }
            }
        }

        // Paint each run of lines left without spans in one go.
        let mut row = 0;
        while row < spans.len() {
            if spans[row].is_some() {
                row += 1;
                continue;
            }
            let start = row;
            while row < spans.len() && spans[row].is_none() {
                row += 1;
            }
            let painted = highlighter.lines_in(&parsed, parsed.line_bytes(start..row));
            for (offset, line) in painted.into_iter().enumerate() {
                if let Some(slot) = spans.get_mut(start + offset) {
                    *slot = Some(line);
                }
            }
        }
        Some(ResultPaint {
            texts: texts.iter().map(|text| text.to_string()).collect(),
            parsed,
            spans: spans.into_iter().map(Option::unwrap_or_default).collect(),
        })
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
            // A closed "both" is only offered once checked.
            Choice::Candidate(kind) => self.checked[index]
                .get()
                .unwrap_or(&self.analyses[index])
                .candidates
                .iter()
                .find(|candidate| candidate.kind == *kind)
                .map(|candidate| candidate.text.as_str()),
            Choice::Edited(edit) => Some(edit.text.as_str()),
            Choice::Outside(outside) => Some(outside.text.as_str()),
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
            Command::MarkReviewed { conflict } => self.mark_reviewed(conflict)?,
            Command::Reopen { conflict } => self.reopen(conflict)?,
            Command::Undo => self.undo(),
            Command::Redo => self.redo(),
            Command::Save { overwrite } => {
                self.save(overwrite)?;
                return Ok(vec![Event::Saved]);
            }
        })
        .inspect(|events: &Vec<Event>| {
            if !events.is_empty() {
                self.keep_reviews();
            }
        })
    }

    /// The choices of a file resolved outside Scry: each resolved
    /// conflict's index and what it holds, checked here. `ends_with_break`
    /// is the file's own final line break.
    pub(crate) fn resolved_outside(
        mut self,
        outside: Vec<(usize, Outside)>,
        ends_with_break: bool,
    ) -> MergeSession {
        for (index, outside) in outside {
            let parse = self.check_in_place(index, &outside.text);
            self.choices[index] = Some(Choice::Outside(Outside { parse, ..outside }));
        }
        self.outside_break = Some(ends_with_break);
        self
    }

    /// Names the conflicts `ids`, those of the file as git wrote it when
    /// the session shows them rebuilt with their base: the ids the
    /// agent, the CLI and a review of the file see. Kept as computed when
    /// the counts differ.
    fn with_ids(mut self, ids: Vec<String>) -> MergeSession {
        if ids.len() == self.analyses.len() {
            for (analysis, id) in self.analyses.iter_mut().zip(ids) {
                analysis.id = id;
            }
        }
        self
    }

    /// Keeps the review in `store` after every change, for the file at
    /// `path` from the repository root.
    pub(crate) fn keeping_reviews(mut self, store: Store, path: String) -> MergeSession {
        self.reviews = Some((store, path));
        self
    }

    /// Writes what is reviewed. A failure leaves the review as it was last
    /// kept: the user marks it again, nothing in the file is lost.
    fn keep_reviews(&self) {
        if let Some((store, path)) = &self.reviews {
            let _ = store.set_reviewed(path, &self.reviewed());
            let _ = store.set_chosen(path, &self.chosen());
        }
    }

    /// The conflicts the user resolved in Scry, with what they chose and
    /// its text: opening the file again gives them back as chosen.
    pub fn chosen(&self) -> BTreeMap<String, Chosen> {
        (0..self.choices.len())
            .filter_map(|index| {
                let candidate = match self.choices[index].as_ref()? {
                    Choice::Candidate(kind) => Some(*kind),
                    Choice::Edited(_) => None,
                    Choice::Outside(_) => return None,
                };
                let text = self.chosen_text(index)?.to_string();
                Some((self.analyses[index].id.clone(), Chosen { candidate, text }))
            })
            .collect()
    }

    /// Gives back what the user chose in Scry (`chosen`) to the conflicts
    /// still holding that text, in place of a resolution from outside.
    pub(crate) fn restoring(mut self, chosen: &BTreeMap<String, Chosen>) -> MergeSession {
        for index in 0..self.choices.len() {
            let Some(Choice::Outside(outside)) = &self.choices[index] else {
                continue;
            };
            let Some(chosen) = chosen
                .get(&self.analyses[index].id)
                .filter(|chosen| chosen.text == outside.text)
            else {
                continue;
            };
            let outside = outside.clone();
            self.choices[index] = Some(match chosen.candidate {
                Some(kind) => Choice::Candidate(kind),
                None => Choice::Edited(Edit {
                    text: outside.text.clone(),
                    parse: outside.parse,
                }),
            });
            // A candidate whose text is not that one any more stays as
            // the file holds it.
            if self.chosen_text(index) != Some(outside.text.as_str()) {
                self.choices[index] = Some(Choice::Outside(outside));
            }
        }
        self
    }

    /// The conflicts whose resolution the user reviewed, with its text:
    /// those marked reviewed, and those the user resolved in Scry, so that
    /// opening the file again does not ask to review them.
    pub fn reviewed(&self) -> BTreeMap<String, String> {
        (0..self.choices.len())
            .filter(|&index| match &self.choices[index] {
                Some(Choice::Outside(outside)) => outside.reviewed,
                Some(_) => true,
                None => false,
            })
            .filter_map(|index| {
                let text = self.chosen_text(index)?;
                Some((self.analyses[index].id.clone(), text.to_string()))
            })
            .collect()
    }

    /// Conflicts resolved outside Scry and not reviewed yet.
    pub fn unreviewed(&self) -> usize {
        self.choices
            .iter()
            .filter(|choice| matches!(choice, Some(Choice::Outside(outside)) if !outside.reviewed))
            .count()
    }

    /// Marks a conflict resolved outside Scry as reviewed, as one step of
    /// the history; nothing for any other conflict.
    fn mark_reviewed(&mut self, index: usize) -> Result<Vec<Event>, ChooseError> {
        let Some(choice) = self.choices.get(index) else {
            return Err(ChooseError::NoSuchConflict(index.saturating_add(1)));
        };
        let Some(Choice::Outside(outside)) = choice else {
            return Ok(Vec::new());
        };
        let reviewed = Outside {
            reviewed: true,
            ..outside.clone()
        };
        let command = Command::MarkReviewed { conflict: index };
        Ok(self.record(command, vec![(index, Some(Choice::Outside(reviewed)))]))
    }

    /// Takes a conflict's choice away, whatever it is, as one step of the
    /// history: the conflict is open again.
    fn reopen(&mut self, index: usize) -> Result<Vec<Event>, ChooseError> {
        if index >= self.choices.len() {
            return Err(ChooseError::NoSuchConflict(index.saturating_add(1)));
        }
        Ok(self.record(Command::Reopen { conflict: index }, vec![(index, None)]))
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

    /// Whether the file is no longer what the session read or last wrote,
    /// so that a save without `overwrite` would be refused: asked before
    /// writing, so the user is told once.
    pub fn changed_on_disk(&self) -> bool {
        let Some(path) = self.file.as_deref() else {
            return false;
        };
        fs::read(path).map_or(true, |bytes| bytes != self.on_disk.as_bytes())
    }

    pub fn conflict_count(&self) -> usize {
        self.conflicts.len()
    }

    /// The conflict at `index`, from 0. Panics when out of range, like a
    /// slice.
    pub fn conflict(&self, index: usize) -> &Conflict {
        &self.conflicts[index]
    }

    /// Conflict `index`'s candidates, checked by parsing when the file can
    /// be checked; the first call for a conflict checks it.
    pub fn analysis(&self, index: usize) -> &Analysis {
        let Some(reference) = self.reference.as_ref().filter(|_| self.checkable) else {
            return &self.analyses[index];
        };
        self.checked[index].get_or_init(|| {
            check_conflict(
                &self.segments,
                &self.conflicts,
                &self.analyses[index],
                reference,
                &self.ranges,
                index,
            )
        })
    }

    /// Conflict `index`'s line changes against its base, ours then theirs.
    pub(crate) fn line_changes(&self, index: usize) -> &SideChanges {
        self.line_changes[index].get_or_init(|| {
            let conflict = &self.conflicts[index];
            let base = conflict.base.as_ref().map(|base| base.text.as_str());
            (
                words::line_changes(base, &conflict.ours.text),
                words::line_changes(base, &conflict.theirs.text),
            )
        })
    }

    /// Conflict `index`'s candidates as far as they are known: checked once
    /// `analysis` checked them, as the rules give them before.
    pub fn analysis_so_far(&self, index: usize) -> &Analysis {
        self.checked[index].get().unwrap_or(&self.analyses[index])
    }

    /// Whether conflict `index`'s candidates are checked, or never will be
    /// (a file the grammar cannot check).
    pub fn is_checked(&self, index: usize) -> bool {
        !self.checkable || self.checked[index].get().is_some()
    }

    /// Checks every conflict not checked yet, spread over the processor's
    /// cores: for a front end that shows them all at once, such as the CLI.
    pub fn check_all(&self) {
        let Some(reference) = self.reference.as_ref().filter(|_| self.checkable) else {
            return;
        };
        let left: Vec<usize> = (0..self.conflicts.len())
            .filter(|index| self.checked[*index].get().is_none())
            .collect();
        let threads = std::thread::available_parallelism().map_or(1, |count| count.get());
        let per_thread = left.len().div_ceil(threads).max(1);
        let results: Vec<(usize, Analysis)> = std::thread::scope(|scope| {
            let handles: Vec<_> = left
                .chunks(per_thread)
                .map(|chunk| {
                    scope.spawn(move || {
                        chunk
                            .iter()
                            .map(|&index| {
                                let analysis = check_conflict(
                                    &self.segments,
                                    &self.conflicts,
                                    &self.analyses[index],
                                    reference,
                                    &self.ranges,
                                    index,
                                );
                                (index, analysis)
                            })
                            .collect::<Vec<_>>()
                    })
                })
                .collect();
            handles
                .into_iter()
                .flat_map(|handle| handle.join().unwrap_or_default())
                .collect()
        });
        for (index, analysis) in results {
            let _ = self.checked[index].set(analysis);
        }
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
        if index >= self.analyses.len() {
            return Err(ChooseError::NoSuchConflict(index.saturating_add(1)));
        }
        // The rules' candidates are offered checked or not; a closed "both"
        // only once checked.
        let offered = |analysis: &Analysis| analysis.candidates.iter().any(|c| c.answers(kind));
        let analysis = if offered(self.analysis_so_far(index)) {
            self.analysis_so_far(index)
        } else {
            self.analysis(index)
        };
        // A kind that gives the text of another candidate chooses that one.
        let Some(kind) = analysis
            .candidates
            .iter()
            .find(|candidate| candidate.answers(kind))
            .map(|candidate| candidate.kind)
        else {
            return Err(ChooseError::NotOffered {
                conflict: analysis.id.clone(),
                candidate: kind.id(),
            });
        };
        let command = Command::Choose {
            conflict: index,
            candidate: kind,
        };
        Ok(self.record(command, vec![(index, Some(Choice::Candidate(kind)))]))
    }

    /// Resolves a conflict with text written by hand, checked by parsing
    /// like a candidate. Every line break becomes the file's, whatever the
    /// front end or a paste used, and a text without a final line break
    /// gets one, so the line after the conflict stays on its own line.
    pub(crate) fn edit(&mut self, index: usize, text: &str) -> Result<Vec<Event>, ChooseError> {
        if index >= self.conflicts.len() {
            return Err(ChooseError::NoSuchConflict(index.saturating_add(1)));
        }
        let mut text = text.replace("\r\n", "\n");
        if !text.is_empty() && !text.ends_with('\n') {
            text.push('\n');
        }
        if self.line_break == "\r\n" {
            text = text.replace('\n', "\r\n");
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

    /// Whether the file parses with `text` resolving conflict `index`, the
    /// other conflicts filled with ours, without choosing it: an agent's
    /// proposal, checked before the user sees it.
    pub fn check_text(&self, index: usize, text: &str) -> ParseCheck {
        self.check_in_place(index, text)
    }

    /// Whether the file parses with `text` resolving conflict `index` and
    /// the other conflicts filled with ours, as candidates are checked.
    fn check_in_place(&self, index: usize, text: &str) -> ParseCheck {
        if !self.checkable {
            return ParseCheck::NotChecked;
        }
        // The other conflicts stay as ours, as in the reference file.
        match &self.reference {
            Some(reference) => reference.locate_with(&[(self.ranges[index].clone(), text)]),
            None => ParseCheck::NotChecked,
        }
    }

    /// `located_checks` when they are already computed, without
    /// computing them.
    pub(crate) fn located_so_far(&self, index: usize) -> Option<&[ParseCheck]> {
        self.located[index].get().map(Vec::as_slice)
    }

    /// Conflict `index`'s candidate checks, in the order of its analysis,
    /// each error at its exact line: only the texts that do not parse are
    /// checked again, and only those whose quick check stopped early are
    /// parsed in full, once per conflict.
    pub(crate) fn located_checks(&self, index: usize) -> &[ParseCheck] {
        self.located[index].get_or_init(|| {
            let reference = self.reference.as_ref().filter(|_| self.checkable);
            self.analysis(index)
                .candidates
                .iter()
                .map(|candidate| match (candidate.parse, reference) {
                    (ParseCheck::DoesNotParse { .. }, Some(reference)) => reference
                        .locate_with(&[(self.ranges[index].clone(), candidate.text.as_str())]),
                    (parse, _) => parse,
                })
                .collect()
        })
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
            checks_off: self.language.is_some() && !self.checkable,
            can_undo: self.can_undo(),
            can_redo: self.can_redo(),
            unreviewed: self.unreviewed(),
            unwritten: self.merged_text().is_none_or(|text| text != self.on_disk),
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
        if !self.ends_with_break() {
            let end = text.strip_suffix('\n').unwrap_or(&text);
            let end = end.strip_suffix('\r').unwrap_or(end).len();
            text.truncate(end);
        }
        Some(text)
    }

    /// Whether the merged file ends with a line break: as the side that
    /// ends the last conflict's choice did, when the file ends with a
    /// conflict; ours for a choice of neither side (base, by hand). An
    /// empty last choice adds no line: the break before it stays.
    fn ends_with_break(&self) -> bool {
        let Some((ours, theirs)) = self.final_breaks else {
            return true;
        };
        if !self.ends_with_conflict() {
            return true;
        }
        let last = self.choices.len() - 1;
        if self.chosen_text(last).is_some_and(str::is_empty) {
            return true;
        }
        if let Some(Some(Choice::Outside(_))) = self.choices.last() {
            return self.outside_break.unwrap_or(ours);
        }
        match self.choices.last() {
            Some(Some(Choice::Candidate(
                CandidateKind::Theirs
                | CandidateKind::OursThenTheirs
                | CandidateKind::OursThenTheirsClosed,
            ))) => theirs,
            _ => ours,
        }
    }
}

/// The lines, from 1, where a conflict opens in `text`: those starting
/// with `<<<<<<<`, whatever the marker size beyond it.
fn opening_lines(text: &str) -> Vec<usize> {
    text.lines()
        .enumerate()
        .filter(|(_, line)| line.starts_with("<<<<<<<"))
        .map(|(index, _)| index + 1)
        .collect()
}

/// The line break most of `text` uses; `"\n"` for a file without any.
pub(crate) fn line_break(text: &str) -> &'static str {
    let crlf = text.matches("\r\n").count();
    let lf = text.matches('\n').count() - crlf;
    if crlf > lf { "\r\n" } else { "\n" }
}

/// Whether candidates can be checked by parsing: only when the file
/// parses both with every conflict taken from ours and with every conflict
/// taken from theirs. Grammars reject some valid code (newer syntax,
/// macros), and a failure must never be blamed on a candidate for that.
fn parses_both_ways(
    conflicts: &[Conflict],
    reference: &ParsedFile,
    ranges: &[Range<usize>],
) -> bool {
    let theirs: Vec<(Range<usize>, &str)> = ranges
        .iter()
        .cloned()
        .zip(conflicts.iter().map(|c| c.theirs.text.as_str()))
        .collect();
    reference.check() == ParseCheck::Parses && reference.check_with(&theirs) == ParseCheck::Parses
}

/// Conflict `index`'s candidates checked by parsing the whole file with
/// each in place, the other conflicts as ours, and with the "closed"
/// variants of "both" added where they repair it. A text is parsed once,
/// and ours is the reference file itself, which parses.
fn check_conflict(
    segments: &[Segment],
    conflicts: &[Conflict],
    analysis: &Analysis,
    reference: &ParsedFile,
    ranges: &[Range<usize>],
    index: usize,
) -> Analysis {
    let mut analysis = analysis.clone();
    let checked = std::cell::RefCell::new(Vec::<(String, ParseCheck)>::new());
    let check = |text: &str| -> ParseCheck {
        if text == conflicts[index].ours.text {
            return ParseCheck::Parses;
        }
        if let Some((_, known)) = checked.borrow().iter().find(|(seen, _)| seen == text) {
            return *known;
        }
        let result = reference.check_with(&[(ranges[index].clone(), text)]);
        checked.borrow_mut().push((text.to_string(), result));
        result
    };
    add_closed_candidates(segments, &conflicts[index], index, &mut analysis, &|text| {
        check(text) == ParseCheck::Parses
    });
    for candidate in &mut analysis.candidates {
        candidate.parse = check(&candidate.text);
    }
    analysis
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
                match analysis.candidates.iter_mut().find(|c| c.text == text) {
                    Some(same) => same.aliases.push(kind),
                    None => analysis.candidates.push(Candidate {
                        kind,
                        text,
                        parse: ParseCheck::NotChecked,
                        aliases: Vec::new(),
                    }),
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
///
/// A file whose markers do not parse because a side has a line such as
/// `=======` (a Markdown underline) is rebuilt with longer markers, again
/// only while it is what git wrote.
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
    let segments = match conflict::parse_with_marker_size(&merged, marker_size) {
        Ok(segments) => segments,
        Err(error) => {
            return match rebuild_with_longer_markers(
                git,
                files,
                &merged,
                marker_size,
                ours,
                theirs,
            )? {
                Some(session) => Ok(session),
                None => Err(LoadError::Malformed(
                    in_repository(git, &files.merged),
                    error,
                )),
            };
        }
    };
    let has_base = {
        let mut conflicts = conflict::conflicts(&segments).peekable();
        if conflicts.peek().is_none() && left_as_ours(git, files, &merged) {
            return Err(LoadError::NotMerged(in_repository(git, &files.merged)));
        }
        conflicts.peek().is_none() || conflicts.all(|conflict| conflict.base.is_some())
    };
    let (ours_break, theirs_break) = (
        ends_with_break(&files.local),
        ends_with_break(&files.remote),
    );
    if has_base {
        return Ok(
            MergeSession::from_segments(segments, Origin::File, merged, language)
                .with_file(files.merged.clone())
                .with_final_breaks(ours_break, theirs_break),
        );
    }

    let as_git_wrote = merge_file(git, files, ours, theirs, marker_size, None)?;
    let unchanged = conflict::parse_with_marker_size(&as_git_wrote, marker_size)
        .is_ok_and(|written| same_ignoring_labels(&segments, &written));
    if !unchanged {
        return Ok(
            MergeSession::from_segments(segments, Origin::EditedFile, merged, language)
                .with_file(files.merged.clone())
                .with_final_breaks(ours_break, theirs_break),
        );
    }
    let text = merge_file(git, files, ours, theirs, marker_size, Some("--zdiff3"))?;
    let rebuilt = conflict::parse_with_marker_size(&text, marker_size).map_err(LoadError::Parse)?;
    Ok(
        MergeSession::from_segments(rebuilt, Origin::Rebuilt, merged, language)
            .with_file(files.merged.clone())
            .with_final_breaks(ours_break, theirs_break)
            .with_ids(resolution::ids(&segments)),
    )
}

/// Git did not merge `files.merged` (`-merge`, `merge=binary`, the
/// `binary` macro) and it still holds ours as git left it: answering it
/// resolved would stage ours and drop theirs unseen.
fn left_as_ours(git: &dyn Git, files: &MergetoolFiles, merged: &str) -> bool {
    let path = files.merged.to_string_lossy();
    let not_merged = git
        .run(&["check-attr", "merge", "--", &path])
        .is_ok_and(|output| {
            matches!(
                output.trim_end().rsplit(": ").next(),
                Some("unset" | "binary")
            )
        });
    not_merged && fs::read(&files.local).is_ok_and(|ours| ours == merged.as_bytes())
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

/// The session rebuilt from the three versions with markers longer than any
/// line of them starts with, for a file whose markers are ambiguous: a side
/// with a `=======` line, which git does not escape. `None` unless the file
/// is still what git wrote, in any conflict style, labels aside: an edited
/// file is never rebuilt over.
fn rebuild_with_longer_markers(
    git: &dyn Git,
    files: &MergetoolFiles,
    merged: &str,
    marker_size: usize,
    ours: &str,
    theirs: &str,
) -> Result<Option<MergeSession>, LoadError> {
    let mut as_git_wrote = false;
    for style in [None, Some("--diff3"), Some("--zdiff3")] {
        let written = merge_file(git, files, ours, theirs, marker_size, style)?;
        if without_labels(&written, marker_size) == without_labels(merged, marker_size) {
            as_git_wrote = true;
            break;
        }
    }
    if !as_git_wrote {
        return Ok(None);
    }
    let mut longest = marker_size;
    for path in [&files.base, &files.local, &files.remote] {
        let text = read_text(path)?;
        for line in text.lines() {
            let Some(first) = line.chars().next().filter(|c| "<|=>".contains(*c)) else {
                continue;
            };
            longest = longest.max(line.chars().take_while(|c| *c == first).count());
        }
    }
    let size = longest + 1;
    let text = merge_file(git, files, ours, theirs, size, Some("--zdiff3"))?;
    let Ok(segments) = conflict::parse_with_marker_size(&text, size) else {
        return Ok(None);
    };
    let session = MergeSession::from_segments(
        segments,
        Origin::Rebuilt,
        merged.to_string(),
        Language::from_path(&files.merged),
    )
    .with_file(files.merged.clone())
    .with_final_breaks(
        ends_with_break(&files.local),
        ends_with_break(&files.remote),
    );
    Ok(Some(session))
}

/// `text` with the labels cut off its marker lines, to compare a file with
/// what git writes under other labels.
fn without_labels(text: &str, marker_size: usize) -> Vec<&str> {
    text.split_inclusive('\n')
        .map(|line| {
            let marker = ["<", "|", ">"]
                .iter()
                .any(|c| line.starts_with(&c.repeat(marker_size)));
            match line.get(..marker_size) {
                Some(start) if marker => start,
                _ => line,
            }
        })
        .collect()
}

/// Opens a file with conflict markers, saved back where it was read.
/// `git` runs in the file's folder (`GitCli::beside`): it reads the
/// marker size `.gitattributes` gives the file, and the sides' final line
/// break from the index when the file ends with a conflict.
pub fn open_file(git: &dyn Git, path: &Path) -> Result<MergeSession, LoadError> {
    let text = read_text(path)?;
    let marker_size = match path.file_name() {
        Some(name) => conflict::marker_size(git, Path::new(name)),
        None => conflict::DEFAULT_MARKER_SIZE,
    };
    let session = match MergeSession::from_text_with_marker_size(
        &text,
        Language::from_path(path),
        marker_size,
    ) {
        Ok(session) => session.with_file(path.to_path_buf()),
        Err(error) => {
            return match rebuild_from_index(git, path, &text, marker_size)? {
                Some(session) => Ok(session),
                None => Err(LoadError::Parse(error)),
            };
        }
    };
    if !session.ends_with_conflict() {
        return Ok(session);
    }
    // The file's index stages hold the sides as they were.
    let stage_ends_with_break = |stage: u8| -> Option<bool> {
        let name = path.file_name()?.to_str()?;
        let output = git.output(&["show", &format!(":{stage}:./{name}")]).ok()?;
        (output.status == Some(0)).then(|| output.stdout.last().is_none_or(|byte| *byte == b'\n'))
    };
    Ok(match (stage_ends_with_break(2), stage_ends_with_break(3)) {
        (Some(ours), Some(theirs)) => session.with_final_breaks(ours, theirs),
        _ => session,
    })
}

/// `rebuild_with_longer_markers` for a file opened from the repository: its
/// three versions are read from the index into a temporary folder. `None`
/// when the index lacks one of them (added or deleted on one side).
fn rebuild_from_index(
    git: &dyn Git,
    path: &Path,
    text: &str,
    marker_size: usize,
) -> Result<Option<MergeSession>, LoadError> {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return Ok(None);
    };
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |time| time.as_nanos());
    let folder = std::env::temp_dir().join(format!("scry-{}-{nanos}", std::process::id()));
    fs::create_dir(&folder).map_err(|error| LoadError::Io(folder.clone(), error))?;
    let files = MergetoolFiles {
        base: folder.join("base"),
        local: folder.join("ours"),
        remote: folder.join("theirs"),
        merged: path.to_path_buf(),
    };
    let label = |marker: &str| {
        text.lines()
            .find_map(|line| line.strip_prefix(&format!("{} ", marker.repeat(marker_size))))
            .unwrap_or_default()
            .to_string()
    };
    let rebuilt = (|| {
        for (stage, file) in [(1, &files.base), (2, &files.local), (3, &files.remote)] {
            let output = git.output(&["show", &format!(":{stage}:./{name}")])?;
            if output.status != Some(0) {
                return Ok(None);
            }
            fs::write(file, output.stdout).map_err(|error| LoadError::Io(file.clone(), error))?;
        }
        rebuild_with_longer_markers(git, &files, text, marker_size, &label("<"), &label(">"))
    })();
    let _ = fs::remove_dir_all(&folder);
    rebuilt
}

/// `path` from the repository's root, as git commands take it there; as it
/// is when that cannot be told.
fn in_repository(git: &dyn Git, path: &Path) -> PathBuf {
    let relative = || -> Option<PathBuf> {
        let root = git.run(&["rev-parse", "--show-toplevel"]).ok()?;
        // Both resolved, since a temporary folder on macOS is a link.
        let root = fs::canonicalize(root.trim_end()).ok()?;
        let path = fs::canonicalize(path).ok()?;
        path.strip_prefix(root).ok().map(Path::to_path_buf)
    };
    relative().unwrap_or_else(|| path.to_path_buf())
}

/// Whether a file ends with a line break; an empty or unreadable one counts
/// as ending with one, so nothing is taken off.
fn ends_with_break(path: &Path) -> bool {
    fs::read(path)
        .ok()
        .and_then(|bytes| bytes.last().copied())
        .is_none_or(|byte| byte == b'\n')
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
///
/// A file with other hard links is written in place instead: a rename
/// would give this name a new file and leave the other names on the old
/// one. Extended attributes are not carried over by the rename.
pub fn write_atomically(path: &Path, text: &str) -> io::Result<()> {
    let is_link =
        fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_symlink());
    let target = if is_link {
        fs::canonicalize(path)?
    } else {
        path.to_path_buf()
    };
    if has_other_names(&target) {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .truncate(true)
            .open(&target)?;
        file.write_all(text.as_bytes())?;
        return file.sync_all();
    }
    let (mut file, temporary) = create_temporary(&target)?;
    let result = write_then_rename(&mut file, &target, &temporary, text);
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

/// A new file next to `path`, named after it, that did not exist before.
fn create_temporary(path: &Path) -> io::Result<(File, PathBuf)> {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or(0);
    create_temporary_stamped(path, stamp)
}

/// `create_temporary` with the time stamp its names carry, so that a test
/// can take a name before it.
fn create_temporary_stamped(path: &Path, stamp: u128) -> io::Result<(File, PathBuf)> {
    if path.file_name().is_none() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "not a file path",
        ));
    }
    for attempt in 0..100 {
        let temporary = temporary_name(path, stamp, attempt);
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

/// The hidden file next to `path` that attempt `attempt` writes to.
fn temporary_name(path: &Path, stamp: u128, attempt: usize) -> PathBuf {
    let mut name = OsString::from(".");
    name.push(path.file_name().unwrap_or_default());
    name.push(format!(
        ".scry-{}-{stamp}-{attempt}.tmp",
        std::process::id()
    ));
    path.with_file_name(name)
}

/// Whether the file has hard links besides this name.
#[cfg(unix)]
fn has_other_names(path: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    fs::metadata(path).is_ok_and(|metadata| metadata.nlink() > 1)
}

#[cfg(not(unix))]
fn has_other_names(_path: &Path) -> bool {
    false
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
    /// Git did not merge the file (`-merge` or `binary` in its
    /// attributes): it holds ours, with no marker, and nothing is decided.
    NotMerged(PathBuf),
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
            LoadError::Malformed(path, error) => {
                let shown = path.display().to_string();
                let argument = if shown
                    .chars()
                    .all(|c| c.is_alphanumeric() || "/._-+@".contains(c))
                {
                    shown.clone()
                } else {
                    format!("'{}'", shown.replace('\'', "'\\''"))
                };
                write!(
                    f,
                    "{shown}: {error}; fix the markers, or run `git checkout --merge -- {argument}` at the repository's root to recreate the conflicts (this drops edits made to the file)"
                )
            }
            LoadError::NotMerged(path) => write!(
                f,
                "git did not merge {} (its merge attribute) and left ours in it; pick a version in Scry's overview (`git scry`), or with `git checkout --ours` or `--theirs`",
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
        // An index from JSON can be anything: refused, never an overflow.
        assert!(session.edit(usize::MAX, "x").is_err());
        assert!(session.choose(usize::MAX, CandidateKind::Ours).is_err());
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

    /// Every line typed into a CRLF file gets `\r\n`, not only the last
    /// one, even when the conflict's sides give no hint (ours empty) or a
    /// paste mixes the two: a mixed file shows as changed on every line
    /// in Windows tools and diffs.
    #[test]
    fn edit_writes_the_files_line_break_on_every_line() {
        let text = "a\r\n<<<<<<< ours\r\n=======\r\nt\r\n>>>>>>> theirs\r\nz\r\n";
        let mut session = MergeSession::from_text(text, None).unwrap();
        session.edit(0, "x\ny\r\nw").unwrap();
        assert_eq!(
            session.merged_text().as_deref(),
            Some("a\r\nx\r\ny\r\nw\r\nz\r\n")
        );

        let unix = "a\n<<<<<<< ours\n=======\nt\n>>>>>>> theirs\n";
        let mut session = MergeSession::from_text(unix, None).unwrap();
        session.edit(0, "x\r\ny").unwrap();
        assert_eq!(session.chosen_text(0), Some("x\ny\n"));
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

    /// A draft for a later conflict is painted where it will land, earlier conflicts
    /// chosen with another length moving it: its spans are those the result pane shows
    /// once it is applied, whether or not it ends with a line break. An empty draft, or
    /// one for a conflict that does not exist, has nothing to paint.
    #[test]
    fn draft_spans_match_the_applied_edit() {
        let text = "let a = 1\n\
            <<<<<<< ours\nlet b = 2\nlet c = 3\n||||||| base\nlet b = 0\n=======\nlet b = \"x\"\n>>>>>>> theirs\n\
            func f() {\n}\n\
            <<<<<<< ours\nlet d = 4\n||||||| base\nlet d = 0\n=======\nlet d = 5\n>>>>>>> theirs\n\
            let z = \"end\"\n";
        let session = || {
            let mut session = MergeSession::from_text(text, Some(Language::Swift)).unwrap();
            // One line instead of ours' two: the second conflict moves up.
            session.choose(0, CandidateKind::Theirs).unwrap();
            session
        };
        for draft in ["let s = \"q\" // note", "let s = 1\nfunc g() {}\n"] {
            let spans = session().highlight_draft(1, draft);
            let mut applied = session();
            let mut edit = draft.to_string();
            if !edit.ends_with('\n') {
                edit.push('\n');
            }
            applied
                .apply(Command::Edit {
                    conflict: 1,
                    text: edit,
                })
                .unwrap();
            // The second conflict starts on row 4, after `let a`, `let b`, `func f` and `}`.
            let rows = draft.lines().count();
            assert_eq!(spans, applied.result_spans()[4..4 + rows], "{draft:?}");
        }
        assert!(session().highlight_draft(1, "").is_empty());
        assert!(session().highlight_draft(2, "x").is_empty());
    }

    /// The status counts the conflicts resolved so far out of all of them.
    #[test]
    fn status_counts_resolved_conflicts() {
        let text = "<<<<<<< ours\na\n=======\nb\n>>>>>>> theirs\nc\n<<<<<<< ours\nd\n=======\ne\n>>>>>>> theirs\n";
        let mut session = MergeSession::from_text(text, None).unwrap();
        assert_eq!((session.status().resolved, session.status().total), (0, 2));
        session.choose(1, CandidateKind::Theirs).unwrap();
        assert_eq!((session.status().resolved, session.status().total), (1, 2));
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

    /// Typing in a later conflict indents for the file as it will be before it: earlier
    /// conflicts as chosen, so a block one of them opens counts, and only what comes
    /// before this conflict.
    #[test]
    fn indent_in_a_later_conflict_sees_the_earlier_ones() {
        let text = "func f() {\n\
            <<<<<<< ours\n    if a {\n=======\n    if b {\n>>>>>>> theirs\n\
            \x20       mid()\n\
            <<<<<<< ours\n        c()\n=======\n        d()\n>>>>>>> theirs\n\
            \x20   }\n}\n";
        let mut session = MergeSession::from_text(text, Some(Language::Swift)).unwrap();
        session.choose(0, CandidateKind::Theirs).unwrap();
        // The brace closes the `if` the first conflict's choice opened, wherever the
        // cursor's line stood.
        for before in ["        c()\n        ", "        c()\n            "] {
            assert_eq!(
                session.typed_indent(1, before, '}').as_deref(),
                Some("    "),
                "{before:?}"
            );
        }
    }

    /// A last conflict whose sides have the same text but not the same final line break
    /// offers ours and theirs apart, so taking theirs keeps theirs' break as `git merge-file
    /// --theirs` does; with the same break, theirs stays one card with ours.
    #[test]
    fn sides_differing_only_by_the_final_break_are_offered_apart() {
        let text = "a\n<<<<<<< ours\nx y\n=======\nx y\n>>>>>>> theirs\n";
        let session = |ours, theirs| {
            MergeSession::from_text(text, None)
                .unwrap()
                .with_final_breaks(ours, theirs)
        };
        for (ours, theirs) in [(false, true), (true, false)] {
            let kinds: Vec<CandidateKind> = session(ours, theirs)
                .analysis(0)
                .candidates
                .iter()
                .map(|candidate| candidate.kind)
                .collect();
            assert_eq!(&kinds[..2], [CandidateKind::Ours, CandidateKind::Theirs]);
            for (kind, break_kept) in [(CandidateKind::Ours, ours), (CandidateKind::Theirs, theirs)]
            {
                let mut session = session(ours, theirs);
                session.choose(0, kind).unwrap();
                let expected = if break_kept { "a\nx y\n" } else { "a\nx y" };
                assert_eq!(session.merged_text().as_deref(), Some(expected), "{kind:?}");
            }
        }
        assert_eq!(
            session(true, true).analysis(0).candidates[0].aliases,
            [CandidateKind::Theirs]
        );
    }

    /// When the base has that same text too, the card ours keeps answers
    /// for it; the theirs card split off answers for theirs alone, so no
    /// choice is offered by two cards.
    #[test]
    fn a_split_theirs_card_answers_for_theirs_alone() {
        let text = "a\n<<<<<<< ours\nx y\n||||||| base\nx y\n=======\nx y\n>>>>>>> theirs\n";
        let session = MergeSession::from_text(text, None)
            .unwrap()
            .with_final_breaks(false, true);
        let answers: Vec<(CandidateKind, Vec<CandidateKind>)> = session
            .analysis(0)
            .candidates
            .iter()
            .map(|candidate| (candidate.kind, candidate.aliases.clone()))
            .collect();
        assert_eq!(
            answers[..2],
            [
                (CandidateKind::Ours, vec![CandidateKind::Base]),
                (CandidateKind::Theirs, vec![])
            ]
        );
    }

    /// The file's own line break is the one most of its lines use; a tie, or a file
    /// without any, takes `\n`.
    #[test]
    fn the_line_break_is_the_most_used_one() {
        assert_eq!(line_break("a\r\nb\r\nc\n"), "\r\n");
        assert_eq!(line_break("a\r\nb\n"), "\n");
        assert_eq!(line_break("a\nb\n"), "\n");
        assert_eq!(line_break("a"), "\n");
    }

    /// A file that ends with common text keeps its final line break whatever the sides' last
    /// lines were: those only decide when a conflict ends the file.
    #[test]
    fn the_final_break_follows_the_sides_only_after_a_last_conflict() {
        let text = "<<<<<<< ours\na\n=======\nb\n>>>>>>> theirs\ntail\n";
        let mut session = MergeSession::from_text(text, None)
            .unwrap()
            .with_final_breaks(false, false);
        assert!(!session.ends_with_conflict());
        session.choose(0, CandidateKind::Theirs).unwrap();
        assert_eq!(session.merged_text().as_deref(), Some("b\ntail\n"));
    }

    /// A last conflict resolved with no text (neither side, an empty hand edit) adds no
    /// line, so the line break that ends the file is the one before it, and stays: taking
    /// neither side must not join the common text's last line with nothing.
    #[test]
    fn an_empty_last_choice_keeps_the_break_before_it() {
        let text = "a\n<<<<<<< ours\nx\n=======\ny\n>>>>>>> theirs\n";
        let mut session = MergeSession::from_text(text, None)
            .unwrap()
            .with_final_breaks(false, true);
        session.choose(0, CandidateKind::Neither).unwrap();
        assert_eq!(session.merged_text().as_deref(), Some("a\n"));
    }

    /// Checks need the file to parse both with every conflict as ours and with every
    /// conflict as theirs: theirs not parsing turns them off, as ours not parsing does.
    #[test]
    fn checks_are_off_when_one_side_does_not_parse() {
        let theirs_broken =
            "func a() {\n<<<<<<< ours\n    x()\n=======\n    y(\n>>>>>>> theirs\n}\n";
        let ours_broken = "func a() {\n<<<<<<< ours\n    x(\n=======\n    y()\n>>>>>>> theirs\n}\n";
        for text in [theirs_broken, ours_broken] {
            let session = MergeSession::from_text(text, Some(Language::Swift)).unwrap();
            assert!(session.status().checks_off, "{text}");
        }
        let both_parse = "func a() {\n<<<<<<< ours\n    x()\n=======\n    y()\n>>>>>>> theirs\n}\n";
        let session = MergeSession::from_text(both_parse, Some(Language::Swift)).unwrap();
        assert!(!session.status().checks_off);
    }

    /// The closed variants of "both" are offered for any conflict, not only the first: the
    /// lines after the second conflict close it the same way.
    #[test]
    fn closed_both_is_offered_for_a_later_conflict() {
        let text = format!(
            "let a = 1\n<<<<<<< ours\nlet x = 1\n||||||| base\nlet x = 0\n=======\nlet x = 2\n>>>>>>> theirs\n{SWIFT}"
        );
        let session = MergeSession::from_text(&text, Some(Language::Swift)).unwrap();
        assert!(
            session
                .analysis(1)
                .candidates
                .iter()
                .any(|c| c.kind == CandidateKind::OursThenTheirsClosed)
        );
    }

    /// Errors read as sentences that name what went wrong: the CLI and the app show them
    /// as they are.
    #[test]
    fn errors_say_what_went_wrong() {
        assert_eq!(ChooseError::NoSuchConflict(3).to_string(), "no conflict 3");
        assert_eq!(
            ChooseError::NotOffered {
                conflict: String::from("ab12"),
                candidate: "ours-then-theirs-closed",
            }
            .to_string(),
            "conflict ab12 has no candidate \"ours-then-theirs-closed\""
        );
        assert_eq!(SaveError::Unresolved(2).to_string(), "2 conflict(s) left");
        assert_eq!(
            SaveError::ChangedOnDisk(PathBuf::from("a.txt")).to_string(),
            "a.txt changed on disk since Scry read it"
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

    /// The lines around a conflict are the common text next to it, cut to the number asked
    /// and never reaching into another conflict: an agent reads them as they are in the file.
    #[test]
    fn context_is_the_common_text_next_to_a_conflict() {
        let text = "one\ntwo\nthree\n<<<<<<< ours\na\n=======\nb\n>>>>>>> theirs\nfour\n\
            <<<<<<< ours\nc\n=======\nd\n>>>>>>> theirs\n<<<<<<< ours\ne\n=======\nf\n>>>>>>> theirs\n";
        let session = MergeSession::from_text(text, None).unwrap();
        assert_eq!(
            session.context(0, 2),
            (String::from("two\nthree\n"), String::from("four\n"))
        );
        assert_eq!(session.context(0, 0), (String::new(), String::new()));
        assert_eq!(
            session.context(1, 5),
            (String::from("four\n"), String::new())
        );
        // Two conflicts in a row: nothing between them, nothing after the last.
        assert_eq!(session.context(2, 3), (String::new(), String::new()));
        assert_eq!(session.context(3, 3), (String::new(), String::new()));
    }

    /// A kind whose text another candidate already gives chooses that candidate: with the
    /// same change on both sides, "theirs" takes the one card shown, as ours, so "take
    /// theirs" works on every conflict. A kind the conflict cannot give at all, or a
    /// conflict that does not exist, is an error rather than a silent no-op.
    #[test]
    fn a_kind_with_the_same_text_chooses_that_candidate() {
        let mut session = MergeSession::from_text(TEXT, None).unwrap();
        // Conflict 2 has the same text on both sides: one card, "ours".
        assert_eq!(
            session.analysis(1).candidates[0].aliases,
            [CandidateKind::Theirs]
        );
        session.choose(1, CandidateKind::Theirs).unwrap();
        assert_eq!(
            session.choice(1),
            Some(&Choice::Candidate(CandidateKind::Ours))
        );
        assert_eq!(
            session.choose(1, CandidateKind::OursThenTheirsClosed),
            Err(ChooseError::NotOffered {
                conflict: session.analysis(1).id.clone(),
                candidate: "ours-then-theirs-closed",
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

    /// A file with another hard link keeps it: both names show the
    /// result, rather than the other name keeping the conflict markers.
    #[cfg(unix)]
    #[test]
    fn atomic_write_keeps_a_hard_link() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.txt");
        let other = dir.path().join("b.txt");
        fs::write(&path, "old\n").unwrap();
        fs::hard_link(&path, &other).unwrap();

        write_atomically(&path, "new\n").unwrap();

        assert_eq!(fs::read_to_string(&other).unwrap(), "new\n");
        assert_eq!(fs::read_to_string(&path).unwrap(), "new\n");
    }

    /// A file with a single name is replaced by a new one renamed over it, not rewritten in
    /// place: a crash or a full disk mid-write then leaves the old file whole. No temporary
    /// file is left behind.
    #[cfg(unix)]
    #[test]
    fn atomic_write_replaces_a_single_named_file() {
        use std::os::unix::fs::MetadataExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.txt");
        fs::write(&path, "old\n").unwrap();
        let before = fs::metadata(&path).unwrap().ino();

        write_atomically(&path, "new\n").unwrap();

        assert_ne!(fs::metadata(&path).unwrap().ino(), before);
        assert_eq!(fs::read_to_string(&path).unwrap(), "new\n");
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    /// A temporary name already taken (another Scry, a file left by a crash) is skipped and
    /// left as it is; another error, such as a missing folder, is reported at once rather
    /// than retried.
    #[test]
    fn a_taken_temporary_name_is_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.txt");
        let taken = temporary_name(&path, 7, 0);
        fs::write(&taken, "someone else's\n").unwrap();

        let (_, temporary) = create_temporary_stamped(&path, 7).unwrap();

        assert_eq!(temporary, temporary_name(&path, 7, 1));
        assert_eq!(fs::read_to_string(&taken).unwrap(), "someone else's\n");
        let missing = dir.path().join("missing").join("a.txt");
        let error = create_temporary_stamped(&missing, 7).err().unwrap();
        assert_eq!(error.kind(), io::ErrorKind::NotFound);
    }

    /// A file that cannot be read back before saving (here a folder in its place) is an
    /// error to report, not a change on disk the user could overwrite.
    #[test]
    fn a_file_that_cannot_be_read_back_is_not_saved() {
        let dir = tempfile::tempdir().unwrap();
        let mut session = MergeSession::from_text(SWIFT, None)
            .unwrap()
            .with_file(dir.path().to_path_buf());
        session.choose(0, CandidateKind::Ours).unwrap();
        assert!(matches!(session.save(false), Err(SaveError::Io(..))));
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

    /// The result pane is repainted only where a choice changes it; after any sequence of
    /// choices, undos and hand edits, the part repainted must succeed and its spans must be
    /// what painting the whole result gives, including lines after a conflict whose text
    /// opens a string.
    #[test]
    fn the_result_repainted_in_part_matches_a_full_paint() {
        let text = "let a = 1\n\
            <<<<<<< ours\nlet b = 2\nlet c = 3\n||||||| base\nlet b = 0\n=======\nlet b = \"x\"\n>>>>>>> theirs\n\
            func f() {\n    print(a)\n}\n\
            <<<<<<< ours\nlet d = 4\n||||||| base\nlet d = 0\n=======\nlet d = 5\nlet e = 6\nlet f = 7\n>>>>>>> theirs\n\
            let z = \"end\"\n";
        let steps = [
            Command::Choose {
                conflict: 1,
                candidate: CandidateKind::Theirs,
            },
            Command::Choose {
                conflict: 0,
                candidate: CandidateKind::Ours,
            },
            Command::Choose {
                conflict: 0,
                candidate: CandidateKind::Theirs,
            },
            Command::Choose {
                conflict: 1,
                candidate: CandidateKind::Neither,
            },
            Command::Edit {
                conflict: 0,
                text: String::from("let s = \"open\nlet t = 1\n"),
            },
            Command::Undo,
            Command::Choose {
                conflict: 1,
                candidate: CandidateKind::OursThenTheirs,
            },
        ];
        assert_repaints_in_part(text, &steps);
    }

    /// Applying the suggestions changes several conflicts in one step: each later one is
    /// repainted where the earlier ones, now longer or shorter, moved it.
    #[test]
    fn the_result_repainted_after_several_changes_matches_a_full_paint() {
        let text = "let a = 1\n\
            <<<<<<< ours\nlet b = 0\n||||||| base\nlet b = 0\n=======\nlet b = 1\nlet c = 2\nlet d = 3\n>>>>>>> theirs\n\
            func f() {\n}\n\
            <<<<<<< ours\nlet e = 0\n||||||| base\nlet e = 0\n=======\nlet e = \"x\"\n>>>>>>> theirs\n\
            let z = 9\n\
            <<<<<<< ours\nlet g = 0\n||||||| base\nlet g = 0\n=======\nlet g = 1\nlet h = 2\n>>>>>>> theirs\n\
            let y = \"end\"\n";
        let steps = [
            Command::ApplySuggestions,
            Command::Undo,
            Command::Redo,
            Command::Choose {
                conflict: 0,
                candidate: CandidateKind::Ours,
            },
            Command::Undo,
        ];
        assert_repaints_in_part(text, &steps);
    }

    /// Runs `steps` on a Swift session of `text`, checking after each that the result pane
    /// repainted in part succeeds and matches a full paint.
    fn assert_repaints_in_part(text: &str, steps: &[Command]) {
        let mut session = MergeSession::from_text(text, Some(Language::Swift)).unwrap();
        let full = |session: &MergeSession| {
            let texts: Vec<&str> = (0..session.conflict_count())
                .map(|index| session.chosen_text(index).unwrap_or(""))
                .collect();
            session.highlight_variant(&texts)
        };
        assert_eq!(session.result_spans(), full(&session));
        for step in steps {
            // The paint before the step, to repaint from directly: through
            // `result_spans` a failed repaint falls back to a full paint
            // unseen, and the pane would be repainted whole on every choice.
            let last = {
                let paint = session.result_paint.lock().unwrap();
                let last = paint.as_ref().unwrap();
                ResultPaint {
                    texts: last.texts.clone(),
                    parsed: last.parsed.clone(),
                    spans: last.spans.clone(),
                }
            };
            session.apply(step.clone()).unwrap();
            let texts: Vec<&str> = (0..session.conflict_count())
                .map(|index| session.chosen_text(index).unwrap_or(""))
                .collect();
            let highlighter = session.highlighter.as_ref().unwrap();
            let repainted = session.repaint(highlighter, &last, &texts);
            assert_eq!(
                repainted.map(|paint| paint.spans),
                Some(full(&session)),
                "repainted in part after {step:?}"
            );
            assert_eq!(session.result_spans(), full(&session), "after {step:?}");
        }
    }

    /// An Objective-C header the grammar cannot read (Apple's nullability macros) is not
    /// checked, and the status says so: silence would read as a pass. A file the grammar
    /// reads, or one without a language, has the checks on or none to speak of.
    #[test]
    fn a_file_the_grammar_cannot_read_says_checks_are_off() {
        let header = "#import <Foundation/Foundation.h>\n\nNS_ASSUME_NONNULL_BEGIN\n\n\
            @interface Uploader : NSObject\n<<<<<<< ours\n- (void)cancel;\n||||||| base\n=======\n\
            - (void)pause;\n>>>>>>> theirs\n@end\n\nNS_ASSUME_NONNULL_END\n";
        let session = MergeSession::from_text(header, Some(Language::ObjectiveC)).unwrap();
        assert!(session.status().checks_off);
        assert!(
            !MergeSession::from_text(SWIFT, Some(Language::Swift))
                .unwrap()
                .status()
                .checks_off
        );
        assert!(
            !MergeSession::from_text(TEXT, None)
                .unwrap()
                .status()
                .checks_off
        );
    }

    /// Candidates are checked the first time a conflict is asked for, not on opening:
    /// checking every conflict of a large catalog up front took seconds. A closed "both"
    /// can be chosen before anything asked, and the check is kept.
    #[test]
    fn candidates_are_checked_when_first_asked_for() {
        let mut session = MergeSession::from_text(SWIFT, Some(Language::Swift)).unwrap();
        assert!(!session.is_checked(0));
        assert!(
            session
                .analysis_so_far(0)
                .candidates
                .iter()
                .all(|c| c.parse == ParseCheck::NotChecked)
        );
        session
            .choose(0, CandidateKind::OursThenTheirsClosed)
            .unwrap();
        assert!(session.is_checked(0));
        assert_eq!(session.merged_check(), ParseCheck::Parses);

        let session = MergeSession::from_text(SWIFT, Some(Language::Swift)).unwrap();
        session.check_all();
        assert!(session.is_checked(0));
        assert_eq!(
            session.analysis_so_far(0).candidates.len(),
            session.analysis(0).candidates.len()
        );
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
