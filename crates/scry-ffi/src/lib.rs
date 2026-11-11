//! The Scry core exposed to Swift through UniFFI.
//!
//! The macOS app only displays what this returns and calls its commands
//! (portability rule 1). Types here are plain records converted from the
//! core's, so the core stays free of UniFFI.

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use scry_core::blame;
use scry_core::command::{self as core, CommandError};
use scry_core::conflict;
use scry_core::git::GitCli;
use scry_core::highlight;
use scry_core::layout;
use scry_core::mergetool;
use scry_core::operation;
use scry_core::overview;
use scry_core::presentation;
use scry_core::rebase_plan;
use scry_core::resolution::CandidateKind;
use scry_core::session::{self, MergeSession, MergetoolFiles};
use scry_core::syntax::{Language, ParseCheck};
use scry_core::theme;
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
        let git = git_beside(&path);
        let marker_size = match path.file_name() {
            Some(name) => conflict::marker_size(&git, Path::new(name)),
            None => conflict::DEFAULT_MARKER_SIZE,
        };
        let session = MergeSession::from_text_with_marker_size(&text, language, marker_size)
            .map_err(failed)?
            .with_file(path.clone());
        let operation = operation::current(&git).ok().flatten();
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
        let git = git_beside(&files.merged);
        let operation = operation::current(&git).ok().flatten();
        let (ours, theirs) = presentation::side_labels(operation.as_ref());
        let session = session::load(&git, &files, &ours, &theirs).map_err(failed)?;
        Ok(MergeDocument::new(&files.merged, session, &git, operation))
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

    fn conflict_info(&self, session: &MergeSession, index: usize) -> ConflictInfo {
        let view = session.conflict_view(index, &self.header.ours.label, &self.header.theirs.label);
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
    /// What the side that kept it changed, as a unified diff.
    pub changes: String,
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
    pub files: Vec<OverviewFile>,
    pub can_continue: bool,
    pub can_skip: bool,
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
    /// Git's word: "pick", "reword", "fixup", "drop", "exec"…
    pub action: String,
    pub short_id: Option<String>,
    /// The commit's subject, or an `exec`'s command.
    pub subject: String,
    pub state: RebaseStepState,
    /// A fixup or squash joins the commit of this step.
    pub joins: Option<u32>,
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
    Continue,
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
        let probe = GitCli::with_program(git_program(), PathBuf::from(&path));
        let root = overview::root(&probe)
            .map_err(|_| failed(format!("{path} is not in a git repository")))?;
        Ok(Arc::new(RepositoryOverview {
            git: GitCli::with_program(git_program(), root.clone()),
            overview: Mutex::new(overview::Overview::new(root)),
        }))
    }

    /// Reads the operation and its files again.
    pub fn snapshot(&self) -> Result<OverviewSnapshot, ScryError> {
        let mut overview = self.lock();
        let snapshot = overview.refresh(&self.git).map_err(failed)?;
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
            step: operation
                .and_then(|operation| operation.progress)
                .map(|progress| progress.current as u32),
            steps: operation
                .and_then(|operation| operation.progress)
                .map(|progress| progress.total as u32),
            can_continue: snapshot.can_continue(),
            can_skip: snapshot.can_skip(),
            applied: snapshot.applied.iter().map(|step| *step as u32).collect(),
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
                        action: step.action.name().to_string(),
                        short_id: step.short_id.clone(),
                        subject: step.subject.clone(),
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
                    path: file.path,
                    name: file.name,
                    directory: file.directory,
                    kind: file.kind,
                    state: conflict_state(file.state),
                    conflicts: file.conflicts.map(|count| count as u32),
                    openable: file.openable,
                    special: file.special.map(|special| match special {
                        overview::Special::DeletedOnOneSide => SpecialKind::DeletedOnOneSide,
                        overview::Special::Binary => SpecialKind::Binary,
                        overview::Special::ProjectFile => SpecialKind::ProjectFile,
                        overview::Special::StringCatalog => SpecialKind::StringCatalog,
                        overview::Special::PackageResolved => SpecialKind::PackageResolved,
                        overview::Special::Lockfile => SpecialKind::Lockfile,
                    }),
                })
                .collect(),
        })
    }

    /// Runs one of the operation's steps; the UI asks before `Abort`.
    pub fn apply(&self, action: OverviewAction) -> Result<(), ScryError> {
        let command = match action {
            OverviewAction::StageResolved => overview::OverviewCommand::StageResolved,
            OverviewAction::Continue => overview::OverviewCommand::Continue,
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
        };
        self.lock()
            .apply(&self.git, command)
            .map(|_| ())
            .map_err(failed)
    }

    /// Who deleted a file the other side changed.
    pub fn deletion(&self, path: String) -> Result<DeletionInfo, ScryError> {
        let deletion = self.lock().deletion(&self.git, &path).map_err(failed)?;
        Ok(DeletionInfo {
            deleted_by: ffi_version(deletion.deleted_by),
            deleted_label: deletion.deleted_label,
            kept_label: deletion.kept_label,
            kept_lines: deletion.kept_lines.map(|lines| lines as u32),
            changes: deletion.changes,
        })
    }

    /// Both versions of a file to take whole.
    pub fn binary(&self, path: String) -> Result<BinaryInfo, ScryError> {
        let (ours, theirs) = self.lock().binary(&self.git, &path).map_err(failed)?;
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
        let review = self.lock().project(&self.git, &path).map_err(failed)?;
        let (ours_label, theirs_label) = overview::side_labels(&self.git).map_err(failed)?;
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
        let merge = self.lock().catalog(&self.git, &path).map_err(failed)?;
        let (ours_label, theirs_label) = overview::side_labels(&self.git).map_err(failed)?;
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
        let merge = self.lock().packages(&self.git, &path).map_err(failed)?;
        let (ours_label, theirs_label) = overview::side_labels(&self.git).map_err(failed)?;
        let pins = |changes: Vec<scry_core::resolved::PinChange>| {
            changes
                .into_iter()
                .map(|change| PinChangeInfo {
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
        let review = self.lock().lockfile(&self.git, &path).map_err(failed)?;
        let (ours_label, theirs_label) = overview::side_labels(&self.git).map_err(failed)?;
        Ok(LockfileInfo {
            ours_label,
            theirs_label,
            tool: review.tool,
            manifest: review.manifest,
            manifest_conflicted: review.manifest_conflicted,
            command: review.command,
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
    let command = mergetool_command(cli, app);
    mergetool::install(&global_git(), &command).map_err(failed)
}

/// Git for the global configuration, run from the home folder.
fn global_git() -> GitCli {
    let home = std::env::var_os("HOME").map_or_else(|| PathBuf::from("/"), PathBuf::from);
    GitCli::with_program(git_program(), home)
}

/// Git, run from the directory of the file at `path`.
fn git_beside(path: &Path) -> GitCli {
    let directory = match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.to_path_buf(),
        _ => PathBuf::from("."),
    };
    GitCli::with_program(git_program(), directory)
}

/// The git the user runs. Opened from the Finder, the app gets launchd's
/// `PATH`, where `git` is Apple's `/usr/bin/git`: without Xcode or its
/// command-line tools, it asks to install them instead of running. A git
/// elsewhere on the `PATH`, or else Homebrew's, is the one the user's
/// terminal runs.
fn git_program() -> PathBuf {
    let path = std::env::var_os("PATH").unwrap_or_default();
    choose_git(std::env::split_paths(&path).collect(), |candidate| {
        candidate.is_file()
    })
}

fn choose_git(path: Vec<PathBuf>, exists: impl Fn(&Path) -> bool) -> PathBuf {
    let apple = Path::new("/usr/bin/git");
    let homebrew = [
        PathBuf::from("/opt/homebrew/bin/git"),
        PathBuf::from("/usr/local/bin/git"),
    ];
    path.into_iter()
        .map(|directory| directory.join("git"))
        .filter(|candidate| candidate != apple)
        .chain(homebrew)
        .find(|candidate| exists(candidate))
        .unwrap_or_else(|| PathBuf::from("git"))
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
        columns: cell.columns as u32,
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

    /// Opened from the Finder, the app must not pick Apple's `/usr/bin/git`
    /// over the git the user installed, which may ask to install Xcode's
    /// tools; a git on the user's own `PATH` comes first.
    #[test]
    fn picks_the_users_git() {
        let installed = |found: &'static [&'static str]| {
            move |candidate: &Path| found.iter().any(|path| candidate == Path::new(path))
        };
        let launchd = vec![PathBuf::from("/usr/bin"), PathBuf::from("/bin")];

        assert_eq!(
            choose_git(
                launchd.clone(),
                installed(&["/usr/bin/git", "/opt/homebrew/bin/git"])
            ),
            PathBuf::from("/opt/homebrew/bin/git")
        );
        assert_eq!(
            choose_git(
                vec![PathBuf::from("/nix/bin"), PathBuf::from("/usr/bin")],
                installed(&["/nix/bin/git", "/opt/homebrew/bin/git"])
            ),
            PathBuf::from("/nix/bin/git")
        );
        // Only Apple's: plain `git`, found on the PATH as before.
        assert_eq!(
            choose_git(launchd, installed(&["/usr/bin/git"])),
            PathBuf::from("git")
        );
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
