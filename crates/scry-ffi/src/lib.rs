//! The Scry core exposed to Swift through UniFFI.
//!
//! The macOS app only displays what this returns and calls its commands
//! (portability rule 1). Types here are plain records converted from the
//! core's, so the core stays free of UniFFI.

use std::fmt;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};

use scry_core::blame;
use scry_core::command::{self as core, CommandError};
use scry_core::git::GitCli;
use scry_core::highlight;
use scry_core::layout;
use scry_core::operation;
use scry_core::presentation;
use scry_core::resolution::CandidateKind;
use scry_core::session::{self, MergeSession};
use scry_core::syntax::{Language, ParseCheck};
use scry_core::words;

uniffi::setup_scaffolding!();

#[derive(Debug, uniffi::Error)]
pub enum ScryError {
    Failed {
        message: String,
    },
    /// The file changed on disk since it was opened; saving again with
    /// `overwrite` writes over it.
    ChangedOnDisk {
        message: String,
    },
}

impl fmt::Display for ScryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ScryError::Failed { message } | ScryError::ChangedOnDisk { message } => {
                write!(f, "{message}")
            }
        }
    }
}

impl std::error::Error for ScryError {}

fn failed(message: impl fmt::Display) -> ScryError {
    ScryError::Failed {
        message: message.to_string(),
    }
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct PaneSide {
    /// "main + 1 rebased commit · rebase target".
    pub label: String,
    /// One line of context under the label.
    pub detail: String,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct Header {
    pub file_name: String,
    pub path: String,
    /// The operation in plain words, when git has one in progress.
    pub summary: Option<String>,
    pub ours: PaneSide,
    pub theirs: PaneSide,
    pub language: Option<String>,
}

#[derive(Debug, Clone, uniffi::Enum)]
pub enum CellKind {
    Common,
    Conflict,
    Spacer,
    Unresolved,
    Chosen,
    Removed,
}

/// The design's syntax colour roles; text without a span is plain.
#[derive(Debug, Clone, Copy, uniffi::Enum)]
pub enum SyntaxRole {
    Keyword,
    String,
    Number,
    Comment,
    Type,
    Function,
    Variable,
    Constant,
    Operator,
    Punctuation,
    Attribute,
}

/// A run of one role in a cell's text, in UTF-8 bytes from its start, end
/// excluded.
#[derive(Debug, Clone, uniffi::Record)]
pub struct Span {
    pub start: u32,
    pub end: u32,
    pub role: SyntaxRole,
}

/// A part of a cell's text, in UTF-8 bytes from its start, end excluded.
#[derive(Debug, Clone, uniffi::Record)]
pub struct ByteRange {
    pub start: u32,
    pub end: u32,
}

/// How a side's line in a conflict differs from the base.
#[derive(Debug, Clone, uniffi::Enum)]
pub enum LineChange {
    Unchanged,
    Added,
    /// `words` are the changed parts; empty when only whitespace changed or
    /// the base is unknown.
    Edited {
        words: Vec<ByteRange>,
    },
}

/// What Return does while editing by hand: `text` replaces the current
/// line up to the cursor; `cursor` is in UTF-8 bytes from its start.
#[derive(Debug, Clone, uniffi::Record)]
pub struct Newline {
    pub text: String,
    pub cursor: u32,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct Cell {
    pub number: Option<u32>,
    pub text: String,
    pub kind: CellKind,
    pub spans: Vec<Span>,
    pub change: LineChange,
}

// Same as the core's rows: almost every row is a code row.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, uniffi::Enum)]
pub enum Row {
    Lens {
        conflict: u32,
    },
    Code {
        ours: Cell,
        result: Cell,
        theirs: Cell,
        conflict: Option<u32>,
    },
}

#[derive(Debug, Clone, uniffi::Enum)]
pub enum ParseStatus {
    NotChecked,
    Parses,
    DoesNotParse { line: u32 },
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct CandidateInfo {
    /// Stable within its conflict: "ours", "theirs-then-ours-closed"…
    pub id: String,
    pub title: String,
    pub subtitle: String,
    /// At most three lines.
    pub preview: Vec<String>,
    /// Whether the candidate has more lines than the preview.
    pub truncated: bool,
    pub parse: ParseStatus,
    pub provenance: Provenance,
    /// Picked by a rule because no judgement is needed.
    pub suggested: bool,
}

/// Where a resolution comes from, as a role the app maps to a name, a
/// symbol and a colour.
#[derive(Debug, Clone, Copy, uniffi::Enum)]
pub enum Provenance {
    /// A side as git produced it.
    Git,
    /// Made by a format rule.
    Rule,
    /// Written by the user.
    Human,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct ConflictInfo {
    pub id: String,
    pub number: u32,
    pub start_line: u32,
    pub classification: String,
    pub candidates: Vec<CandidateInfo>,
    /// The resolution: one of `candidates`, or the text written by hand
    /// (ID "edited", provenance `Human`).
    pub chosen: Option<CandidateInfo>,
    /// The text editing by hand starts from: the current resolution, or
    /// ours while unresolved.
    pub edit_text: String,
    /// The commit that last touched the conflict's lines on each side, for
    /// the lens row; `None` outside an operation or when git has no answer.
    pub ours_commit: Option<LensCommit>,
    pub theirs_commit: Option<LensCommit>,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct LensCommit {
    pub short_id: String,
    pub subject: String,
    pub author: String,
    /// "today", "2 days ago"…
    pub age: String,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct Status {
    pub resolved: u32,
    pub total: u32,
    /// Of the merged file, once every conflict has a choice.
    pub merged: ParseStatus,
    pub language: Option<String>,
    pub can_undo: bool,
    pub can_redo: bool,
}

/// Something the app asks the core to do; conflicts are counted from 0.
/// The same commands as the core's, which other UIs send as JSON.
#[derive(Debug, Clone, uniffi::Enum)]
pub enum Command {
    /// Resolves a conflict with a candidate, by ID ("ours", "theirs"…).
    Choose {
        conflict: u32,
        candidate: String,
    },
    /// Resolves a conflict with text written by hand.
    Edit {
        conflict: u32,
        text: String,
    },
    ApplySuggestions,
    Undo,
    Redo,
    /// Writes the merged file; with `overwrite`, even over a file changed
    /// on disk since it was opened.
    Save {
        overwrite: bool,
    },
}

/// What a command changed.
#[derive(Debug, Clone, uniffi::Enum)]
pub enum Event {
    /// These conflicts' resolutions changed: rows, conflicts and status
    /// must be read again.
    ChoicesChanged {
        conflicts: Vec<u32>,
    },
    Undone {
        command: Command,
    },
    Redone {
        command: Command,
    },
    HistoryChanged {
        can_undo: bool,
        can_redo: bool,
    },
    Saved,
}

/// One conflicted file open in the app.
#[derive(uniffi::Object)]
pub struct MergeDocument {
    header: Header,
    /// Per conflict; computed once, since the sides never change.
    commits: Vec<blame::ConflictCommits>,
    // UniFFI shares the object with Swift behind an `Arc`, so methods take
    // `&self`; the mutex lets `choose` change the session safely.
    session: Mutex<MergeSession>,
}

#[uniffi::export]
impl MergeDocument {
    /// Opens a file that contains conflict markers. Side labels come from
    /// the operation git has in progress in the file's repository.
    #[uniffi::constructor]
    pub fn open(path: String) -> Result<Arc<MergeDocument>, ScryError> {
        let path = PathBuf::from(path);
        let text = std::fs::read_to_string(&path)
            .map_err(|error| failed(format!("cannot read {}: {error}", path.display())))?;
        let language = Language::from_path(&path);
        let session = MergeSession::from_text(&text, language)
            .map_err(failed)?
            .with_file(path.clone());
        let directory = match path.parent() {
            Some(parent) if !parent.as_os_str().is_empty() => parent.to_path_buf(),
            _ => PathBuf::from("."),
        };
        let git = GitCli::new(directory);
        let operation = operation::current(&git).ok().flatten();
        let header = header(presentation::header(&path, &session, operation.as_ref()));
        let commits = match (&operation, path.file_name()) {
            (Some(operation), Some(name)) => {
                blame::conflict_commits(&git, operation, &name.to_string_lossy(), &session)
            }
            _ => Vec::new(),
        };
        Ok(Arc::new(MergeDocument {
            header,
            commits,
            session: Mutex::new(session),
        }))
    }

    pub fn header(&self) -> Header {
        self.header.clone()
    }

    pub fn rows(&self) -> Vec<Row> {
        self.lock().rows().into_iter().map(row).collect()
    }

    pub fn conflicts(&self) -> Vec<ConflictInfo> {
        let session = self.lock();
        (0..session.conflict_count())
            .map(|index| self.conflict_info(&session, index))
            .collect()
    }

    /// Runs a command: the only way the app changes the merge. Returns
    /// what changed, nothing when the command changes nothing. Saving a
    /// file changed on disk fails with `ChangedOnDisk`; the app asks, then
    /// sends `Save` with `overwrite`.
    pub fn apply(&self, command: Command) -> Result<Vec<Event>, ScryError> {
        let command = core_command(command)?;
        match self.lock().apply(command) {
            Ok(events) => Ok(events.into_iter().map(event).collect()),
            Err(error @ CommandError::Save(session::SaveError::ChangedOnDisk(_))) => {
                Err(ScryError::ChangedOnDisk {
                    message: error.to_string(),
                })
            }
            Err(error) => Err(failed(error)),
        }
    }

    /// What Return does while editing the conflict at `conflict`, with
    /// `before` the draft's text before the cursor and `after` the rest of
    /// its line. `Newline.text` replaces the current line up to the cursor.
    pub fn newline(&self, conflict: u32, before: String, after: String) -> Newline {
        let newline = self.lock().newline(conflict as usize, &before, &after);
        Newline {
            text: newline.text,
            cursor: newline.cursor as u32,
        }
    }

    /// The indentation for the current line when `typed` (one character)
    /// is typed while editing the conflict at `conflict`, with `before` the
    /// draft's text before the cursor; `None` to type it as it is.
    pub fn typed_indent(&self, conflict: u32, before: String, typed: String) -> Option<String> {
        let mut characters = typed.chars();
        let (Some(typed), None) = (characters.next(), characters.next()) else {
            return None;
        };
        self.lock().typed_indent(conflict as usize, &before, typed)
    }

    /// Highlight spans, one list per line, of `text` being typed for the
    /// conflict at `conflict` (from 0), coloured in place in the file.
    pub fn highlight_draft(&self, conflict: u32, text: String) -> Vec<Vec<Span>> {
        self.lock()
            .highlight_draft(conflict as usize, &text)
            .into_iter()
            .map(|line| line.into_iter().map(span).collect())
            .collect()
    }

    pub fn status(&self) -> Status {
        let session = self.lock();
        Status {
            can_undo: session.can_undo(),
            can_redo: session.can_redo(),
            resolved: (session.conflict_count() - session.unresolved()) as u32,
            total: session.conflict_count() as u32,
            merged: parse_status(session.merged_check()),
            language: session
                .language()
                .map(|language| language.name().to_string()),
        }
    }
}

impl MergeDocument {
    fn lock(&self) -> MutexGuard<'_, MergeSession> {
        // A panic while the lock was held leaves a usable session: recover it.
        self.session
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn conflict_info(&self, session: &MergeSession, index: usize) -> ConflictInfo {
        let view = session.conflict_view(index, &self.header.ours.label, &self.header.theirs.label);
        ConflictInfo {
            id: view.id,
            number: view.number as u32,
            start_line: view.start_line as u32,
            classification: view.classification,
            candidates: view.candidates.into_iter().map(candidate_info).collect(),
            chosen: view.chosen.map(candidate_info),
            edit_text: view.edit_text,
            ours_commit: self.lens_commit(index, true),
            theirs_commit: self.lens_commit(index, false),
        }
    }

    fn lens_commit(&self, index: usize, ours: bool) -> Option<LensCommit> {
        let commits = self.commits.get(index)?;
        let commit = if ours { &commits.ours } else { &commits.theirs };
        let commit = commit.as_ref()?;
        Some(LensCommit {
            short_id: commit.short_id.clone(),
            subject: commit.subject.clone(),
            author: commit.author.clone(),
            age: blame::age_from_now(commit.time),
        })
    }
}

fn header(header: presentation::Header) -> Header {
    Header {
        file_name: header.file_name,
        path: header.path,
        summary: header.summary,
        ours: pane_side(header.ours),
        theirs: pane_side(header.theirs),
        language: header.language,
    }
}

fn pane_side(side: presentation::PaneSide) -> PaneSide {
    PaneSide {
        label: side.label,
        detail: side.detail,
    }
}

fn candidate_info(candidate: presentation::CandidateView) -> CandidateInfo {
    CandidateInfo {
        id: candidate.id,
        title: candidate.title,
        subtitle: candidate.subtitle,
        preview: candidate.preview,
        truncated: candidate.truncated,
        parse: parse_status(candidate.parse),
        provenance: match candidate.provenance {
            presentation::Provenance::Git => Provenance::Git,
            presentation::Provenance::Rule => Provenance::Rule,
            presentation::Provenance::Human => Provenance::Human,
        },
        suggested: candidate.suggested,
    }
}

fn core_command(command: Command) -> Result<core::Command, ScryError> {
    Ok(match command {
        Command::Choose {
            conflict,
            candidate,
        } => core::Command::Choose {
            conflict: conflict as usize,
            candidate: CandidateKind::from_id(&candidate)
                .ok_or_else(|| failed(format!("unknown candidate {candidate:?}")))?,
        },
        Command::Edit { conflict, text } => core::Command::Edit {
            conflict: conflict as usize,
            text,
        },
        Command::ApplySuggestions => core::Command::ApplySuggestions,
        Command::Undo => core::Command::Undo,
        Command::Redo => core::Command::Redo,
        Command::Save { overwrite } => core::Command::Save { overwrite },
    })
}

fn command(command: core::Command) -> Command {
    match command {
        core::Command::Choose {
            conflict,
            candidate,
        } => Command::Choose {
            conflict: conflict as u32,
            candidate: candidate.id().to_string(),
        },
        core::Command::Edit { conflict, text } => Command::Edit {
            conflict: conflict as u32,
            text,
        },
        core::Command::ApplySuggestions => Command::ApplySuggestions,
        core::Command::Undo => Command::Undo,
        core::Command::Redo => Command::Redo,
        core::Command::Save { overwrite } => Command::Save { overwrite },
    }
}

fn event(event: core::Event) -> Event {
    match event {
        core::Event::ChoicesChanged { conflicts } => Event::ChoicesChanged {
            conflicts: conflicts.into_iter().map(|index| index as u32).collect(),
        },
        core::Event::Undone { command: undone } => Event::Undone {
            command: command(undone),
        },
        core::Event::Redone { command: redone } => Event::Redone {
            command: command(redone),
        },
        core::Event::HistoryChanged { can_undo, can_redo } => {
            Event::HistoryChanged { can_undo, can_redo }
        }
        core::Event::Saved => Event::Saved,
    }
}

fn parse_status(check: ParseCheck) -> ParseStatus {
    match check {
        ParseCheck::NotChecked => ParseStatus::NotChecked,
        ParseCheck::Parses => ParseStatus::Parses,
        ParseCheck::DoesNotParse { line } => ParseStatus::DoesNotParse { line: line as u32 },
    }
}

fn cell(cell: layout::Cell) -> Cell {
    Cell {
        number: cell.number.map(|number| number as u32),
        text: cell.text,
        kind: match cell.kind {
            layout::CellKind::Common => CellKind::Common,
            layout::CellKind::Conflict => CellKind::Conflict,
            layout::CellKind::Spacer => CellKind::Spacer,
            layout::CellKind::Unresolved => CellKind::Unresolved,
            layout::CellKind::Chosen => CellKind::Chosen,
            layout::CellKind::Removed => CellKind::Removed,
        },
        spans: cell.spans.into_iter().map(span).collect(),
        change: match cell.change {
            words::LineChange::Unchanged => LineChange::Unchanged,
            words::LineChange::Added => LineChange::Added,
            words::LineChange::Edited { words } => LineChange::Edited {
                words: words
                    .into_iter()
                    .map(|range| ByteRange {
                        start: range.start as u32,
                        end: range.end as u32,
                    })
                    .collect(),
            },
        },
    }
}

fn span(span: highlight::Span) -> Span {
    Span {
        start: span.start as u32,
        end: span.end as u32,
        role: match span.role {
            highlight::SyntaxRole::Keyword => SyntaxRole::Keyword,
            highlight::SyntaxRole::String => SyntaxRole::String,
            highlight::SyntaxRole::Number => SyntaxRole::Number,
            highlight::SyntaxRole::Comment => SyntaxRole::Comment,
            highlight::SyntaxRole::Type => SyntaxRole::Type,
            highlight::SyntaxRole::Function => SyntaxRole::Function,
            highlight::SyntaxRole::Variable => SyntaxRole::Variable,
            highlight::SyntaxRole::Constant => SyntaxRole::Constant,
            highlight::SyntaxRole::Operator => SyntaxRole::Operator,
            highlight::SyntaxRole::Punctuation => SyntaxRole::Punctuation,
            highlight::SyntaxRole::Attribute => SyntaxRole::Attribute,
        },
    }
}

fn row(row: layout::Row) -> Row {
    match row {
        layout::Row::Lens { conflict } => Row::Lens {
            conflict: conflict as u32,
        },
        layout::Row::Code {
            ours,
            result,
            theirs,
            conflict,
        } => Row::Code {
            ours: cell(ours),
            result: cell(result),
            theirs: cell(theirs),
            conflict: conflict.map(|index| index as u32),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn choose(document: &MergeDocument, candidate: &str) -> Result<Vec<Event>, ScryError> {
        document.apply(Command::Choose {
            conflict: 0,
            candidate: candidate.to_string(),
        })
    }

    fn save(document: &MergeDocument, overwrite: bool) -> Result<Vec<Event>, ScryError> {
        document.apply(Command::Save { overwrite })
    }

    /// The Swift-facing API end to end: open a file, read its header and candidates, choose,
    /// refuse saving while unresolved, save once resolved, then edit by hand and undo.
    #[test]
    fn open_choose_and_save() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.swift");
        std::fs::write(
            &path,
            "<<<<<<< HEAD\nlet a = 1\n||||||| base\nlet a = 0\n=======\nlet a = 2\n>>>>>>> other\n",
        )
        .unwrap();
        let document = MergeDocument::open(path.display().to_string()).unwrap();
        assert_eq!(document.header().file_name, "a.swift");
        assert_eq!(document.header().ours.label, "HEAD");
        assert_eq!(document.status().total, 1);
        assert!(matches!(
            save(&document, false),
            Err(ScryError::Failed { .. })
        ));

        let conflicts = document.conflicts();
        assert_eq!(conflicts[0].candidates[1].id, "theirs");
        assert!(matches!(
            conflicts[0].candidates[1].parse,
            ParseStatus::Parses
        ));
        choose(&document, "theirs").unwrap();
        assert_eq!(document.status().resolved, 1);
        assert!(choose(&document, "nonsense").is_err());
        save(&document, false).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "let a = 2\n");
        // Saving again is fine: the file is what Scry wrote.
        save(&document, false).unwrap();

        // By hand, then undone back to "theirs".
        document
            .apply(Command::Edit {
                conflict: 0,
                text: String::from("let a = 3"),
            })
            .unwrap();
        let conflict = &document.conflicts()[0];
        let edited = conflict.chosen.as_ref().unwrap();
        assert_eq!(edited.id, "edited");
        assert!(matches!(edited.provenance, Provenance::Human));
        assert!(matches!(edited.parse, ParseStatus::Parses));
        assert_eq!(conflict.edit_text, "let a = 3\n");
        assert!(document.status().can_undo);
        let undone = document.apply(Command::Undo).unwrap();
        assert!(matches!(
            &undone[0],
            Event::Undone {
                command: Command::Edit { .. }
            }
        ));
        let chosen = document.conflicts()[0].chosen.clone().unwrap();
        assert_eq!(chosen.id, "theirs");
        assert!(matches!(chosen.provenance, Provenance::Git));
        assert!(document.status().can_redo);
    }

    /// A file changed on disk after it was opened (resolved in another
    /// editor, an aborted rebase) is not written over without the user's
    /// say: `save` reports it apart, so the app can ask, and `overwrite`
    /// writes anyway.
    #[test]
    fn save_refuses_a_file_changed_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.swift");
        std::fs::write(
            &path,
            "<<<<<<< HEAD\nlet a = 1\n=======\nlet a = 2\n>>>>>>> other\n",
        )
        .unwrap();
        let document = MergeDocument::open(path.display().to_string()).unwrap();
        choose(&document, "theirs").unwrap();

        std::fs::write(&path, "let a = 42\n").unwrap();
        assert!(matches!(
            save(&document, false),
            Err(ScryError::ChangedOnDisk { .. })
        ));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "let a = 42\n");

        save(&document, true).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "let a = 2\n");
    }

    /// The app sends conflict indices it got from the core, but a stale
    /// or wrong one must be refused, never crash the app and lose the
    /// user's choices: commands fail, queries return nothing.
    #[test]
    fn out_of_range_conflicts_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.swift");
        std::fs::write(
            &path,
            "<<<<<<< HEAD\nlet a = 1\n=======\nlet a = 2\n>>>>>>> other\n",
        )
        .unwrap();
        let document = MergeDocument::open(path.display().to_string()).unwrap();

        for command in [
            Command::Choose {
                conflict: 7,
                candidate: String::from("ours"),
            },
            Command::Edit {
                conflict: 7,
                text: String::from("x"),
            },
        ] {
            assert!(matches!(
                document.apply(command),
                Err(ScryError::Failed { .. })
            ));
        }
        assert!(document.highlight_draft(7, String::from("x\n")).is_empty());
        document.newline(7, String::from("{"), String::new());
        document.typed_indent(7, String::from("{\n"), String::from("}"));
        assert!(!document.status().can_undo);
    }
}
