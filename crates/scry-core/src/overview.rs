//! The operation overview (S1): every file git left with conflicts in the
//! operation in progress, where each stands, and the operation's own steps
//! (stage the resolved files, continue, skip, abort).
//!
//! Like a merge session, it changes the repository only through commands
//! (portability rule 2). Nothing here runs on an agent's behalf: every
//! command comes from the user.

use std::collections::HashMap;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::catalog::{self, CatalogMerge};
use crate::commit_changes::{self, ChangedFile, DiffLine};
use crate::conflict;
use crate::git::{Git, GitError};
use crate::lockfile::{self, PackageChange};
use crate::merge_graph::{self, MergeGraph};
use crate::operation::{self, Operation, OperationKind};
use crate::pbxproj::{self, ProjectReview};
use crate::rebase_plan::{self, Action, Plan};
use crate::records::Store;
use crate::resolved::{self, ResolvedMerge};
use crate::review;
use crate::session::{self, MergetoolFiles, write_atomically};
use crate::syntax::Language;
use crate::theme::State;

impl FileView {
    /// It opens from the overview: in the merge editor, or in its own
    /// view. A staged file is unstaged first.
    pub fn can_open(&self) -> bool {
        (self.openable || self.special.is_some()) && self.state != State::Staged
    }

    /// Git can take it as it is: resolved, or a rule's result to accept.
    pub fn can_stage(&self) -> bool {
        matches!(self.state, State::Resolved | State::NeedsReview)
    }

    /// Its conflict can come back, to change it.
    pub fn can_unstage(&self) -> bool {
        self.state == State::Staged
    }
}

/// One file of the overview.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FileView {
    /// From the repository root, as git names it.
    pub path: String,
    pub name: String,
    /// The folder, from the root; empty at the root.
    pub directory: String,
    /// "Swift", "Xcode project", "Deleted on one side"…
    pub kind: String,
    /// Unresolved while it has markers, resolved once it has none, staged
    /// once git has it.
    pub state: State,
    /// Conflicts left in the file, when it can be read.
    pub conflicts: Option<usize>,
    /// Whether the merge editor can open it: a text file with markers, or
    /// resolved but not staged yet.
    pub openable: bool,
    /// The view that settles it, for a file the editor cannot (deleted on
    /// one side, binary) or one a view settles better (Xcode project,
    /// string catalog); `None` once staged.
    pub special: Option<Special>,
    /// For a text file resolved and not staged: how far the review of its
    /// resolutions is. Not fully reviewed, the file needs review.
    pub review: Option<review::Progress>,
}

/// A conflicted file the overview settles with its own view.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Special {
    /// One side deleted it, the other changed it: keep it or delete it.
    DeletedOnOneSide,
    /// No text to merge: take one version.
    Binary,
    /// `project.pbxproj`: usually both sides adding entries.
    ProjectFile,
    /// `.xcstrings`: merged key by key.
    StringCatalog,
    /// `Package.resolved`: merged package by package.
    PackageResolved,
    /// A lockfile (`package-lock.json`, `Podfile.lock`…): take a side, then
    /// let the package manager write it again.
    Lockfile,
    /// `.gitattributes` tells git not to merge it (`-merge`, `binary`):
    /// git left ours without markers. Take one version, as for a binary.
    NotMerged,
    /// A conflict with no two versions to compare: both sides renamed the
    /// file, a name only one side has, a submodule, a symbolic link.
    /// Settle it in the folder, then mark it resolved as it is there.
    Unusual,
}

/// One of the two versions of a conflicted file, as git stages them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Version {
    /// Git's "ours" (stage 2).
    Ours,
    /// Git's "theirs" (stage 3).
    Theirs,
}

impl Version {
    fn stage(self) -> &'static str {
        match self {
            Version::Ours => "2",
            Version::Theirs => "3",
        }
    }

    fn flag(self) -> &'static str {
        match self {
            Version::Ours => "--ours",
            Version::Theirs => "--theirs",
        }
    }
}

/// Who deleted a file the other side changed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Deletion {
    pub deleted_by: Version,
    /// The sides as the editor names them ("main · rebase target").
    pub deleted_label: String,
    pub kept_label: String,
    /// The lines of the version that changed it.
    pub kept_lines: Option<usize>,
    /// What that side changed since the base, line by line: what deleting
    /// the file would throw away.
    pub changes: Vec<DiffLine>,
}

/// One version of a binary file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BinaryVersion {
    pub label: String,
    /// Bytes; `None` when that side has no such file.
    pub size: Option<u64>,
    /// The content, up to `BINARY_PREVIEW_LIMIT`, so the UI can show an
    /// image.
    pub bytes: Option<Vec<u8>>,
}

/// A lockfile's two sides, and how to write it again.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LockfileReview {
    /// "npm", "CocoaPods"…
    pub tool: String,
    /// The manifest beside it, from the root ("web/package.json").
    pub manifest: String,
    /// The manifest has conflicts too: resolve it first, since the tool
    /// writes the lockfile from it.
    pub manifest_conflicted: bool,
    /// What to run once a side is taken, from the root
    /// ("cd web && npm install").
    pub command: String,
    /// What to know before running it, when it may not keep the locked
    /// versions (Poetry 1, Pipenv, Carthage).
    pub caveat: Option<String>,
    /// Both sides' lines can be kept (`go.sum`).
    pub merges_by_lines: bool,
    /// The packages each side changed since the base.
    pub changes: Vec<PackageChange>,
}

/// A conflict the overview can only settle as it is in the folder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UnusualConflict {
    /// What git left, in a sentence.
    pub what: String,
    /// What to do in the folder before marking it resolved.
    pub advice: String,
    /// The path exists in the folder: marking it resolved stages it;
    /// otherwise it stages its removal.
    pub in_folder: bool,
}

/// Binary versions larger than this are not read for a preview.
pub const BINARY_PREVIEW_LIMIT: u64 = 8 * 1024 * 1024;

/// The operation and its files, as the overview shows them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Snapshot {
    /// `None` once the operation is finished or aborted.
    pub operation: Option<Operation>,
    /// A rebase's todo list and where git stands in it.
    pub plan: Option<Plan>,
    /// A merge's two branches since their merge base.
    pub merge: Option<MergeGraph>,
    /// The steps git applied on its own, without conflicts, before the
    /// one it stopped on: kept until it stops elsewhere.
    pub applied: Vec<usize>,
    /// The message of the commit a `reword` step stopped on, to write
    /// before continuing.
    pub reword: Option<Reword>,
    pub files: Vec<FileView>,
}

/// A rebase stopped on a `reword` line: the user writes the commit's
/// message, which Continue gives git.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Reword {
    /// The message to start from: the commit's own.
    pub message: String,
    /// Git already made the commit and stopped for its message, so
    /// Continue amends it. Otherwise the step stopped on conflicts, and
    /// git makes the commit with this message once they are staged.
    pub committed: bool,
    /// The commit git made, once `committed`: what it changes, to read
    /// while writing its message.
    pub commit: Option<RewordCommit>,
    /// The original commit the line rewords (`stopped-sha`), so a message
    /// prepared for it never reaches another commit.
    pub original: String,
}

/// The commit a reword stopped on, as git replayed it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RewordCommit {
    /// The full id, to ask for a file's diff.
    pub id: String,
    pub author: String,
    /// The author's email is the repository's `user.email`.
    pub by_you: bool,
    /// When it was authored, in seconds since 1970.
    pub authored_at: i64,
    pub files: Vec<ChangedFile>,
}

impl Snapshot {
    pub fn count(&self, state: State) -> usize {
        self.files.iter().filter(|file| file.state == state).count()
    }

    /// Continue is offered once git has every file.
    pub fn can_continue(&self) -> bool {
        self.operation.is_some() && self.files.iter().all(|file| file.state == State::Staged)
    }

    /// A merge has no commit to skip. Nor has a `reword` git already
    /// committed: `--skip` would keep that commit, with its old message.
    pub fn can_skip(&self) -> bool {
        self.operation
            .as_ref()
            .is_some_and(|operation| operation.kind != OperationKind::Merge)
            && !self.reword.as_ref().is_some_and(|reword| reword.committed)
    }
}

/// Something the user asks of the operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "kebab-case")]
pub enum OverviewCommand {
    /// Stages every file resolved and not staged yet.
    StageResolved,
    /// Stages resolved files, or files to review as the rule left them,
    /// in one git run.
    Stage { paths: Vec<String> },
    /// Gives staged files their conflict back in git, keeping their
    /// content as resolved: to change a choice before continuing.
    Unstage { paths: Vec<String> },
    /// Goes on with the operation once every file is staged: the next
    /// commit of a rebase, or the merge commit. At a `reword` step,
    /// `message` replaces the commit's message; without it, the message
    /// stays.
    Continue {
        #[serde(default)]
        message: Option<String>,
    },
    /// Leaves the current commit out (rebase, cherry-pick, revert).
    Skip,
    /// Gives up the whole operation; the UI asks first.
    Abort,
    /// Keeps a file one side deleted, as the other side changed it.
    KeepFile { path: String },
    /// Deletes a file one side deleted and the other changed.
    DeleteFile { path: String },
    /// Takes one version of a file whole, such as a binary file.
    TakeVersion { path: String, version: Version },
    /// Keeps both sides' changes to an Xcode project file where that is
    /// safe; with `remove_missing`, also removes the list entries of
    /// objects no longer defined.
    KeepBothAdditions { path: String, remove_missing: bool },
    /// Writes a string catalog merged key by key, with one pick per value
    /// both sides changed.
    ResolveCatalog { path: String, picks: Vec<Version> },
    /// Writes one side of a lockfile without staging it: the package
    /// manager writes it again first.
    TakeLockfile { path: String, version: Version },
    /// Keeps both sides' lines of a `go.sum`.
    MergeLockfileLines { path: String },
    /// Writes a `Package.resolved` merged package by package, with one
    /// pick per package both sides changed.
    ResolvePackages { path: String, picks: Vec<Version> },
    /// Stages an unusual conflict as it is in the folder, or its removal
    /// when it is not there.
    MarkResolved { path: String },
}

impl OverviewCommand {
    /// The file a command writes or settles; `None` for the operation's
    /// own steps and for staging, which check their paths themselves.
    fn settled_path(&self) -> Option<&str> {
        match self {
            OverviewCommand::KeepFile { path }
            | OverviewCommand::DeleteFile { path }
            | OverviewCommand::TakeVersion { path, .. }
            | OverviewCommand::KeepBothAdditions { path, .. }
            | OverviewCommand::ResolveCatalog { path, .. }
            | OverviewCommand::TakeLockfile { path, .. }
            | OverviewCommand::MergeLockfileLines { path }
            | OverviewCommand::ResolvePackages { path, .. }
            | OverviewCommand::MarkResolved { path } => Some(path),
            OverviewCommand::StageResolved
            | OverviewCommand::Stage { .. }
            | OverviewCommand::Unstage { .. }
            | OverviewCommand::Continue { .. }
            | OverviewCommand::Skip
            | OverviewCommand::Abort => None,
        }
    }
}

/// What a command did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "kebab-case")]
pub enum OverviewEvent {
    Staged {
        paths: Vec<String>,
    },
    /// Staged files have their conflict back.
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

/// Why a command was refused.
#[derive(Debug)]
pub enum OverviewError {
    NoOperation,
    /// Files git still has with conflicts.
    NotStaged(usize),
    CannotSkipMerge,
    /// The command does not fit the file (keeping a file nobody deleted).
    NotThatKind(String),
    /// Keeping both additions would break the project file.
    CannotKeepBoth(String),
    Invalid(String),
    Io(std::io::Error),
    Git(GitError),
}

impl From<GitError> for OverviewError {
    fn from(error: GitError) -> OverviewError {
        OverviewError::Git(error)
    }
}

impl fmt::Display for OverviewError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            OverviewError::NoOperation => {
                write!(f, "no merge, rebase, cherry-pick or revert in progress")
            }
            OverviewError::NotStaged(count) => write!(f, "{count} file(s) not staged yet"),
            OverviewError::CannotSkipMerge => write!(f, "a merge has no commit to skip"),
            OverviewError::NotThatKind(path) => write!(f, "{path} cannot be settled this way"),
            OverviewError::CannotKeepBoth(path) => {
                write!(
                    f,
                    "keeping both sides would break {path}: resolve it in the editor"
                )
            }
            OverviewError::Invalid(reason) => write!(f, "{reason}"),
            OverviewError::Io(error) => error.fmt(f),
            OverviewError::Git(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for OverviewError {}

/// The repository's root, from any folder in it.
pub fn root(git: &dyn Git) -> Result<PathBuf, GitError> {
    Ok(PathBuf::from(
        git.run(&["rev-parse", "--show-toplevel"])?.trim(),
    ))
}

/// The overview of the repository at `root`; its git runs there.
pub struct Overview {
    root: PathBuf,
    /// The step the files belong to (the commit being applied), so a new
    /// commit's conflicts start a new list.
    step: Option<String>,
    /// Every file with conflicts at this step, in git's order: those
    /// staged since stay listed, as staged.
    paths: Vec<String>,
    /// The rebase step last seen, to tell which steps git applied since.
    last_step: Option<usize>,
    applied: Vec<usize>,
}

impl Overview {
    pub fn new(root: PathBuf) -> Overview {
        Overview {
            root,
            step: None,
            paths: Vec::new(),
            last_step: None,
            applied: Vec::new(),
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Reads the operation and its files again.
    pub fn refresh(&mut self, git: &dyn Git) -> Result<Snapshot, GitError> {
        let operation = operation::current(git)?;
        let step = operation.as_ref().map(step);
        // Listed with no operation too: `git stash pop` and `git checkout
        // --merge` leave conflicts without one, and nothing to continue.
        let unmerged = operation::conflicted_paths(git)?;
        if step != self.step {
            self.step = step;
            // What was reviewed at another stop will never be asked again.
            if let Ok(store) = Store::open(git) {
                let _ = store.prune();
            }
            // Files staged at this step before Scry opened: git keeps
            // their conflict until the step ends (resolve-undo).
            self.paths = unmerged.clone();
            if operation.is_some() {
                for path in staged_this_step(git)? {
                    if !self.paths.contains(&path) {
                        self.paths.push(path);
                    }
                }
                self.paths.sort();
            }
        } else {
            if operation.is_none() && !self.paths.is_empty() {
                // No step ends to reset the list: a file stays while it
                // is in conflict or staged, and leaves once committed.
                let staged = staged_paths(git)?;
                self.paths
                    .retain(|path| unmerged.contains(path) || staged.contains(path));
            }
            for path in &unmerged {
                if !self.paths.contains(path) {
                    self.paths.push(path.clone());
                }
            }
        }
        // One git run each for every file's stages and attributes, rather
        // than several per file: a large conflict list refreshes on every
        // activation of the app.
        let facts = if unmerged.is_empty() {
            Facts::default()
        } else {
            Facts {
                stages: Stages::all(git)?,
                attributes: attributes(git, &unmerged)?,
                store: Store::open(git).ok(),
            }
        };
        let mut files = Vec::new();
        for path in &self.paths {
            files.push(self.file(git, path, unmerged.contains(path), &facts)?);
        }
        let plan = match &operation {
            Some(operation) if operation.kind == OperationKind::Rebase => rebase_plan::read(git)?,
            _ => None,
        };
        let merge = match &operation {
            Some(Operation {
                kind: OperationKind::Merge,
                ours,
                theirs: Some(theirs),
                ..
            }) => merge_graph::read(git, &ours.name, &theirs.commit.id, &theirs.name)?,
            _ => None,
        };
        let reword = reword(git, plan.as_ref(), unmerged.is_empty())?;
        let current = plan.as_ref().and_then(Plan::current);
        if current != self.last_step {
            // Git ran every line between the one it stopped on before
            // and the one it stops on now.
            self.applied = match (self.last_step, current) {
                (Some(before), Some(now)) if now > before + 1 => (before + 1..now).collect(),
                _ => Vec::new(),
            };
            self.last_step = current;
        }
        Ok(Snapshot {
            operation,
            plan,
            merge,
            applied: self.applied.clone(),
            reword,
            files,
        })
    }

    /// Runs a command, then the overview is read again with `refresh`.
    pub fn apply(
        &mut self,
        git: &dyn Git,
        command: OverviewCommand,
    ) -> Result<Vec<OverviewEvent>, OverviewError> {
        let snapshot = self.refresh(git)?;
        // Files are settled without an operation too (`git stash pop`);
        // only Continue, Skip and Abort need one.
        let operation = || match &snapshot.operation {
            Some(operation) => Ok((operation.kind, verb(operation.kind))),
            None => Err(OverviewError::NoOperation),
        };
        let state = |path: &str| {
            snapshot
                .files
                .iter()
                .find(|file| file.path == path)
                .map(|file| file.state)
        };
        // A command that writes or settles a file acts only on a file git
        // still has in conflict at this step: never on a clean file, nor
        // on a path outside the list (an absolute path, `../`).
        if let Some(path) = command.settled_path()
            && !matches!(
                state(path),
                Some(State::Unresolved | State::NeedsReview | State::Resolved)
            )
        {
            return Err(OverviewError::NotThatKind(path.to_string()));
        }
        match command {
            OverviewCommand::Stage { paths } => {
                // A file to review holds the rule's result: staging it
                // accepts that result as it is.
                if let Some(path) = paths
                    .iter()
                    .find(|path| !matches!(state(path), Some(State::Resolved | State::NeedsReview)))
                {
                    return Err(OverviewError::NotThatKind(path.clone()));
                }
                if paths.is_empty() {
                    return Ok(Vec::new());
                }
                let mut args = vec!["add", "--"];
                args.extend(paths.iter().map(String::as_str));
                git.run(&args)?;
                Ok(vec![OverviewEvent::Staged { paths }])
            }
            OverviewCommand::Unstage { paths } => {
                if let Some(path) = paths.iter().find(|path| state(path) != Some(State::Staged)) {
                    return Err(OverviewError::NotThatKind(path.clone()));
                }
                if paths.is_empty() {
                    return Ok(Vec::new());
                }
                // The index keeps each conflict git resolved by `git add`
                // or `git rm` (resolve-undo): this puts its versions back
                // and leaves the file in the folder as it is.
                let mut args = vec!["update-index", "--unresolve", "--"];
                args.extend(paths.iter().map(String::as_str));
                git.run(&args)?;
                let conflicted = operation::conflicted_paths(git)?;
                let lost: Vec<&str> = paths
                    .iter()
                    .filter(|path| !conflicted.contains(path))
                    .map(String::as_str)
                    .collect();
                if lost.is_empty() {
                    Ok(vec![OverviewEvent::Unstaged { paths }])
                } else {
                    Err(OverviewError::Invalid(format!(
                        "git no longer keeps the conflict of {}, so it cannot be unstaged",
                        lost.join(", ")
                    )))
                }
            }
            // Keeping, deleting and taking a version stage at once, unlike
            // the kinds below that only write: git keeps these conflicts in
            // the index (a missing side, a file it left as ours), so the
            // list would still read the file as unresolved from the folder.
            OverviewCommand::KeepFile { path } => {
                self.deletion(git, &path)?;
                git.run(&["add", "--", &path])?;
                Ok(vec![OverviewEvent::Settled { path }])
            }
            OverviewCommand::DeleteFile { path } => {
                self.deletion(git, &path)?;
                git.run(&["rm", "--quiet", "--", &path])?;
                Ok(vec![OverviewEvent::Settled { path }])
            }
            OverviewCommand::TakeVersion { path, version } => {
                if stage(git, &path, version)?.is_none() {
                    return Err(OverviewError::NotThatKind(path));
                }
                git.run(&["checkout", version.flag(), "--", &path])?;
                git.run(&["add", "--", &path])?;
                Ok(vec![OverviewEvent::Settled { path }])
            }
            OverviewCommand::KeepBothAdditions {
                path,
                remove_missing,
            } => {
                let (text, size) = self.project_text(git, &path)?;
                let review = pbxproj::review(&text, size)
                    .map_err(|error| OverviewError::Invalid(format!("{path}: {error:?}")))?;
                if !review.can_keep_both() {
                    return Err(OverviewError::CannotKeepBoth(path));
                }
                let result = pbxproj::keep_both(&text, size, remove_missing)
                    .map_err(|error| OverviewError::Invalid(format!("{path}: {error:?}")))?;
                write_atomically(&self.root.join(&path), &result).map_err(OverviewError::Io)?;
                Ok(vec![OverviewEvent::Written { path }])
            }
            OverviewCommand::ResolveCatalog { path, picks } => {
                let merge = self.catalog(git, &path)?;
                write_atomically(
                    &self.root.join(&path),
                    &merge
                        .resolve(&picks_of(&picks))
                        .map_err(OverviewError::Invalid)?,
                )
                .map_err(OverviewError::Io)?;
                Ok(vec![OverviewEvent::Written { path }])
            }
            OverviewCommand::TakeLockfile { path, version } => {
                self.lockfile(git, &path)?;
                if stage(git, &path, version)?.is_none() {
                    return Err(OverviewError::NotThatKind(path));
                }
                git.run(&["checkout", version.flag(), "--", &path])?;
                Ok(vec![OverviewEvent::Written { path }])
            }
            OverviewCommand::MergeLockfileLines { path } => {
                if !self.lockfile(git, &path)?.merges_by_lines {
                    return Err(OverviewError::NotThatKind(path));
                }
                let merged = lockfile::merge_lines(
                    &stage_text(git, &path, "1")?,
                    &stage_text(git, &path, "2")?,
                    &stage_text(git, &path, "3")?,
                );
                write_atomically(&self.root.join(&path), &merged).map_err(OverviewError::Io)?;
                Ok(vec![OverviewEvent::Written { path }])
            }
            OverviewCommand::ResolvePackages { path, picks } => {
                let merge = self.packages(git, &path)?;
                write_atomically(
                    &self.root.join(&path),
                    &merge
                        .resolve(&picks_of(&picks))
                        .map_err(OverviewError::Invalid)?,
                )
                .map_err(OverviewError::Io)?;
                Ok(vec![OverviewEvent::Written { path }])
            }
            OverviewCommand::MarkResolved { path } => {
                let unusual = snapshot.files.iter().any(|file| {
                    file.path == path
                        && file.state == State::Unresolved
                        && file.special == Some(Special::Unusual)
                });
                if !unusual {
                    return Err(OverviewError::NotThatKind(path));
                }
                // `--all` also stages a removal, when the path is gone.
                git.run(&["add", "--all", "--", &path])?;
                Ok(vec![OverviewEvent::Settled { path }])
            }
            OverviewCommand::StageResolved => {
                let paths: Vec<String> = snapshot
                    .files
                    .iter()
                    .filter(|file| file.state == State::Resolved)
                    .map(|file| file.path.clone())
                    .collect();
                if paths.is_empty() {
                    return Ok(Vec::new());
                }
                let mut args = vec!["add", "--"];
                args.extend(paths.iter().map(String::as_str));
                git.run(&args)?;
                Ok(vec![OverviewEvent::Staged { paths }])
            }
            OverviewCommand::Continue { message } => {
                let (_, verb) = operation()?;
                let left = snapshot
                    .files
                    .iter()
                    .filter(|file| file.state != State::Staged)
                    .count();
                if left > 0 {
                    return Err(OverviewError::NotStaged(left));
                }
                let mut editor = Editor::new(git)?;
                match (&snapshot.reword, message) {
                    (None, Some(_)) => {
                        return Err(OverviewError::Invalid(String::from(
                            "only a reword step takes a new message",
                        )));
                    }
                    (None, None) => {}
                    (Some(reword), message) => {
                        let message = message.unwrap_or_else(|| reword.message.clone());
                        if message.trim().is_empty() {
                            return Err(OverviewError::Invalid(String::from(
                                "the commit message is empty",
                            )));
                        }
                        if !reword.committed {
                            // The commit is made on Continue: the editor
                            // hands git this message, for this commit only.
                            editor
                                .prepare(&message, &reword.original)
                                .map_err(OverviewError::Io)?;
                        } else if message.trim_end() != reword.message.trim_end() {
                            git.run_with_input(
                                &["commit", "--amend", "--file=-"],
                                message.as_bytes(),
                            )?;
                        }
                    }
                }
                let result = editor.go_on(git, &[verb, "--continue"]);
                editor.discard();
                result?;
                Ok(vec![OverviewEvent::Continued])
            }
            OverviewCommand::Skip => {
                let (kind, verb) = operation()?;
                if kind == OperationKind::Merge {
                    return Err(OverviewError::CannotSkipMerge);
                }
                Editor::new(git)?.go_on(git, &[verb, "--skip"])?;
                Ok(vec![OverviewEvent::Skipped])
            }
            OverviewCommand::Abort => {
                let (_, verb) = operation()?;
                git.run(&[verb, "--abort"])?;
                Ok(vec![OverviewEvent::Aborted])
            }
        }
    }

    /// Who deleted the file, and how the other side left it.
    pub fn deletion(&self, git: &dyn Git, path: &str) -> Result<Deletion, OverviewError> {
        let ours = stage(git, path, Version::Ours)?;
        let theirs = stage(git, path, Version::Theirs)?;
        if stage_number(git, path, "1")?.is_none() {
            return Err(OverviewError::NotThatKind(path.to_string()));
        }
        let (deleted_by, kept) = match (&ours, &theirs) {
            (None, Some(kept)) => (Version::Ours, kept),
            (Some(kept), None) => (Version::Theirs, kept),
            _ => return Err(OverviewError::NotThatKind(path.to_string())),
        };
        let (ours_label, theirs_label) = side_labels(git)?;
        let (deleted_label, kept_label) = match deleted_by {
            Version::Ours => (ours_label, theirs_label),
            Version::Theirs => (theirs_label, ours_label),
        };
        let kept_stage = match deleted_by {
            Version::Ours => Version::Theirs,
            Version::Theirs => Version::Ours,
        };
        let changes = git.output(&[
            "diff",
            "--no-color",
            "--no-ext-diff",
            &format!(":1:{path}"),
            &format!(":{}:{path}", kept_stage.stage()),
        ])?;
        let changes = commit_changes::parse_diff(&String::from_utf8_lossy(&changes.stdout));
        Ok(Deletion {
            deleted_by,
            deleted_label,
            kept_label,
            changes,
            kept_lines: std::str::from_utf8(kept)
                .ok()
                .map(|text| text.lines().count()),
        })
    }

    /// What git left of an unusual conflict, and how to settle it.
    pub fn unusual(&self, git: &dyn Git, path: &str) -> Result<UnusualConflict, OverviewError> {
        let stages = Stages::of(git, path)?;
        let (ours_label, theirs_label) = side_labels(git)?;
        let (what, advice) = match stages.unusual() {
            Some(Unusual::Submodule) => (
                String::from("A submodule both sides moved to different commits."),
                String::from(
                    "Check out the commit you want inside the submodule, then mark it resolved: the commit checked out is staged.",
                ),
            ),
            Some(Unusual::Link) => (
                String::from("A symbolic link both sides changed."),
                String::from("Point the link where you want in the folder, then mark it resolved."),
            ),
            Some(Unusual::RenamedOnBothSides) => (
                String::from("Both sides renamed this file, each to its own name."),
                String::from(
                    "The new names are listed as files of their own. Mark this one resolved to drop the old name, unless you want to keep it in the folder.",
                ),
            ),
            Some(Unusual::OneSideOnly(version)) => {
                let side = match version {
                    Version::Ours => ours_label,
                    Version::Theirs => theirs_label,
                };
                (
                    format!(
                        "Only {side} has this name, in a conflict on the file's name (both sides renamed it, or added it under different names)."
                    ),
                    String::from(
                        "Keep it as it is in the folder, or delete it there first, then mark it resolved.",
                    ),
                )
            }
            None => return Err(OverviewError::NotThatKind(path.to_string())),
        };
        Ok(UnusualConflict {
            what,
            advice,
            in_folder: self.root.join(path).symlink_metadata().is_ok(),
        })
    }

    /// Both versions of a file to take whole, with their content when
    /// small enough to preview.
    pub fn binary(
        &self,
        git: &dyn Git,
        path: &str,
    ) -> Result<(BinaryVersion, BinaryVersion), OverviewError> {
        let (ours_label, theirs_label) = side_labels(git)?;
        // The size first: a version too large to preview is never read,
        // so a 2 GB asset does not pass through memory to say how big it is.
        let version = |version: Version, label: String| -> Result<BinaryVersion, OverviewError> {
            let size = stage_size(git, path, version.stage())?;
            let bytes = match size {
                Some(size) if size <= BINARY_PREVIEW_LIMIT => stage(git, path, version)?,
                _ => None,
            };
            Ok(BinaryVersion { label, size, bytes })
        };
        Ok((
            version(Version::Ours, ours_label)?,
            version(Version::Theirs, theirs_label)?,
        ))
    }

    /// What each side adds to an Xcode project file, and whether keeping
    /// both is safe.
    pub fn project(&self, git: &dyn Git, path: &str) -> Result<ProjectReview, OverviewError> {
        let (text, size) = self.project_text(git, path)?;
        pbxproj::review(&text, size)
            .map_err(|error| OverviewError::Invalid(format!("{path}: {error:?}")))
    }

    /// A string catalog's three versions merged key by key.
    pub fn catalog(&self, git: &dyn Git, path: &str) -> Result<CatalogMerge, OverviewError> {
        catalog::merge(
            &stage_text(git, path, "1")?,
            &stage_text(git, path, "2")?,
            &stage_text(git, path, "3")?,
        )
        .map_err(OverviewError::Invalid)
    }

    /// A `Package.resolved`'s three versions merged package by package.
    pub fn packages(&self, git: &dyn Git, path: &str) -> Result<ResolvedMerge, OverviewError> {
        if !is_package_resolved(path) {
            return Err(OverviewError::NotThatKind(path.to_string()));
        }
        resolved::merge(
            &stage_text(git, path, "1")?,
            &stage_text(git, path, "2")?,
            &stage_text(git, path, "3")?,
        )
        .map_err(OverviewError::Invalid)
    }

    /// What each side changed in a lockfile, and how to write it again.
    pub fn lockfile(&self, git: &dyn Git, path: &str) -> Result<LockfileReview, OverviewError> {
        let Some(format) = lockfile::Format::of(path) else {
            return Err(OverviewError::NotThatKind(path.to_string()));
        };
        let directory = path.rsplit_once('/').map(|(directory, _)| directory);
        let beside = |name: &str| match directory {
            Some(directory) => format!("{directory}/{name}"),
            None => name.to_string(),
        };
        let candidates: Vec<String> = format.manifests().iter().map(|name| beside(name)).collect();
        let manifest = candidates
            .iter()
            .find(|candidate| self.root.join(candidate).exists())
            .unwrap_or(&candidates[0])
            .clone();
        let manifest_conflicted = operation::conflicted_paths(git)?.contains(&manifest);
        let manifest_text = std::fs::read_to_string(self.root.join(&manifest)).unwrap_or_default();
        let command = match directory {
            Some(directory) => format!("cd {directory} && {}", format.command(&manifest_text)),
            None => format.command(&manifest_text).to_string(),
        };
        Ok(LockfileReview {
            tool: format.tool().to_string(),
            manifest,
            manifest_conflicted,
            command,
            caveat: format.caveat().map(String::from),
            merges_by_lines: format.merges_by_lines(),
            changes: lockfile::changes(
                format,
                &stage_text(git, path, "1")?,
                &stage_text(git, path, "2")?,
                &stage_text(git, path, "3")?,
            ),
        })
    }

    /// A project file with its markers and their size. Its conflicts need
    /// the base to tell a removal from an addition: when git wrote them in
    /// the `merge` style, and the file is still as git wrote it, they are
    /// rebuilt from the three versions with the base, as the editor does.
    fn project_text(&self, git: &dyn Git, path: &str) -> Result<(String, usize), OverviewError> {
        let text = std::fs::read_to_string(self.root.join(path)).map_err(OverviewError::Io)?;
        let size = conflict::marker_size(git, Path::new(path));
        let Ok(segments) = conflict::parse_with_marker_size(&text, size) else {
            return Ok((text, size));
        };
        if conflict::conflicts(&segments).all(|conflict| conflict.base.is_some()) {
            return Ok((text, size));
        }
        let folder = std::env::temp_dir().join(format!(
            "scry-project-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|elapsed| elapsed.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&folder).map_err(OverviewError::Io)?;
        let rebuilt = (|| -> Result<Option<String>, OverviewError> {
            let mut files = MergetoolFiles {
                base: folder.join("base"),
                local: folder.join("ours"),
                remote: folder.join("theirs"),
                merged: self.root.join(path),
            };
            for (stage, file) in [
                ("1", &mut files.base),
                ("2", &mut files.local),
                ("3", &mut files.remote),
            ] {
                let bytes = stage_number(git, path, stage)?.unwrap_or_default();
                std::fs::write(&*file, bytes).map_err(OverviewError::Io)?;
            }
            let failed =
                |_| OverviewError::Invalid(format!("{path}: cannot rebuild its conflicts"));
            let as_written =
                session::merge_file(git, &files, "ours", "theirs", size, None).map_err(failed)?;
            let unchanged = conflict::parse_with_marker_size(&as_written, size)
                .is_ok_and(|written| session::same_ignoring_labels(&segments, &written));
            if !unchanged {
                return Ok(None);
            }
            Ok(Some(
                session::merge_file(git, &files, "ours", "theirs", size, Some("--zdiff3"))
                    .map_err(failed)?,
            ))
        })();
        let _ = std::fs::remove_dir_all(&folder);
        Ok((rebuilt?.unwrap_or(text), size))
    }

    fn file(
        &self,
        git: &dyn Git,
        path: &str,
        unmerged: bool,
        facts: &Facts,
    ) -> Result<FileView, GitError> {
        let (name, directory) = match path.rsplit_once('/') {
            Some((directory, name)) => (name.to_string(), directory.to_string()),
            None => (path.to_string(), String::new()),
        };
        let mut view = FileView {
            path: path.to_string(),
            name,
            directory,
            kind: kind(path),
            state: State::Staged,
            conflicts: None,
            openable: false,
            special: None,
            review: None,
        };
        if !unmerged {
            return Ok(view);
        }
        view.state = State::Unresolved;
        let stages = facts.stages.get(path).unwrap_or(&Stages::NONE);
        if let Some(unusual) = stages.unusual() {
            view.kind = unusual.kind().to_string();
            view.special = Some(Special::Unusual);
            return Ok(view);
        }
        if stages.ours.is_none() || stages.theirs.is_none() {
            view.kind = String::from("Deleted on one side");
            view.special = Some(Special::DeletedOnOneSide);
            return Ok(view);
        }
        let Ok(bytes) = std::fs::read(self.root.join(path)) else {
            return Ok(view);
        };
        let Ok(text) = String::from_utf8(bytes) else {
            view.kind = String::from("Binary");
            view.special = Some(Special::Binary);
            return Ok(view);
        };
        view.special = if path.ends_with(".pbxproj") {
            Some(Special::ProjectFile)
        } else if path.ends_with(".xcstrings") {
            Some(Special::StringCatalog)
        } else if is_package_resolved(path) {
            Some(Special::PackageResolved)
        } else if lockfile::Format::of(path).is_some() {
            Some(Special::Lockfile)
        } else {
            None
        };
        let attributes = facts.attributes.get(path);
        let size = attributes.map_or(conflict::DEFAULT_MARKER_SIZE, |found| found.marker_size);
        if let Ok(segments) = conflict::parse_with_marker_size(&text, size) {
            let count = conflict::conflicts(&segments).count();
            view.conflicts = Some(count);
            view.openable = true;
            if count == 0 {
                let merge = attributes.map_or("", |found| found.merge.as_str());
                if left_as_ours(git, path, merge, text.as_bytes())? {
                    view.kind = String::from("Not merged by git");
                    view.special = Some(Special::NotMerged);
                    view.openable = false;
                } else {
                    view.state = State::Resolved;
                    if view.special.is_none() && matches!(merge, "unspecified" | "set" | "text") {
                        view.review = self.review(git, path, facts);
                        if view.review.is_some_and(|progress| !progress.is_done()) {
                            view.state = State::NeedsReview;
                        }
                    }
                }
            }
        }
        Ok(view)
    }
}

impl Overview {
    /// How far the review of `path`, resolved outside the merge editor or
    /// in it, is; `None` when it cannot be told (no store, a side missing,
    /// git failing), the file then reading as resolved, as before reviews.
    fn review(&self, git: &dyn Git, path: &str, facts: &Facts) -> Option<review::Progress> {
        let store = facts.store.as_ref()?;
        let found = review::find(git, &self.root, path).ok()??;
        Some(review::progress(&found, &store.reviewed(path)))
    }
}

/// Git did not merge the file (`-merge` or `merge=binary` in
/// `.gitattributes`, its `merge` attribute) and it still holds ours as git
/// left it: no markers, yet nothing was decided.
fn left_as_ours(git: &dyn Git, path: &str, merge: &str, content: &[u8]) -> Result<bool, GitError> {
    if merge != "unset" && merge != "binary" {
        return Ok(false);
    }
    Ok(stage(git, path, Version::Ours)?.as_deref() == Some(content))
}

/// What git says about the files in conflict, read once per refresh.
#[derive(Default)]
struct Facts {
    stages: HashMap<String, Stages>,
    attributes: HashMap<String, Attributes>,
    /// Where the reviews of this stop are kept.
    store: Option<Store>,
}

/// A file's attributes that change how its conflict is read.
struct Attributes {
    /// `conflict-marker-size`, or git's default.
    marker_size: usize,
    /// `merge`: "unset" for `-merge`, "binary", a driver's name, or
    /// "unspecified".
    merge: String,
}

/// The attributes of every path in `paths`, in one `git check-attr` run.
fn attributes(git: &dyn Git, paths: &[String]) -> Result<HashMap<String, Attributes>, GitError> {
    let mut input = Vec::new();
    for path in paths {
        input.extend_from_slice(path.as_bytes());
        input.push(0);
    }
    let output = git.run_with_input(
        &[
            "check-attr",
            "-z",
            "--stdin",
            "conflict-marker-size",
            "merge",
        ],
        &input,
    )?;
    let mut found: HashMap<String, Attributes> = HashMap::new();
    // `<path>\0<attribute>\0<value>\0`, for each path and attribute.
    let fields: Vec<&str> = output.split('\0').collect();
    for entry in fields.chunks(3) {
        let [path, attribute, value] = entry else {
            continue;
        };
        let attributes = found.entry(path.to_string()).or_insert(Attributes {
            marker_size: conflict::DEFAULT_MARKER_SIZE,
            merge: String::from("unspecified"),
        });
        match *attribute {
            "conflict-marker-size" => {
                if let Some(size) = value.parse().ok().filter(|size| *size > 0) {
                    attributes.marker_size = size;
                }
            }
            "merge" => attributes.merge = value.to_string(),
            _ => {}
        }
    }
    Ok(found)
}

/// The versions git staged for an unmerged path: the mode of the
/// base (stage 1), ours (2) and theirs (3), `None` when it has none.
struct Stages {
    base: Option<String>,
    ours: Option<String>,
    theirs: Option<String>,
}

/// A conflict shape with no two versions to compare.
enum Unusual {
    Submodule,
    Link,
    /// Only the base: each side gave it another name.
    RenamedOnBothSides,
    /// No base and one side: the name one side gave in a rename, or an
    /// addition in a conflict on the name.
    OneSideOnly(Version),
}

impl Unusual {
    fn kind(&self) -> &'static str {
        match self {
            Unusual::Submodule => "Submodule",
            Unusual::Link => "Symbolic link",
            Unusual::RenamedOnBothSides => "Renamed on both sides",
            Unusual::OneSideOnly(_) => "Name on one side only",
        }
    }
}

impl Stages {
    /// No stage: a path git does not have in conflict.
    const NONE: Stages = Stages {
        base: None,
        ours: None,
        theirs: None,
    };

    fn of(git: &dyn Git, path: &str) -> Result<Stages, GitError> {
        let output = git.run(&["ls-files", "--unmerged", "-z", "--", path])?;
        Ok(Stages::read(&output).remove(path).unwrap_or(Stages::NONE))
    }

    /// The stages of every path in conflict, in one git run.
    fn all(git: &dyn Git) -> Result<HashMap<String, Stages>, GitError> {
        Ok(Stages::read(&git.run(&["ls-files", "--unmerged", "-z"])?))
    }

    /// Stages by path from `git ls-files --unmerged -z` output.
    fn read(output: &str) -> HashMap<String, Stages> {
        let mut all: HashMap<String, Stages> = HashMap::new();
        // `<mode> <object> <stage>\t<path>`.
        for entry in output.split('\0') {
            let Some((info, path)) = entry.split_once('\t') else {
                continue;
            };
            let stages = all.entry(path.to_string()).or_insert(Stages::NONE);
            let mut fields = info.split(' ');
            let (Some(mode), Some(_object), Some(stage)) =
                (fields.next(), fields.next(), fields.next())
            else {
                continue;
            };
            let mode = Some(mode.to_string());
            match stage {
                "1" => stages.base = mode,
                "2" => stages.ours = mode,
                "3" => stages.theirs = mode,
                _ => {}
            }
        }
        all
    }

    fn unusual(&self) -> Option<Unusual> {
        let modes = [&self.base, &self.ours, &self.theirs];
        let any = |wanted: &str| modes.iter().any(|mode| mode.as_deref() == Some(wanted));
        if any("160000") {
            return Some(Unusual::Submodule);
        }
        if any("120000") {
            return Some(Unusual::Link);
        }
        match (&self.base, &self.ours, &self.theirs) {
            (Some(_), None, None) => Some(Unusual::RenamedOnBothSides),
            (None, Some(_), None) => Some(Unusual::OneSideOnly(Version::Ours)),
            (None, None, Some(_)) => Some(Unusual::OneSideOnly(Version::Theirs)),
            _ => None,
        }
    }
}

/// Paths whose conflict was resolved at this step, by `git add` or
/// `git rm`: git keeps their versions (resolve-undo) until the step ends.
/// The paths whose staged content differs from `HEAD`.
fn staged_paths(git: &dyn Git) -> Result<Vec<String>, GitError> {
    let output = git.run(&["diff", "--cached", "--name-only", "--no-renames", "-z"])?;
    Ok(output
        .split('\0')
        .filter(|path| !path.is_empty())
        .map(String::from)
        .collect())
}

fn staged_this_step(git: &dyn Git) -> Result<Vec<String>, GitError> {
    let output = git.run(&["ls-files", "--resolve-undo", "-z"])?;
    let mut paths: Vec<String> = Vec::new();
    for entry in output.split('\0') {
        if let Some((_, path)) = entry.split_once('\t')
            && !paths.iter().any(|known| known == path)
        {
            paths.push(path.to_string());
        }
    }
    Ok(paths)
}

/// A version of the file as git staged it; `None` when that side has
/// none (it deleted the file, or never had it).
fn stage(git: &dyn Git, path: &str, version: Version) -> Result<Option<Vec<u8>>, GitError> {
    stage_number(git, path, version.stage())
}

/// A staged version as text; empty when there is none.
fn stage_text(git: &dyn Git, path: &str, stage: &str) -> Result<String, GitError> {
    let bytes = stage_number(git, path, stage)?.unwrap_or_default();
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

fn picks_of(versions: &[Version]) -> Vec<catalog::Pick> {
    versions
        .iter()
        .map(|version| match version {
            Version::Ours => catalog::Pick::Ours,
            Version::Theirs => catalog::Pick::Theirs,
        })
        .collect()
}

fn is_package_resolved(path: &str) -> bool {
    path == "Package.resolved" || path.ends_with("/Package.resolved")
}

/// A staged version's size in bytes; `None` when there is none.
fn stage_size(git: &dyn Git, path: &str, stage: &str) -> Result<Option<u64>, GitError> {
    let spec = format!(":{stage}:{path}");
    let output = git.output(&["cat-file", "-s", &spec])?;
    if output.status != Some(0) {
        return Ok(None);
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().parse().ok())
}

fn stage_number(git: &dyn Git, path: &str, stage: &str) -> Result<Option<Vec<u8>>, GitError> {
    let spec = format!(":{stage}:{path}");
    let output = git.output(&["cat-file", "blob", &spec])?;
    Ok((output.status == Some(0)).then_some(output.stdout))
}

/// Ours and theirs as the editor names them.
pub fn side_labels(git: &dyn Git) -> Result<(String, String), GitError> {
    Ok(crate::presentation::side_labels(
        operation::current(git)?.as_ref(),
    ))
}

/// What the file is, in words.
fn kind(path: &str) -> String {
    if path.ends_with(".pbxproj") {
        return String::from("Xcode project");
    }
    if path.ends_with(".xcstrings") {
        return String::from("String catalog");
    }
    if is_package_resolved(path) {
        return String::from("Swift packages");
    }
    if let Some(format) = lockfile::Format::of(path) {
        return format!("{} lockfile", format.tool());
    }
    match Language::from_path(Path::new(path)) {
        Some(language) => language.name().to_string(),
        None => String::from("Text"),
    }
}

/// The operation's step: the commit being applied, or the operation.
fn step(operation: &Operation) -> String {
    match &operation.theirs {
        Some(side) => side.commit.id.clone(),
        None => format!("{:?}", operation.kind),
    }
}

fn verb(kind: OperationKind) -> &'static str {
    match kind {
        OperationKind::Merge => "merge",
        OperationKind::Rebase => "rebase",
        OperationKind::CherryPick => "cherry-pick",
        OperationKind::Revert => "revert",
    }
}

/// The `reword` step a rebase stopped on, if it did.
fn reword(git: &dyn Git, plan: Option<&Plan>, merged: bool) -> Result<Option<Reword>, GitError> {
    let Some(plan) = plan else {
        return Ok(None);
    };
    let is_reword = plan
        .steps
        .iter()
        .any(|step| step.state == rebase_plan::StepState::Current && step.action == Action::Reword);
    if !is_reword {
        return Ok(None);
    }
    let dir = rebase_dir(git)?;
    let Ok(stopped) = fs::read_to_string(dir.join("stopped-sha")) else {
        return Ok(None);
    };
    let stopped = stopped.trim();
    // Git stops after making the commit when its editor fails: HEAD is
    // then the commit replayed, with the author, date and message of the
    // original, and nothing left in the index. Author and date alone are
    // not enough: the commit before may share them (scripts, `commit -C`),
    // and after a conflict the staged resolution is not committed yet.
    // Git writes no marker for this stop (no `rebase-merge/amend`).
    let identity = |commit: &str| {
        git.run(&[
            "show",
            "--no-patch",
            "--no-show-signature",
            "--date=raw",
            "--format=%an%x00%ae%x00%ad%x00%B",
            commit,
            "--",
        ])
    };
    let committed = merged
        && identity("HEAD")? == identity(stopped)?
        && git
            .output(&["diff", "--cached", "--quiet", "HEAD", "--"])?
            .status
            == Some(0);
    // The commit's own message: git's `message` file adds comments
    // listing the conflicts.
    let message = message_of(git, if committed { "HEAD" } else { stopped })?;
    let commit = if committed {
        Some(reword_commit(git)?)
    } else {
        None
    };
    Ok(Some(Reword {
        message: message.trim_end().to_string(),
        committed,
        commit,
        original: stopped.to_string(),
    }))
}

/// HEAD, the commit git made of a reword, and what it changes.
fn reword_commit(git: &dyn Git) -> Result<RewordCommit, GitError> {
    let output = git.run(&[
        "show",
        "--no-patch",
        "--no-show-signature",
        "--format=%H%x00%an%x00%ae%x00%at",
        "HEAD",
        "--",
    ])?;
    let mut fields = output.trim_end().split('\0');
    let id = fields.next().unwrap_or("").to_string();
    let author = fields.next().unwrap_or("").to_string();
    let email = fields.next().unwrap_or("");
    let authored_at = fields
        .next()
        .and_then(|time| time.parse().ok())
        .unwrap_or(0);
    // No `user.email` makes git fail: then nobody is "you".
    let by_you = git
        .run(&["config", "user.email"])
        .is_ok_and(|you| !email.is_empty() && you.trim().eq_ignore_ascii_case(email));
    Ok(RewordCommit {
        files: commit_changes::files(git, &id)?,
        id,
        author,
        by_you,
        authored_at,
    })
}

fn message_of(git: &dyn Git, commit: &str) -> Result<String, GitError> {
    git.run(&[
        "show",
        "--no-patch",
        "--no-show-signature",
        "--format=%B",
        commit,
        "--",
    ])
}

fn rebase_dir(git: &dyn Git) -> Result<PathBuf, GitError> {
    Ok(git_dir(git)?.join("rebase-merge"))
}

fn git_dir(git: &dyn Git) -> Result<PathBuf, GitError> {
    Ok(PathBuf::from(
        git.run(&["rev-parse", "--absolute-git-dir"])?.trim(),
    ))
}

/// The editor Scry gives git when it continues or skips, so git never
/// waits on one. The first message git asks for is the one prepared, if
/// any; a later `reword` line makes it fail, so git stops there with the
/// commit made and the user writes its message; any other message, such
/// as a squash's or a commit after conflicts, stays as git wrote it.
struct Editor {
    /// The prepared message, in the git folder.
    message: PathBuf,
    /// The commit the message is for: git's line running must name it.
    commit: String,
    /// The comment character git is given, when the message has lines
    /// starting with `#` (`#42 Fix the queue`) that git would strip.
    comment: Option<char>,
    /// The rebase's lines done, the last one being the line running.
    done: PathBuf,
}

/// A comment character for git that no line of `message` starts with, so
/// git strips its own comments and none of the message; `None` when `#`,
/// git's own, is fine.
fn comment_char(message: &str) -> Option<char> {
    let starts = |c: char| message.lines().any(|line| line.trim_start().starts_with(c));
    if !starts('#') {
        return None;
    }
    [';', '@', '!', '$', '%', '^', '&', '|', ':']
        .into_iter()
        .find(|c| !starts(*c))
}

/// Run by git's shell with the file to write as `$1`. The paths come in
/// the environment, so they need no quoting. The prepared message is used
/// only when the line running (the last of `done`, `reword <id> …`) names
/// the commit it was written for: a reword that became empty is dropped by
/// git, and its message must not land on the next commit.
const EDITOR: &str = "sh -c 'file=\"$1\"; \
     if [ -f \"$SCRY_MESSAGE\" ]; then \
     set -- $(tail -n 1 \"$SCRY_DONE\" 2>/dev/null); \
     if [ -n \"$2\" ]; then case \"$SCRY_FOR\" in \"$2\"*) \
     cat \"$SCRY_MESSAGE\" > \"$file\" && rm -f \"$SCRY_MESSAGE\"; exit;; esac; fi; fi; \
     case \"$(tail -n 1 \"$SCRY_DONE\" 2>/dev/null)\" in r\\ *|reword\\ *) exit 1;; esac' scry-editor";

impl Editor {
    /// Discards a message left by an earlier run that stopped half-way.
    fn new(git: &dyn Git) -> Result<Editor, GitError> {
        let dir = git_dir(git)?;
        let editor = Editor {
            message: dir.join("scry-message"),
            commit: String::new(),
            comment: None,
            done: dir.join("rebase-merge").join("done"),
        };
        editor.discard();
        Ok(editor)
    }

    fn prepare(&mut self, message: &str, commit: &str) -> std::io::Result<()> {
        self.commit = commit.to_string();
        self.comment = comment_char(message);
        fs::write(&self.message, message)
    }

    /// A message git did not ask for must not reach a later commit.
    fn discard(&self) {
        let _ = fs::remove_file(&self.message);
    }

    /// Continues or skips. Git exits with an error when the next commit
    /// stops on conflicts, or on a `reword` line, which is not a failure:
    /// the overview then shows them.
    fn go_on(&self, git: &dyn Git, args: &[&str]) -> Result<(), GitError> {
        let message = self.message.display().to_string();
        let done = self.done.display().to_string();
        let comment = self.comment.map(|c| format!("core.commentChar={c}"));
        let mut full: Vec<&str> = Vec::new();
        if let Some(comment) = &comment {
            full.extend(["-c", comment]);
        }
        full.extend(args);
        // The commit being applied, to tell git stopping on the next one
        // from git failing on this one (a leftover `index.lock`).
        let before = operation::current(git)?.as_ref().map(step);
        let output = git.output_with_env(
            &full,
            &[
                // GIT_EDITOR comes before core.editor, even one inherited.
                ("GIT_EDITOR", EDITOR),
                ("SCRY_MESSAGE", &message),
                ("SCRY_FOR", &self.commit),
                ("SCRY_DONE", &done),
            ],
        )?;
        if output.status == Some(0) {
            return Ok(());
        }
        let after = operation::current(git)?.as_ref().map(step);
        if after != before && !operation::conflicted_paths(git)?.is_empty() {
            return Ok(());
        }
        let plan = rebase_plan::read(git)?;
        if reword(git, plan.as_ref(), true)?.is_some_and(|reword| reword.committed) {
            return Ok(());
        }
        Err(GitError::Failed {
            command: args.join(" "),
            stderr: output.stderr,
        })
    }
}
