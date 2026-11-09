//! The Scry core exposed to Swift through UniFFI.
//!
//! The macOS app only displays what this returns and calls its commands
//! (portability rule 1). Types here are plain records converted from the
//! core's, so the core stays free of UniFFI.

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use scry_core::git::GitCli;
use scry_core::highlight;
use scry_core::layout;
use scry_core::operation::{self, SideRole};
use scry_core::resolution::CandidateKind;
use scry_core::session::{self, MergeSession};
use scry_core::syntax::{Language, ParseCheck};

uniffi::setup_scaffolding!();

#[derive(Debug, uniffi::Error)]
pub enum ScryError {
    Failed { message: String },
}

impl fmt::Display for ScryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ScryError::Failed { message } => write!(f, "{message}"),
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

#[derive(Debug, Clone, uniffi::Record)]
pub struct Cell {
    pub number: Option<u32>,
    pub text: String,
    pub kind: CellKind,
    pub spans: Vec<Span>,
}

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
    /// Picked by a rule because no judgement is needed.
    pub suggested: bool,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct ConflictInfo {
    pub id: String,
    pub number: u32,
    pub start_line: u32,
    pub classification: String,
    pub candidates: Vec<CandidateInfo>,
    /// The chosen candidate's ID.
    pub chosen: Option<String>,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct Status {
    pub resolved: u32,
    pub total: u32,
    /// Of the merged file, once every conflict has a choice.
    pub merged: ParseStatus,
    pub language: Option<String>,
}

/// One conflicted file open in the app.
#[derive(uniffi::Object)]
pub struct MergeDocument {
    path: PathBuf,
    header: Header,
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
        let session = MergeSession::from_text(&text, language).map_err(failed)?;
        let header = header(&path, &session, language);
        Ok(Arc::new(MergeDocument {
            path,
            header,
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

    /// Chooses a candidate, by ID, for the conflict at `conflict` (from 0).
    pub fn choose(&self, conflict: u32, candidate: String) -> Result<(), ScryError> {
        let kind = CandidateKind::from_id(&candidate)
            .ok_or_else(|| failed(format!("unknown candidate {candidate:?}")))?;
        self.lock().choose(conflict as usize, kind).map_err(failed)
    }

    pub fn status(&self) -> Status {
        let session = self.lock();
        Status {
            resolved: (session.conflict_count() - session.unresolved()) as u32,
            total: session.conflict_count() as u32,
            merged: parse_status(session.merged_check()),
            language: session
                .language()
                .map(|language| language.name().to_string()),
        }
    }

    /// Writes the merged file in place. Fails while a conflict is left; the
    /// app asks first when the result does not parse.
    pub fn save(&self) -> Result<(), ScryError> {
        let session = self.lock();
        let text = session
            .merged_text()
            .ok_or_else(|| failed(format!("{} conflict(s) left", session.unresolved())))?;
        session::write_atomically(&self.path, &text).map_err(failed)
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
        let analysis = session.analysis(index);
        let ours = &self.header.ours.label;
        let theirs = &self.header.theirs.label;
        let candidates = analysis
            .candidates
            .iter()
            .map(|candidate| {
                let (title, subtitle) = describe(candidate.kind, ours, theirs);
                let lines: Vec<String> = candidate.text.lines().map(String::from).collect();
                CandidateInfo {
                    id: candidate.kind.id().to_string(),
                    title,
                    subtitle,
                    preview: lines.iter().take(3).cloned().collect(),
                    truncated: lines.len() > 3,
                    parse: parse_status(candidate.parse),
                    suggested: analysis.suggestion == Some(candidate.kind),
                }
            })
            .collect();
        ConflictInfo {
            id: analysis.id.clone(),
            number: analysis.number as u32,
            start_line: analysis.start_line as u32,
            classification: analysis.kind.describe(ours, theirs),
            candidates,
            chosen: session.choice(index).map(|kind| kind.id().to_string()),
        }
    }
}

fn header(path: &Path, session: &MergeSession, language: Option<Language>) -> Header {
    let directory = match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.to_path_buf(),
        _ => PathBuf::from("."),
    };
    let git = GitCli::new(directory);
    let (summary, ours, theirs) = match operation::current(&git) {
        Ok(Some(operation)) => {
            let ours = pane_side(&operation.ours, "ours");
            let theirs = match &operation.theirs {
                Some(side) => pane_side(side, "theirs"),
                None => marker_side(session, false),
            };
            (Some(operation.summary()), ours, theirs)
        }
        _ => (
            None,
            marker_side(session, true),
            marker_side(session, false),
        ),
    };
    Header {
        file_name: path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default(),
        path: path.display().to_string(),
        summary,
        ours,
        theirs,
        language: language.map(|language| language.name().to_string()),
    }
}

fn pane_side(side: &operation::Side, git_name: &str) -> PaneSide {
    let git_term = format!("git calls this side \u{201c}{git_name}\u{201d}");
    let detail = match side.role {
        SideRole::CommitBeingRebased | SideRole::CherryPickedCommit | SideRole::RevertedCommit => {
            git_term
        }
        _ => format!("{} · {git_term}", side.commit.short_id),
    };
    PaneSide {
        label: side.label(),
        detail,
    }
}

/// Without an operation: the labels git wrote on the first conflict's
/// markers.
fn marker_side(session: &MergeSession, ours: bool) -> PaneSide {
    let (label, git_name) = if session.conflict_count() == 0 {
        (String::new(), if ours { "ours" } else { "theirs" })
    } else if ours {
        (session.conflict(0).ours.label.clone(), "ours")
    } else {
        (session.conflict(0).theirs.label.clone(), "theirs")
    };
    PaneSide {
        label: if label.is_empty() {
            git_name.to_string()
        } else {
            label
        },
        detail: format!("git calls this side \u{201c}{git_name}\u{201d}"),
    }
}

fn describe(kind: CandidateKind, ours: &str, theirs: &str) -> (String, String) {
    let (title, subtitle) = match kind {
        CandidateKind::Ours => ("Ours", ours),
        CandidateKind::Theirs => ("Theirs", theirs),
        CandidateKind::OursThenTheirs => ("Both · ours first", "Ours, then theirs"),
        CandidateKind::TheirsThenOurs => ("Both · theirs first", "Theirs, then ours"),
        CandidateKind::Neither => ("None", "Remove the block"),
        CandidateKind::Base => ("Base", "As before both changes"),
        CandidateKind::OursThenTheirsClosed => (
            "Both · ours first, each closed",
            "Repeats the lines git moved below the conflict",
        ),
        CandidateKind::TheirsThenOursClosed => (
            "Both · theirs first, each closed",
            "Repeats the lines git moved below the conflict",
        ),
    };
    (title.to_string(), subtitle.to_string())
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

    /// The Swift-facing API end to end: open a file, read its header and candidates, choose,
    /// refuse saving while unresolved and save once resolved.
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
        assert!(document.save().is_err());

        let conflicts = document.conflicts();
        assert_eq!(conflicts[0].candidates[1].id, "theirs");
        assert!(matches!(
            conflicts[0].candidates[1].parse,
            ParseStatus::Parses
        ));
        document.choose(0, String::from("theirs")).unwrap();
        assert_eq!(document.status().resolved, 1);
        assert!(document.choose(0, String::from("nonsense")).is_err());
        document.save().unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "let a = 2\n");
    }
}
