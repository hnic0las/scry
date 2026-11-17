//! The Scry core exposed to Swift through UniFFI.
//!
//! The macOS app only displays what this returns and calls its commands
//! (portability rule 1). Types here are plain records converted from the
//! core's, so the core stays free of UniFFI.

use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use scry_core::blame;
use scry_core::command::{self as core, CommandError};
use scry_core::commit_changes;
use scry_core::git::{GitCli, GitError};
use scry_core::highlight;
use scry_core::layout;
use scry_core::mergetool;
use scry_core::operation;
use scry_core::overview;
use scry_core::presentation;
use scry_core::rebase_plan;
use scry_core::resolution::CandidateKind;
use scry_core::review;
use scry_core::session::{self, MergeSession, MergetoolFiles};
use scry_core::syntax::ParseCheck;
use scry_core::theme;
use scry_core::words;

uniffi::setup_scaffolding!();

mod agent;
mod cli_command;
pub use agent::{AgentSetup, AgentSetupState, agent_setup, register_agent, unregister_agent};
pub use cli_command::{CliCommandState, cli_command_state, install_cli_command};

/// Why a call failed, by kind, each with a sentence the UI shows as it is.
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
    /// The action does not fit the operation as it stands: a file not in
    /// conflict, a merge to skip, files left to stage.
    Refused {
        message: String,
    },
    /// Git ran and reported a failure (a hook, a lock file, a conflict).
    Git {
        message: String,
    },
    /// Git itself cannot be run.
    GitMissing {
        message: String,
    },
    /// The folder is not in a git repository.
    NotARepository {
        message: String,
    },
}

impl ScryError {
    fn message(&self) -> &str {
        match self {
            ScryError::Failed { message }
            | ScryError::ChangedOnDisk { message }
            | ScryError::Refused { message }
            | ScryError::Git { message }
            | ScryError::GitMissing { message }
            | ScryError::NotARepository { message } => message,
        }
    }
}

impl fmt::Display for ScryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message())
    }
}

impl std::error::Error for ScryError {}

impl From<GitError> for ScryError {
    fn from(error: GitError) -> ScryError {
        let message = sentence(&error);
        match error {
            GitError::Spawn(_) => ScryError::GitMissing { message },
            GitError::Failed { .. } | GitError::UnexpectedOutput { .. } => {
                ScryError::Git { message }
            }
        }
    }
}

impl From<overview::OverviewError> for ScryError {
    fn from(error: overview::OverviewError) -> ScryError {
        match error {
            overview::OverviewError::Git(error) => ScryError::from(error),
            overview::OverviewError::Io(error) => failed(error),
            refused => ScryError::Refused {
                message: sentence(&refused),
            },
        }
    }
}

fn failed(message: impl fmt::Display) -> ScryError {
    ScryError::Failed {
        message: sentence(&message),
    }
}

/// The core's messages start in lower case, for the CLI's `git-scry: …`;
/// the app shows them as sentences.
fn sentence(message: &impl fmt::Display) -> String {
    let text = message.to_string();
    let text = text.trim();
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
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
    /// A base line, in the result above its conflict, when the base is
    /// shown.
    Base,
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
    /// The line as shown, tabs expanded.
    pub text: String,
    /// Its width in columns of the monospaced font; wide characters count
    /// 2. The app multiplies it by one column's width.
    pub columns: u32,
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
    /// IDs that choose this candidate too, giving the same text: "theirs"
    /// when both sides made the same change.
    pub aliases: Vec<String>,
}

/// The theme's roles, from the core (portability rule 5): the app maps
/// each to a colour and a symbol. See `scry_core::theme`.
#[derive(Debug, Clone, Copy, uniffi::Enum)]
pub enum Provenance {
    Git,
    Rule,
    Structural,
    Analyzer,
    Agent,
    Human,
}

/// The core's `theme::State`; renamed, since SwiftUI has its own `State`.
#[derive(Debug, Clone, Copy, uniffi::Enum)]
pub enum ConflictState {
    Unresolved,
    NeedsReview,
    Resolved,
    Staged,
}

#[derive(Debug, Clone, Copy, uniffi::Enum)]
pub enum Severity {
    Error,
    Warning,
    Info,
}

#[derive(Debug, Clone, Copy, uniffi::Enum)]
pub enum ThemeRole {
    Provenance { provenance: Provenance },
    State { state: ConflictState },
    Severity { severity: Severity },
}

/// A lens row's label, coloured by its role.
#[derive(Debug, Clone, uniffi::Record)]
pub struct LensBadge {
    pub text: String,
    pub role: ThemeRole,
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
    pub state: ConflictState,
    pub badges: Vec<LensBadge>,
    /// The text editing by hand starts from: the current resolution, or
    /// ours while unresolved.
    pub edit_text: String,
    /// The commit that last touched the conflict's lines on each side, for
    /// the lens row; `None` outside an operation or when git has no answer.
    pub ours_commit: Option<LensCommit>,
    pub theirs_commit: Option<LensCommit>,
    /// git wrote the conflict with its base (diff3 markers, or a merge
    /// tool's base file), so it can be shown.
    pub has_base: bool,
    /// The candidates are checked by parsing (or the file cannot be).
    /// `conflicts` leaves them unchecked until `conflict` asks for one:
    /// checking every conflict of a large file took seconds.
    pub checked: bool,
    /// For a conflict resolved outside Scry (a document from
    /// `MergeDocument::open`): who, why, and whether the user reviewed it.
    pub outside: Option<OutsideInfo>,
}

/// A conflict resolved outside Scry, as the review shows it.
#[derive(Debug, Clone, uniffi::Record)]
pub struct OutsideInfo {
    /// The agent's name and version as it gave them; `None` when it did
    /// not resolve through Scry, so nobody is known.
    pub by: Option<String>,
    pub reason: Option<String>,
    /// What of either side the agent's own text leaves out.
    pub dropped: Option<String>,
    pub reviewed: bool,
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
    /// The language's grammar cannot read this file, so nothing is
    /// checked: the app says so rather than show nothing.
    pub checks_off: bool,
    pub can_undo: bool,
    pub can_redo: bool,
    /// Conflicts resolved outside Scry not reviewed yet.
    pub unreviewed: u32,
    /// The result is not what the file holds: something to write.
    pub unwritten: bool,
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
    /// Marks a conflict resolved outside Scry as reviewed.
    MarkReviewed {
        conflict: u32,
    },
    /// Takes a conflict's choice away: it is open again.
    Reopen {
        conflict: u32,
    },
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
    // `&self`; the mutex lets commands change the session safely.
    session: Mutex<MergeSession>,
}

/// `path` in its repository: the root, and the path from it as git names
/// it; `None` outside a repository.
fn in_repository(path: &Path) -> Option<(PathBuf, String)> {
    let root = overview::root(&GitCli::beside(path)).ok()?;
    let relative = fs::canonicalize(path).ok().and_then(|path| {
        let root = fs::canonicalize(&root).ok()?;
        path.strip_prefix(root).ok().map(Path::to_path_buf)
    })?;
    Some((root, relative.to_str()?.to_string()))
}

#[uniffi::export]
impl MergeDocument {
    /// Opens a conflicted file: from its markers, or, when some of its
    /// conflicts were resolved outside Scry (by an agent, another editor),
    /// to review them, each conflict's choice being what the file holds.
    /// In a repository, the review is kept in its git folder. Side labels
    /// come from the operation git has in progress.
    #[uniffi::constructor]
    pub fn open(path: String) -> Result<Arc<MergeDocument>, ScryError> {
        let path = PathBuf::from(path);
        let git = GitCli::beside(&path);
        let operation = operation::current(&git).ok().flatten();
        let session = match in_repository(&path) {
            Some((root, relative)) => {
                let (ours, theirs) = presentation::side_labels(operation.as_ref());
                review::opened(
                    &GitCli::discover(root.clone()),
                    &git,
                    &root,
                    &relative,
                    (&ours, &theirs),
                )
            }
            None => session::open_file(&git, &path),
        }
        .map_err(failed)?;
        Ok(MergeDocument::new(&path, session, &git, operation))
    }

    /// Opens the files `git mergetool` passes, as `git-scry resolve` does:
    /// a file in the `merge` style, still as git wrote it, is rebuilt from
    /// the three versions so the base shows. Saving writes `merged`.
    #[uniffi::constructor]
    pub fn open_mergetool(
        base: String,
        local: String,
        remote: String,
        merged: String,
    ) -> Result<Arc<MergeDocument>, ScryError> {
        let files = MergetoolFiles {
            base: PathBuf::from(base),
            local: PathBuf::from(local),
            remote: PathBuf::from(remote),
            merged: PathBuf::from(merged),
        };
        let git = GitCli::beside(&files.merged);
        let (session, operation) = session::open_mergetool(&git, &files).map_err(failed)?;
        Ok(MergeDocument::new(&files.merged, session, &git, operation))
    }

    pub fn header(&self) -> Header {
        self.header.clone()
    }

    /// `show_base` adds each conflict's base lines above its block.
    pub fn rows(&self, show_base: bool) -> Vec<Row> {
        self.lock().rows(show_base).into_iter().map(row).collect()
    }

    /// Every conflict, its candidates as far as they are checked.
    pub fn conflicts(&self) -> Vec<ConflictInfo> {
        let session = self.lock();
        (0..session.conflict_count())
            .map(|index| self.conflict_info(&session, index, false))
            .collect()
    }

    /// One conflict, its candidates checked by parsing: for the focused
    /// conflict. `None` out of range.
    pub fn conflict(&self, index: u32) -> Option<ConflictInfo> {
        let session = self.lock();
        let index = index as usize;
        (index < session.conflict_count()).then(|| self.conflict_info(&session, index, true))
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

    /// Whether a save without `overwrite` would be refused because the
    /// file changed on disk: the app asks once, before writing.
    pub fn changed_on_disk(&self) -> bool {
        self.lock().changed_on_disk()
    }

    pub fn status(&self) -> Status {
        let status = self.lock().status();
        Status {
            can_undo: status.can_undo,
            can_redo: status.can_redo,
            resolved: status.resolved as u32,
            total: status.total as u32,
            merged: parse_status(status.merged),
            language: status.language.map(|language| language.name().to_string()),
            checks_off: status.checks_off,
            unreviewed: status.unreviewed as u32,
            unwritten: status.unwritten,
        }
    }
}

impl MergeDocument {
    /// The document on `session`, read from `path`, with its header and
    /// lens commits from the operation in progress.
    fn new(
        path: &Path,
        session: MergeSession,
        git: &GitCli,
        operation: Option<operation::Operation>,
    ) -> Arc<MergeDocument> {
        let header = header(presentation::header(path, &session, operation.as_ref()));
        let commits = match (&operation, path.file_name()) {
            (Some(operation), Some(name)) => {
                blame::conflict_commits(git, operation, &name.to_string_lossy(), &session)
            }
            _ => Vec::new(),
        };
        Arc::new(MergeDocument {
            header,
            commits,
            session: Mutex::new(session),
        })
    }

    fn lock(&self) -> MutexGuard<'_, MergeSession> {
        // A panic while the lock was held leaves a usable session: recover it.
        self.session
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn conflict_info(&self, session: &MergeSession, index: usize, check: bool) -> ConflictInfo {
        let (ours, theirs) = (&self.header.ours.label, &self.header.theirs.label);
        let view = if check {
            session.conflict_view(index, ours, theirs)
        } else {
            session.conflict_view_so_far(index, ours, theirs)
        };
        ConflictInfo {
            id: view.id,
            number: view.number as u32,
            start_line: view.start_line as u32,
            classification: view.classification,
            candidates: view.candidates.into_iter().map(candidate_info).collect(),
            chosen: view.chosen.map(candidate_info),
            state: conflict_state(view.state),
            badges: view.badges.into_iter().map(lens_badge).collect(),
            edit_text: view.edit_text,
            ours_commit: self.lens_commit(index, true),
            theirs_commit: self.lens_commit(index, false),
            has_base: session.conflict(index).base.is_some(),
            checked: view.checked,
            outside: view.outside.map(|outside| OutsideInfo {
                by: outside.by,
                reason: outside.reason,
                dropped: outside.dropped,
                reviewed: outside.reviewed,
            }),
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
        provenance: provenance(candidate.provenance),
        suggested: candidate.suggested,
        aliases: candidate.aliases,
    }
}

fn provenance(provenance: theme::Provenance) -> Provenance {
    match provenance {
        theme::Provenance::Git => Provenance::Git,
        theme::Provenance::Rule => Provenance::Rule,
        theme::Provenance::Structural => Provenance::Structural,
        theme::Provenance::Analyzer => Provenance::Analyzer,
        theme::Provenance::Agent => Provenance::Agent,
        theme::Provenance::Human => Provenance::Human,
    }
}

fn conflict_state(state: theme::State) -> ConflictState {
    match state {
        theme::State::Unresolved => ConflictState::Unresolved,
        theme::State::NeedsReview => ConflictState::NeedsReview,
        theme::State::Resolved => ConflictState::Resolved,
        theme::State::Staged => ConflictState::Staged,
    }
}

fn lens_badge(badge: theme::Badge) -> LensBadge {
    let role = match badge.role {
        theme::Role::Provenance(role) => ThemeRole::Provenance {
            provenance: provenance(role),
        },
        theme::Role::State(state) => ThemeRole::State {
            state: conflict_state(state),
        },
        theme::Role::Severity(severity) => ThemeRole::Severity {
            severity: match severity {
                theme::Severity::Error => Severity::Error,
                theme::Severity::Warning => Severity::Warning,
                theme::Severity::Info => Severity::Info,
            },
        },
    };
    LensBadge {
        text: badge.text,
        role,
    }
}

/// One file of the operation overview.
#[derive(Debug, Clone, uniffi::Record)]
pub struct OverviewFile {
    /// From the repository root.
    pub path: String,
    /// On disk, to open in the merge editor.
    pub absolute_path: String,
    pub name: String,
    /// Empty at the repository root.
    pub directory: String,
    pub kind: String,
    pub state: ConflictState,
    pub conflicts: Option<u32>,
    pub openable: bool,
    /// The view that settles it, when it has one.
    pub special: Option<SpecialKind>,
    /// It opens from the overview, in the editor or its own view.
    pub can_open: bool,
    /// Git can take it as it is.
    pub can_stage: bool,
    /// Its conflict can come back.
    pub can_unstage: bool,
    /// Resolved and not staged: how far the review of its resolutions is.
    pub review: Option<ReviewProgress>,
}

/// How far the review of a resolved file is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Record)]
pub struct ReviewProgress {
    /// Conflicts reviewed, or resolved in Scry.
    pub reviewed: u32,
    pub total: u32,
    /// The text around the conflicts changed.
    pub changed_outside: bool,
}

/// A conflicted file the overview settles with its own view.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum SpecialKind {
    DeletedOnOneSide,
    Binary,
    ProjectFile,
    StringCatalog,
    PackageResolved,
    Lockfile,
    /// `.gitattributes` kept git from merging it: take one version.
    NotMerged,
    /// Renamed on both sides, a submodule, a symbolic link…: settled in
    /// the folder, then marked resolved.
    Unusual,
}

/// A conflict the overview can only settle as it is in the folder.
#[derive(Debug, Clone, uniffi::Record)]
pub struct UnusualInfo {
    pub what: String,
    pub advice: String,
    /// Marking it resolved stages it; otherwise it stages its removal.
    pub in_folder: bool,
}

/// One of the two versions of a conflicted file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum FileVersion {
    /// Git's "ours".
    Ours,
    /// Git's "theirs".
    Theirs,
}

/// Who deleted a file the other side changed.
#[derive(Debug, Clone, uniffi::Record)]
pub struct DeletionInfo {
    pub deleted_by: FileVersion,
    pub deleted_label: String,
    pub kept_label: String,
    pub kept_lines: Option<u32>,
    /// What the side that kept it changed, line by line.
    pub changes: Vec<CommitDiffLine>,
}

/// One version of a binary file.
#[derive(Debug, Clone, uniffi::Record)]
pub struct BinaryVersionInfo {
    pub label: String,
    /// Bytes; `None` when that side has no such file.
    pub size: Option<u64>,
    /// The content when small enough to preview.
    pub data: Option<Vec<u8>>,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct BinaryInfo {
    pub ours: BinaryVersionInfo,
    pub theirs: BinaryVersionInfo,
}

/// One conflict of an Xcode project file.
#[derive(Debug, Clone, uniffi::Record)]
pub struct ProjectConflictInfo {
    pub ours_adds: Vec<String>,
    pub theirs_adds: Vec<String>,
    pub ours_removes: Vec<String>,
    pub theirs_removes: Vec<String>,
    /// "MARKETING_VERSION 1.3 → 1.4".
    pub ours_changes: Vec<String>,
    pub theirs_changes: Vec<String>,
    pub keeps_both: bool,
    /// Why it is left to the editor.
    pub reason: Option<String>,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct ProjectInfo {
    pub ours_label: String,
    pub theirs_label: String,
    pub conflicts: Vec<ProjectConflictInfo>,
    /// How many conflicts keeping both settles; the others stay for the
    /// editor.
    pub resolvable: u32,
    /// Objects the result refers to without defining them, outside the
    /// conflicts left ("A2000002 ForecastView.swift").
    pub missing: Vec<String>,
    /// Removing the missing objects' list entries repairs the project.
    pub missing_removable: bool,
}

/// A string catalog value both sides changed differently.
#[derive(Debug, Clone, uniffi::Record)]
pub struct CatalogConflictInfo {
    pub key: Option<String>,
    /// "fr › value".
    pub detail: String,
    pub base: Option<String>,
    pub ours: Option<String>,
    pub theirs: Option<String>,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct CatalogInfo {
    pub ours_label: String,
    pub theirs_label: String,
    /// Values taken from the one side that changed them.
    pub automatic: u32,
    pub conflicts: Vec<CatalogConflictInfo>,
}

/// A Swift package one side or both changed: "5.8.1", "main @ 3f2a9c1".
#[derive(Debug, Clone, uniffi::Record)]
pub struct PinChangeInfo {
    pub identity: String,
    pub base: Option<String>,
    pub ours: Option<String>,
    pub theirs: Option<String>,
    /// Ours moved it; otherwise theirs did. For a pin taken automatically,
    /// the side it was taken from.
    pub moved_by_ours: bool,
}

/// A `Package.resolved` merged package by package.
#[derive(Debug, Clone, uniffi::Record)]
pub struct PackagesInfo {
    pub ours_label: String,
    pub theirs_label: String,
    /// Pins taken from the one side that changed them.
    pub automatic: Vec<PinChangeInfo>,
    /// Pins both sides changed differently: one pick each.
    pub conflicts: Vec<PinChangeInfo>,
    /// Both sides changed `originHash`; Xcode computes it again.
    pub origin_hash_stale: bool,
}

/// A package a lockfile's sides changed.
#[derive(Debug, Clone, uniffi::Record)]
pub struct PackageChangeInfo {
    pub name: String,
    pub base: Option<String>,
    pub ours: Option<String>,
    pub theirs: Option<String>,
    /// Both sides moved it, differently.
    pub both_changed: bool,
}

/// A lockfile's two sides, and how to write it again.
#[derive(Debug, Clone, uniffi::Record)]
pub struct LockfileInfo {
    pub ours_label: String,
    pub theirs_label: String,
    /// "npm", "CocoaPods"…
    pub tool: String,
    /// "ios/Podfile".
    pub manifest: String,
    pub manifest_conflicted: bool,
    /// "cd ios && pod install".
    pub command: String,
    /// What to know before running it, when it may not keep the locked
    /// versions.
    pub caveat: Option<String>,
    /// Both sides' lines can be kept (`go.sum`).
    pub merges_by_lines: bool,
    pub changes: Vec<PackageChangeInfo>,
}

/// The operation in progress and its files.
#[derive(Debug, Clone, uniffi::Record)]
pub struct OverviewSnapshot {
    /// The repository folder's name.
    pub repository: String,
    pub root: String,
    /// "rebase", "merge", "cherry-pick" or "revert"; `None` when no
    /// operation is in progress.
    pub operation: Option<String>,
    /// The operation in plain words.
    pub summary: Option<String>,
    /// The same without a rebase's step: "Rebasing feature onto main".
    pub headline: Option<String>,
    /// Rebase only: the commit being applied, from 1, and how many.
    pub step: Option<u32>,
    pub steps: Option<u32>,
    /// A rebase's todo list and where git stands in it.
    pub plan: Option<RebasePlan>,
    /// A merge's two branches since their merge base.
    pub merge: Option<MergeHistory>,
    /// The steps git applied on its own, without conflicts, before the one
    /// it stopped on.
    pub applied: Vec<u32>,
    /// A rebase stopped on a `reword` line: the message to write.
    pub reword: Option<RewordInfo>,
    pub files: Vec<OverviewFile>,
    pub can_continue: bool,
    pub can_skip: bool,
}

/// The commit message a `reword` line lets the user write before
/// continuing.
#[derive(Debug, Clone, uniffi::Record)]
pub struct RewordInfo {
    /// The commit's own message, to start from.
    pub message: String,
    /// Git made the commit already, and stopped for its message; otherwise
    /// it makes it once the conflicts are staged.
    pub committed: bool,
    /// The commit git made, once `committed`.
    pub commit: Option<RewordCommitInfo>,
}

/// The commit a reword stopped on, as git replayed it.
#[derive(Debug, Clone, uniffi::Record)]
pub struct RewordCommitInfo {
    /// The full id, for `commit_diff`.
    pub id: String,
    pub author: String,
    /// The author is the repository's `user.email`.
    pub by_you: bool,
    /// Seconds since 1970.
    pub authored_at: i64,
    pub files: Vec<ChangedFileInfo>,
}

/// A file a commit changes.
#[derive(Debug, Clone, uniffi::Record)]
pub struct ChangedFileInfo {
    pub path: String,
    pub name: String,
    /// With a trailing slash; empty at the root.
    pub directory: String,
    /// `None` for a binary file.
    pub added: Option<u32>,
    pub removed: Option<u32>,
}

/// What a line of a diff is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum DiffLineKind {
    /// A hunk's start: its text is the enclosing function or the ranges.
    Hunk,
    Context,
    Added,
    Removed,
}

/// One line of a file's diff in a commit.
#[derive(Debug, Clone, uniffi::Record)]
pub struct CommitDiffLine {
    pub kind: DiffLineKind,
    pub old_number: Option<u32>,
    pub new_number: Option<u32>,
    pub text: String,
}

/// A commit of the merge's history.
#[derive(Debug, Clone, uniffi::Record)]
pub struct HistoryCommit {
    pub short_id: String,
    pub subject: String,
}

/// One branch of a merge since the merge base, newest commit first.
#[derive(Debug, Clone, uniffi::Record)]
pub struct HistoryLane {
    pub name: String,
    pub commits: Vec<HistoryCommit>,
    /// Commits past those listed.
    pub more: u32,
}

/// The two branches a merge joins, since their merge base.
#[derive(Debug, Clone, uniffi::Record)]
pub struct MergeHistory {
    /// The branch checked out (git's "ours").
    pub ours: HistoryLane,
    /// The branch merged in (git's "theirs").
    pub theirs: HistoryLane,
    pub base: HistoryCommit,
}

/// The commit a rebase replays onto, and its todo list.
#[derive(Debug, Clone, uniffi::Record)]
pub struct RebasePlan {
    /// The branch replayed onto, such as "main", or the commit's id.
    pub onto_name: String,
    /// The branch being rebased; `None` on a detached HEAD.
    pub branch: Option<String>,
    pub onto_short_id: String,
    pub onto_subject: String,
    pub steps: Vec<RebaseStep>,
}

/// One line of a rebase's todo list.
#[derive(Debug, Clone, uniffi::Record)]
pub struct RebaseStep {
    pub number: u32,
    pub action: StepAction,
    /// Git's word: "pick", "reword", "fixup", "drop", "exec"…
    pub action_name: String,
    /// The todo list's commit, which stays once the step is done.
    pub short_id: Option<String>,
    /// The commit's subject, or an `exec`'s command; once done, the
    /// subject of the commit made.
    pub subject: String,
    /// Once done, the commit git made of the step.
    pub replayed_id: Option<String>,
    /// A reword's subject before it.
    pub was: Option<String>,
    pub state: RebaseStepState,
    /// A fixup or squash joins the commit of this step.
    pub joins: Option<u32>,
}

/// A todo list line's action.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum StepAction {
    Pick,
    Reword,
    Edit,
    Squash,
    Fixup,
    Drop,
    Exec,
    Break,
    /// `label`, `reset`, `merge`, `update-ref`.
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum RebaseStepState {
    Done,
    /// The step git stopped on.
    Current,
    Pending,
}

/// What the user asks of the operation.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum OverviewAction {
    StageResolved,
    /// Stages resolved files, or files to review as the rule left them.
    Stage {
        paths: Vec<String>,
    },
    /// Gives staged files their conflict back, keeping their content.
    Unstage {
        paths: Vec<String>,
    },
    /// At a `reword` line, `message` replaces the commit's message.
    Continue {
        message: Option<String>,
    },
    Skip,
    Abort,
    KeepFile {
        path: String,
    },
    DeleteFile {
        path: String,
    },
    TakeVersion {
        path: String,
        version: FileVersion,
    },
    KeepBothAdditions {
        path: String,
        remove_missing: bool,
    },
    ResolveCatalog {
        path: String,
        picks: Vec<FileVersion>,
    },
    /// Writes one side of a lockfile, unstaged.
    TakeLockfile {
        path: String,
        version: FileVersion,
    },
    /// Keeps both sides' lines of a `go.sum`.
    MergeLockfileLines {
        path: String,
    },
    ResolvePackages {
        path: String,
        picks: Vec<FileVersion>,
    },
    /// Stages an unusual conflict as it is in the folder.
    MarkResolved {
        path: String,
    },
}

fn diff_line(line: commit_changes::DiffLine) -> CommitDiffLine {
    CommitDiffLine {
        kind: match line.kind {
            commit_changes::LineKind::Hunk => DiffLineKind::Hunk,
            commit_changes::LineKind::Context => DiffLineKind::Context,
            commit_changes::LineKind::Added => DiffLineKind::Added,
            commit_changes::LineKind::Removed => DiffLineKind::Removed,
        },
        old_number: line.old_number.map(|number| number as u32),
        new_number: line.new_number.map(|number| number as u32),
        text: line.text,
    }
}

/// What a commit message gets wrong, if anything.
#[derive(Debug, Clone, uniffi::Record)]
pub struct MessageCheck {
    pub empty: bool,
    pub subject_length: u32,
    pub subject_too_long: bool,
    pub second_line_is_text: bool,
}

/// The subject's length that `git log`, GitHub and the rebase plan show
/// whole.
#[uniffi::export]
pub fn commit_subject_limit() -> u32 {
    scry_core::commit_message::SUBJECT_LIMIT as u32
}

/// Checks a commit message as git and forges read it.
#[uniffi::export]
pub fn check_message(message: String) -> MessageCheck {
    let check = scry_core::commit_message::check(&message);
    MessageCheck {
        empty: check.empty,
        subject_length: check.subject_length as u32,
        subject_too_long: check.subject_too_long,
        second_line_is_text: check.second_line_is_text,
    }
}

/// What an overview action did, mirroring the core's `OverviewEvent`.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum OverviewEvent {
    Staged {
        paths: Vec<String>,
    },
    Unstaged {
        paths: Vec<String>,
    },
    Continued,
    Skipped,
    Aborted,
    /// A file kept, deleted or taken whole, and staged.
    Settled {
        path: String,
    },
    /// A file written without markers, to stage with the others.
    Written {
        path: String,
    },
}

/// An action's events, and the overview it leaves.
#[derive(Debug, Clone, uniffi::Record)]
pub struct OverviewOutcome {
    pub events: Vec<OverviewEvent>,
    pub snapshot: OverviewSnapshot,
}

fn overview_event(event: overview::OverviewEvent) -> OverviewEvent {
    match event {
        overview::OverviewEvent::Staged { paths } => OverviewEvent::Staged { paths },
        overview::OverviewEvent::Unstaged { paths } => OverviewEvent::Unstaged { paths },
        overview::OverviewEvent::Continued => OverviewEvent::Continued,
        overview::OverviewEvent::Skipped => OverviewEvent::Skipped,
        overview::OverviewEvent::Aborted => OverviewEvent::Aborted,
        overview::OverviewEvent::Settled { path } => OverviewEvent::Settled { path },
        overview::OverviewEvent::Written { path } => OverviewEvent::Written { path },
    }
}

/// The operation overview of one repository (S1).
#[derive(uniffi::Object)]
pub struct RepositoryOverview {
    git: GitCli,
    overview: Mutex<overview::Overview>,
}

#[uniffi::export]
impl RepositoryOverview {
    /// The overview of the repository holding `path`, any folder in it.
    #[uniffi::constructor]
    pub fn open(path: String) -> Result<Arc<RepositoryOverview>, ScryError> {
        let probe = GitCli::discover(PathBuf::from(&path));
        let root = overview::root(&probe).map_err(|error| match error {
            GitError::Spawn(_) => ScryError::from(error),
            _ => ScryError::NotARepository {
                message: format!("{path} is not in a git repository."),
            },
        })?;
        Ok(Arc::new(RepositoryOverview {
            git: GitCli::discover(root.clone()),
            overview: Mutex::new(overview::Overview::new(root)),
        }))
    }

    /// Reads the operation and its files again.
    pub fn snapshot(&self) -> Result<OverviewSnapshot, ScryError> {
        let mut overview = self.lock();
        let snapshot = overview.refresh(&self.git)?;
        let root = overview.root().to_path_buf();
        let operation = snapshot.operation.as_ref();
        Ok(OverviewSnapshot {
            repository: root
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default(),
            root: root.display().to_string(),
            operation: operation.map(|operation| {
                match operation.kind {
                    operation::OperationKind::Merge => "merge",
                    operation::OperationKind::Rebase => "rebase",
                    operation::OperationKind::CherryPick => "cherry-pick",
                    operation::OperationKind::Revert => "revert",
                }
                .to_string()
            }),
            summary: operation.map(|operation| operation.summary()),
            headline: operation.map(|operation| operation.headline()),
            step: operation
                .and_then(|operation| operation.progress)
                .map(|progress| progress.current as u32),
            steps: operation
                .and_then(|operation| operation.progress)
                .map(|progress| progress.total as u32),
            can_continue: snapshot.can_continue(),
            can_skip: snapshot.can_skip(),
            applied: snapshot.applied.iter().map(|step| *step as u32).collect(),
            reword: snapshot.reword.as_ref().map(|reword| RewordInfo {
                message: reword.message.clone(),
                committed: reword.committed,
                commit: reword.commit.as_ref().map(|commit| RewordCommitInfo {
                    id: commit.id.clone(),
                    author: commit.author.clone(),
                    by_you: commit.by_you,
                    authored_at: commit.authored_at,
                    files: commit
                        .files
                        .iter()
                        .map(|file| ChangedFileInfo {
                            path: file.path.clone(),
                            name: file.name.clone(),
                            directory: file.directory.clone(),
                            added: file.added.map(|count| count as u32),
                            removed: file.removed.map(|count| count as u32),
                        })
                        .collect(),
                }),
            }),
            merge: snapshot.merge.as_ref().map(|graph| {
                let commit = |commit: &scry_core::merge_graph::GraphCommit| HistoryCommit {
                    short_id: commit.short_id.clone(),
                    subject: commit.subject.clone(),
                };
                let lane = |lane: &scry_core::merge_graph::Lane| HistoryLane {
                    name: lane.name.clone(),
                    commits: lane.commits.iter().map(commit).collect(),
                    more: lane.more as u32,
                };
                MergeHistory {
                    ours: lane(&graph.ours),
                    theirs: lane(&graph.theirs),
                    base: commit(&graph.base),
                }
            }),
            plan: snapshot.plan.as_ref().map(|plan| RebasePlan {
                onto_name: operation
                    .map(|operation| operation.ours.name.clone())
                    .unwrap_or_else(|| plan.onto_short_id.clone()),
                branch: operation.and_then(|operation| operation.branch.clone()),
                onto_short_id: plan.onto_short_id.clone(),
                onto_subject: plan.onto_subject.clone(),
                steps: plan
                    .steps
                    .iter()
                    .map(|step| RebaseStep {
                        number: step.number as u32,
                        action: match step.action {
                            rebase_plan::Action::Pick => StepAction::Pick,
                            rebase_plan::Action::Reword => StepAction::Reword,
                            rebase_plan::Action::Edit => StepAction::Edit,
                            rebase_plan::Action::Squash => StepAction::Squash,
                            rebase_plan::Action::Fixup => StepAction::Fixup,
                            rebase_plan::Action::Drop => StepAction::Drop,
                            rebase_plan::Action::Exec => StepAction::Exec,
                            rebase_plan::Action::Break => StepAction::Break,
                            rebase_plan::Action::Other => StepAction::Other,
                        },
                        action_name: step.action.name().to_string(),
                        short_id: step.short_id.clone(),
                        subject: step.subject.clone(),
                        replayed_id: step.replayed_id.clone(),
                        was: step.was.clone(),
                        state: match step.state {
                            rebase_plan::StepState::Done => RebaseStepState::Done,
                            rebase_plan::StepState::Current => RebaseStepState::Current,
                            rebase_plan::StepState::Pending => RebaseStepState::Pending,
                        },
                        joins: step.joins.map(|number| number as u32),
                    })
                    .collect(),
            }),
            files: snapshot
                .files
                .into_iter()
                .map(|file| OverviewFile {
                    absolute_path: root.join(&file.path).display().to_string(),
                    can_open: file.can_open(),
                    can_stage: file.can_stage(),
                    can_unstage: file.can_unstage(),
                    path: file.path,
                    name: file.name,
                    directory: file.directory,
                    kind: file.kind,
                    state: conflict_state(file.state),
                    conflicts: file.conflicts.map(|count| count as u32),
                    openable: file.openable,
                    review: file.review.map(|progress| ReviewProgress {
                        reviewed: progress.reviewed as u32,
                        total: progress.total as u32,
                        changed_outside: progress.changed_outside,
                    }),
                    special: file.special.map(|special| match special {
                        overview::Special::DeletedOnOneSide => SpecialKind::DeletedOnOneSide,
                        overview::Special::Binary => SpecialKind::Binary,
                        overview::Special::ProjectFile => SpecialKind::ProjectFile,
                        overview::Special::StringCatalog => SpecialKind::StringCatalog,
                        overview::Special::PackageResolved => SpecialKind::PackageResolved,
                        overview::Special::Lockfile => SpecialKind::Lockfile,
                        overview::Special::NotMerged => SpecialKind::NotMerged,
                        overview::Special::Unusual => SpecialKind::Unusual,
                    }),
                })
                .collect(),
        })
    }

    /// Runs one of the operation's steps: what it did, and the overview
    /// read again, so the UI needs no read of its own. The UI asks before
    /// `Abort`.
    pub fn apply(&self, action: OverviewAction) -> Result<OverviewOutcome, ScryError> {
        let command = match action {
            OverviewAction::StageResolved => overview::OverviewCommand::StageResolved,
            OverviewAction::Stage { paths } => overview::OverviewCommand::Stage { paths },
            OverviewAction::Unstage { paths } => overview::OverviewCommand::Unstage { paths },
            OverviewAction::Continue { message } => overview::OverviewCommand::Continue { message },
            OverviewAction::Skip => overview::OverviewCommand::Skip,
            OverviewAction::Abort => overview::OverviewCommand::Abort,
            OverviewAction::KeepFile { path } => overview::OverviewCommand::KeepFile { path },
            OverviewAction::DeleteFile { path } => overview::OverviewCommand::DeleteFile { path },
            OverviewAction::TakeVersion { path, version } => {
                overview::OverviewCommand::TakeVersion {
                    path,
                    version: core_version(version),
                }
            }
            OverviewAction::KeepBothAdditions {
                path,
                remove_missing,
            } => overview::OverviewCommand::KeepBothAdditions {
                path,
                remove_missing,
            },
            OverviewAction::ResolveCatalog { path, picks } => {
                overview::OverviewCommand::ResolveCatalog {
                    path,
                    picks: picks.into_iter().map(core_version).collect(),
                }
            }
            OverviewAction::TakeLockfile { path, version } => {
                overview::OverviewCommand::TakeLockfile {
                    path,
                    version: core_version(version),
                }
            }
            OverviewAction::MergeLockfileLines { path } => {
                overview::OverviewCommand::MergeLockfileLines { path }
            }
            OverviewAction::ResolvePackages { path, picks } => {
                overview::OverviewCommand::ResolvePackages {
                    path,
                    picks: picks.into_iter().map(core_version).collect(),
                }
            }
            OverviewAction::MarkResolved { path } => {
                overview::OverviewCommand::MarkResolved { path }
            }
        };
        let events = self.lock().apply(&self.git, command)?;
        Ok(OverviewOutcome {
            events: events.into_iter().map(overview_event).collect(),
            snapshot: self.snapshot()?,
        })
    }

    /// The diff `commit` makes to `path`, read only.
    pub fn commit_diff(
        &self,
        commit: String,
        path: String,
    ) -> Result<Vec<CommitDiffLine>, ScryError> {
        let lines = commit_changes::diff(&self.git, &commit, &path)?;
        Ok(lines.into_iter().map(diff_line).collect())
    }

    /// Who deleted a file the other side changed.
    pub fn deletion(&self, path: String) -> Result<DeletionInfo, ScryError> {
        let deletion = self.lock().deletion(&self.git, &path)?;
        Ok(DeletionInfo {
            deleted_by: ffi_version(deletion.deleted_by),
            deleted_label: deletion.deleted_label,
            kept_label: deletion.kept_label,
            kept_lines: deletion.kept_lines.map(|lines| lines as u32),
            changes: deletion.changes.into_iter().map(diff_line).collect(),
        })
    }

    /// What git left of an unusual conflict, and how to settle it.
    pub fn unusual(&self, path: String) -> Result<UnusualInfo, ScryError> {
        let unusual = self.lock().unusual(&self.git, &path)?;
        Ok(UnusualInfo {
            what: unusual.what,
            advice: unusual.advice,
            in_folder: unusual.in_folder,
        })
    }

    /// Both versions of a file to take whole.
    pub fn binary(&self, path: String) -> Result<BinaryInfo, ScryError> {
        let (ours, theirs) = self.lock().binary(&self.git, &path)?;
        let version = |version: overview::BinaryVersion| BinaryVersionInfo {
            label: version.label,
            size: version.size,
            data: version.bytes,
        };
        Ok(BinaryInfo {
            ours: version(ours),
            theirs: version(theirs),
        })
    }

    /// What each side adds to an Xcode project file.
    pub fn project(&self, path: String) -> Result<ProjectInfo, ScryError> {
        let review = self.lock().project(&self.git, &path)?;
        let (ours_label, theirs_label) = overview::side_labels(&self.git)?;
        Ok(ProjectInfo {
            ours_label,
            theirs_label,
            conflicts: review
                .conflicts
                .into_iter()
                .map(|conflict| ProjectConflictInfo {
                    ours_adds: conflict.ours_adds,
                    theirs_adds: conflict.theirs_adds,
                    ours_removes: conflict.ours_removes,
                    theirs_removes: conflict.theirs_removes,
                    ours_changes: conflict.ours_changes,
                    theirs_changes: conflict.theirs_changes,
                    keeps_both: conflict.keeps_both,
                    reason: conflict.reason,
                })
                .collect(),
            resolvable: review.resolvable as u32,
            missing_removable: review.missing_removable,
            missing: review.missing,
        })
    }

    /// A string catalog merged key by key, and the values left to choose.
    pub fn catalog(&self, path: String) -> Result<CatalogInfo, ScryError> {
        let merge = self.lock().catalog(&self.git, &path)?;
        let (ours_label, theirs_label) = overview::side_labels(&self.git)?;
        Ok(CatalogInfo {
            ours_label,
            theirs_label,
            automatic: merge.automatic as u32,
            conflicts: merge
                .conflicts
                .into_iter()
                .map(|conflict| CatalogConflictInfo {
                    key: conflict.key,
                    detail: conflict.detail,
                    base: conflict.base,
                    ours: conflict.ours,
                    theirs: conflict.theirs,
                })
                .collect(),
        })
    }
}

#[uniffi::export]
impl RepositoryOverview {
    /// A `Package.resolved` merged package by package.
    pub fn packages(&self, path: String) -> Result<PackagesInfo, ScryError> {
        let merge = self.lock().packages(&self.git, &path)?;
        let (ours_label, theirs_label) = overview::side_labels(&self.git)?;
        let pins = |changes: Vec<scry_core::resolved::PinChange>| {
            changes
                .into_iter()
                .map(|change| PinChangeInfo {
                    moved_by_ours: change.moved_by_ours(),
                    identity: change.identity,
                    base: change.base,
                    ours: change.ours,
                    theirs: change.theirs,
                })
                .collect()
        };
        Ok(PackagesInfo {
            ours_label,
            theirs_label,
            origin_hash_stale: merge.origin_hash_stale,
            automatic: pins(merge.automatic),
            conflicts: pins(merge.conflicts),
        })
    }

    /// What each side changed in a lockfile, and how to write it again.
    pub fn lockfile(&self, path: String) -> Result<LockfileInfo, ScryError> {
        let review = self.lock().lockfile(&self.git, &path)?;
        let (ours_label, theirs_label) = overview::side_labels(&self.git)?;
        Ok(LockfileInfo {
            ours_label,
            theirs_label,
            tool: review.tool,
            manifest: review.manifest,
            manifest_conflicted: review.manifest_conflicted,
            command: review.command,
            caveat: review.caveat,
            merges_by_lines: review.merges_by_lines,
            changes: review
                .changes
                .into_iter()
                .map(|change| PackageChangeInfo {
                    both_changed: change.both_changed(),
                    name: change.name,
                    base: change.base,
                    ours: change.ours,
                    theirs: change.theirs,
                })
                .collect(),
        })
    }
}

fn core_version(version: FileVersion) -> overview::Version {
    match version {
        FileVersion::Ours => overview::Version::Ours,
        FileVersion::Theirs => overview::Version::Theirs,
    }
}

fn ffi_version(version: overview::Version) -> FileVersion {
    match version {
        overview::Version::Ours => FileVersion::Ours,
        overview::Version::Theirs => FileVersion::Theirs,
    }
}

impl RepositoryOverview {
    fn lock(&self) -> MutexGuard<'_, overview::Overview> {
        self.overview
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// What git's global configuration says about the merge tool.
#[derive(Debug, Clone, uniffi::Enum)]
pub enum MergetoolSetup {
    NotSet,
    Other {
        tool: String,
    },
    /// `current` when its command runs this app, not one moved since.
    Scry {
        current: bool,
    },
}

/// The merge tool command that runs `cli` (the app's `git-scry`) to open
/// `app` (the app bundle).
#[uniffi::export]
pub fn mergetool_command(cli: String, app: String) -> String {
    mergetool::command(Path::new(&cli), Path::new(&app))
}

/// Reads the merge tool from git's global configuration.
#[uniffi::export]
pub fn mergetool_setup(cli: String, app: String) -> Result<MergetoolSetup, ScryError> {
    let command = mergetool_command(cli, app);
    Ok(
        match mergetool::setup(&global_git(), &command).map_err(failed)? {
            mergetool::Setup::NotSet => MergetoolSetup::NotSet,
            mergetool::Setup::Other { tool } => MergetoolSetup::Other { tool },
            mergetool::Setup::Scry { current } => MergetoolSetup::Scry { current },
        },
    )
}

/// Makes Scry git's merge tool for every repository, in git's global
/// configuration.
#[uniffi::export]
pub fn install_mergetool(cli: String, app: String) -> Result<(), ScryError> {
    if mergetool::translocated(Path::new(&app)) {
        return Err(ScryError::Refused {
            message: String::from(
                "macOS runs Scry from a temporary copy, which git could not find once Scry quits. Move Scry to the Applications folder, open it from there, then try again.",
            ),
        });
    }
    let command = mergetool_command(cli, app);
    mergetool::install(&global_git(), &command).map_err(failed)
}

/// The merge tool Scry replaced, put back when it stops being git's.
#[uniffi::export]
pub fn mergetool_previous() -> Result<Option<String>, ScryError> {
    mergetool::previous_tool(&global_git()).map_err(failed)
}

/// Stops using Scry as git's merge tool, putting back the one it replaced.
#[uniffi::export]
pub fn uninstall_mergetool() -> Result<(), ScryError> {
    mergetool::uninstall(&global_git()).map_err(failed)
}

/// Git for the global configuration, run from the home folder.
fn global_git() -> GitCli {
    let home = std::env::var_os("HOME").map_or_else(|| PathBuf::from("/"), PathBuf::from);
    GitCli::discover(home)
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
        Command::MarkReviewed { conflict } => core::Command::MarkReviewed {
            conflict: conflict as usize,
        },
        Command::Reopen { conflict } => core::Command::Reopen {
            conflict: conflict as usize,
        },
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
        core::Command::MarkReviewed { conflict } => Command::MarkReviewed {
            conflict: conflict as u32,
        },
        core::Command::Reopen { conflict } => Command::Reopen {
            conflict: conflict as u32,
        },
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
        columns: cell.columns as u32,
        kind: match cell.kind {
            layout::CellKind::Common => CellKind::Common,
            layout::CellKind::Conflict => CellKind::Conflict,
            layout::CellKind::Spacer => CellKind::Spacer,
            layout::CellKind::Unresolved => CellKind::Unresolved,
            layout::CellKind::Chosen => CellKind::Chosen,
            layout::CellKind::Removed => CellKind::Removed,
            layout::CellKind::Base => CellKind::Base,
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

    /// Errors reach the app by kind, each a sentence to show as it is:
    /// a folder outside git is not reported like a refused action, and
    /// neither prints as `Failed(message: …)`.
    #[test]
    fn errors_say_what_failed_in_a_sentence() {
        let outside = tempfile::tempdir().unwrap();
        let Err(error) = RepositoryOverview::open(outside.path().display().to_string()) else {
            panic!("a folder outside git opened");
        };
        assert!(matches!(error, ScryError::NotARepository { .. }));
        assert!(error.to_string().ends_with("is not in a git repository."));

        let repository = tempfile::tempdir().unwrap();
        let initialised = std::process::Command::new("git")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .args(["init", "--quiet"])
            .current_dir(repository.path())
            .status()
            .unwrap();
        assert!(initialised.success());
        let overview = RepositoryOverview::open(repository.path().display().to_string()).unwrap();
        let error = overview.apply(OverviewAction::Abort).unwrap_err();
        assert!(matches!(error, ScryError::Refused { .. }));
        assert_eq!(
            error.to_string(),
            "No merge, rebase, cherry-pick or revert in progress"
        );
    }

    /// A file resolved outside Scry opens for review through the API the
    /// app uses: its conflict is resolved with what the file holds, needs
    /// review, and marking it reviewed is what the status counts. While
    /// the file still has its markers, the conflict opens open. The conflict
    /// is one line on each side, so the user's conflict style, which git
    /// reads here, gives the same conflict.
    #[test]
    fn a_resolved_file_opens_for_review() {
        let dir = tempfile::tempdir().unwrap();
        let git = |args: &[&str]| {
            let status = std::process::Command::new("git")
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .args([
                    "-c",
                    "user.name=Scry Test",
                    "-c",
                    "user.email=test@example.com",
                ])
                .args(args)
                .current_dir(dir.path())
                .output()
                .unwrap()
                .status;
            status.success()
        };
        let path = dir.path().join("a.txt");
        assert!(git(&["init", "--quiet", "--initial-branch=main"]));
        std::fs::write(&path, "first\nbase\nlast\n").unwrap();
        assert!(git(&["add", "a.txt"]));
        assert!(git(&["commit", "--quiet", "-m", "base"]));
        assert!(git(&["switch", "--quiet", "-c", "feature"]));
        std::fs::write(&path, "first\nfeature\nlast\n").unwrap();
        assert!(git(&["commit", "--quiet", "-am", "feature"]));
        assert!(git(&["switch", "--quiet", "main"]));
        std::fs::write(&path, "first\nmain\nlast\n").unwrap();
        assert!(git(&["commit", "--quiet", "-am", "main"]));
        assert!(!git(&["merge", "--quiet", "feature"]));

        let shown = path.display().to_string();
        let open = MergeDocument::open(shown.clone()).unwrap();
        assert!(open.conflicts()[0].chosen.is_none());
        std::fs::write(&path, "first\nfeature\nlast\n").unwrap();
        let document = MergeDocument::open(shown).unwrap();
        let conflict = document.conflict(0).unwrap();
        assert!(matches!(conflict.state, ConflictState::NeedsReview));
        assert_eq!(conflict.chosen.unwrap().id, "theirs");
        assert!(!conflict.outside.unwrap().reviewed);
        assert_eq!(document.status().unreviewed, 1);
        document
            .apply(Command::MarkReviewed { conflict: 0 })
            .unwrap();
        assert_eq!(document.status().unreviewed, 0);
        assert!(document.conflict(0).unwrap().outside.unwrap().reviewed);
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

        // The list leaves candidates unchecked; the focused conflict is
        // asked for checked.
        let conflicts = document.conflicts();
        assert_eq!(conflicts[0].candidates[1].id, "theirs");
        assert!(!conflicts[0].checked);
        let conflict = document.conflict(0).unwrap();
        assert!(conflict.checked);
        assert!(matches!(conflict.candidates[1].parse, ParseStatus::Parses));
        assert!(document.conflicts()[0].checked);
        assert!(document.conflict(1).is_none());
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
    /// writes anyway. `changed_on_disk` tells it before writing, so the
    /// app asks once instead of chaining alerts.
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

        assert!(!document.changed_on_disk());
        std::fs::write(&path, "let a = 42\n").unwrap();
        assert!(document.changed_on_disk());
        assert!(matches!(
            save(&document, false),
            Err(ScryError::ChangedOnDisk { .. })
        ));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "let a = 42\n");

        save(&document, true).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "let a = 2\n");
        assert!(!document.changed_on_disk());
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

    /// Opened by `git mergetool`, a file in the `merge` style shows its
    /// base, rebuilt from the three versions as the CLI does, and saving
    /// writes the merged file.
    #[test]
    fn open_mergetool_rebuilds_the_base() {
        let dir = tempfile::tempdir().unwrap();
        let write = |name: &str, text: &str| {
            let path = dir.path().join(name);
            std::fs::write(&path, text).unwrap();
            path.display().to_string()
        };
        let base = write("a_BASE.swift", "let a = 0\n");
        let local = write("a_LOCAL.swift", "let a = 1\n");
        let remote = write("a_REMOTE.swift", "let a = 2\n");
        let merged = write(
            "a.swift",
            "<<<<<<< ours\nlet a = 1\n=======\nlet a = 2\n>>>>>>> theirs\n",
        );

        let document = MergeDocument::open_mergetool(base, local, remote, merged.clone()).unwrap();
        let conflicts = document.conflicts();
        assert!(
            conflicts[0]
                .candidates
                .iter()
                .any(|candidate| candidate.id == "base")
        );

        choose(&document, "theirs").unwrap();
        save(&document, false).unwrap();
        assert_eq!(std::fs::read_to_string(&merged).unwrap(), "let a = 2\n");
    }
}
