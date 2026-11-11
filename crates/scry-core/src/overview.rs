//! The operation overview (S1): every file git left with conflicts in the
//! operation in progress, where each stands, and the operation's own steps
//! (stage the resolved files, continue, skip, abort).
//!
//! Like a merge session, it changes the repository only through commands
//! (portability rule 2). Nothing here runs on an agent's behalf: every
//! command comes from the user.

use std::fmt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::conflict;
use crate::git::{Git, GitError};
use crate::merge_graph::{self, MergeGraph};
use crate::operation::{self, Operation, OperationKind};
use crate::rebase_plan::{self, Plan};
use crate::syntax::Language;
use crate::theme::State;

/// One file of the overview.
#[derive(Debug, Clone, PartialEq, Eq)]
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
}

/// The operation and its files, as the overview shows them.
#[derive(Debug, Clone, PartialEq, Eq)]
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
    pub files: Vec<FileView>,
}

impl Snapshot {
    pub fn count(&self, state: State) -> usize {
        self.files.iter().filter(|file| file.state == state).count()
    }

    /// Continue is offered once git has every file.
    pub fn can_continue(&self) -> bool {
        self.operation.is_some() && self.files.iter().all(|file| file.state == State::Staged)
    }

    /// A merge has no commit to skip.
    pub fn can_skip(&self) -> bool {
        self.operation
            .as_ref()
            .is_some_and(|operation| operation.kind != OperationKind::Merge)
    }
}

/// Something the user asks of the operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "kebab-case")]
pub enum OverviewCommand {
    /// Stages every file resolved and not staged yet.
    StageResolved,
    /// Goes on with the operation once every file is staged: the next
    /// commit of a rebase, or the merge commit.
    Continue,
    /// Leaves the current commit out (rebase, cherry-pick, revert).
    Skip,
    /// Gives up the whole operation; the UI asks first.
    Abort,
}

/// What a command did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "kebab-case")]
pub enum OverviewEvent {
    Staged { paths: Vec<String> },
    Continued,
    Skipped,
    Aborted,
}

/// Why a command was refused.
#[derive(Debug)]
pub enum OverviewError {
    NoOperation,
    /// Files git still has with conflicts.
    NotStaged(usize),
    CannotSkipMerge,
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
        let unmerged = if operation.is_some() {
            operation::conflicted_paths(git)?
        } else {
            Vec::new()
        };
        if step != self.step {
            self.step = step;
            self.paths = unmerged.clone();
        } else {
            for path in &unmerged {
                if !self.paths.contains(path) {
                    self.paths.push(path.clone());
                }
            }
        }
        let mut files = Vec::new();
        for path in &self.paths {
            files.push(self.file(git, path, unmerged.contains(path))?);
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
        let Some(operation) = &snapshot.operation else {
            return Err(OverviewError::NoOperation);
        };
        let verb = verb(operation.kind);
        match command {
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
            OverviewCommand::Continue => {
                let left = snapshot
                    .files
                    .iter()
                    .filter(|file| file.state != State::Staged)
                    .count();
                if left > 0 {
                    return Err(OverviewError::NotStaged(left));
                }
                // The commit keeps git's message: no editor to wait for.
                go_on(git, &["-c", "core.editor=true", verb, "--continue"])?;
                Ok(vec![OverviewEvent::Continued])
            }
            OverviewCommand::Skip => {
                if operation.kind == OperationKind::Merge {
                    return Err(OverviewError::CannotSkipMerge);
                }
                go_on(git, &["-c", "core.editor=true", verb, "--skip"])?;
                Ok(vec![OverviewEvent::Skipped])
            }
            OverviewCommand::Abort => {
                git.run(&[verb, "--abort"])?;
                Ok(vec![OverviewEvent::Aborted])
            }
        }
    }

    fn file(&self, git: &dyn Git, path: &str, unmerged: bool) -> Result<FileView, GitError> {
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
        };
        if !unmerged {
            return Ok(view);
        }
        view.state = State::Unresolved;
        // Stages 2 and 3 are ours and theirs: one missing means that side
        // deleted the file.
        let stages = git.run(&["ls-files", "--unmerged", "-z", "--", path])?;
        let has = |stage: &str| {
            stages
                .split('\0')
                .any(|entry| entry.split_whitespace().nth(2) == Some(stage))
        };
        if !(has("2") && has("3")) {
            view.kind = String::from("Deleted on one side");
            return Ok(view);
        }
        let Ok(bytes) = std::fs::read(self.root.join(path)) else {
            return Ok(view);
        };
        let Ok(text) = String::from_utf8(bytes) else {
            view.kind = String::from("Binary");
            return Ok(view);
        };
        let size = conflict::marker_size(git, Path::new(path));
        if let Ok(segments) = conflict::parse_with_marker_size(&text, size) {
            let count = conflict::conflicts(&segments).count();
            view.conflicts = Some(count);
            view.openable = true;
            if count == 0 {
                view.state = State::Resolved;
            }
        }
        Ok(view)
    }
}

/// What the file is, in words.
fn kind(path: &str) -> String {
    if path.ends_with(".pbxproj") {
        return String::from("Xcode project");
    }
    if path.ends_with(".xcstrings") {
        return String::from("String catalog");
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

/// Continues or skips. Git exits with an error when the next commit stops
/// on conflicts too, which is not a failure: the overview then lists them.
fn go_on(git: &dyn Git, args: &[&str]) -> Result<(), GitError> {
    let output = git.output(args)?;
    if output.status == Some(0) || !operation::conflicted_paths(git)?.is_empty() {
        return Ok(());
    }
    Err(GitError::Failed {
        command: args.join(" "),
        stderr: output.stderr,
    })
}
