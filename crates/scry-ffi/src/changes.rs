//! The Changes window: what differs between two versions of a repository,
//! and one file's diff ready to draw (`scry_core::diff` does the reading).

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use scry_core::diff::{self, Comparison, Marks};
use scry_core::file_blame::Target;
use scry_core::git::{GitCli, GitError};
use scry_core::history;
use scry_core::operation;
use scry_core::overview;
use scry_core::syntax::Language;
use scry_core::viewed::{self, Viewed};

use crate::file_blame::FileBlame;
use crate::{ByteRange, ScryError, Span, failed, span};

/// What the window says it compares.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ChangesHeader {
    /// The repository folder's name.
    pub repository: String,
    pub root: String,
    /// "Not committed", "Staged", a branch or a commit.
    pub title: String,
    /// "Against HEAD".
    pub detail: String,
    /// "Not committed, against HEAD": the window's subtitle.
    pub sentence: String,
    /// The branch checked out, when the working copy or the index is read.
    pub branch: Option<String>,
    /// The two sides' names, for the bar above the file.
    pub old_label: String,
    pub new_label: String,
    /// The files come in groups (staged, not staged, untracked): only for
    /// the working copy.
    pub grouped: bool,
    /// The working copy or the index is read: the files can change while
    /// the window is open.
    pub live: bool,
    /// Opened by `git difftool`, which gives two files or two folders and
    /// nothing else: no history, no blame, no repository to read again.
    pub reduced: bool,
    /// One file alone, with no list to choose from.
    pub single_file: bool,
    /// The repository's git folder, to watch with `root` when `live`;
    /// empty without a repository.
    pub git_dir: String,
    /// A file's history can be opened from this window.
    pub has_history: bool,
    /// The commit a file's history goes back from; `None` for HEAD, with
    /// what is not committed before it.
    pub history_from: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum ChangeState {
    Added,
    Modified,
    Deleted,
    Renamed,
    Untracked,
    TypeChanged,
    Conflicted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum ChangeGroup {
    Staged,
    NotStaged,
    Untracked,
}

/// A file that differs.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ChangesFile {
    /// Its place in the list `files` returned: what `file_diff` takes.
    pub index: u32,
    pub path: String,
    /// The path on the old side, for a renamed file.
    pub old_path: Option<String>,
    pub name: String,
    /// The folder, with a trailing slash; empty at the root.
    pub directory: String,
    pub state: ChangeState,
    pub binary: bool,
    /// A repository inside the repository.
    pub submodule: bool,
    /// Lines added and removed, when they are counted.
    pub added: Option<u32>,
    pub removed: Option<u32>,
    pub group: Option<ChangeGroup>,
    /// Under not staged, with part of its changes staged.
    pub partly_staged: bool,
    /// What identifies the content of its two sides: another one means
    /// the file changed.
    pub content_id: String,
    pub viewed: ViewedState,
}

/// Where a file stands with its "Viewed" mark.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum ViewedState {
    NotViewed,
    Viewed,
    /// Viewed, and changed since.
    ChangedSince,
}

/// A line of one side, as drawn: tabs expanded.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct DiffLine {
    pub text: String,
    /// Its width in columns of the monospaced font.
    pub columns: u32,
    pub spans: Vec<Span>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum DiffRowKind {
    Unchanged,
    Added,
    Removed,
    Changed,
}

/// A row of the split view: the old line beside the new one, either one
/// missing. Numbers start at 1.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct DiffRow {
    pub kind: DiffRowKind,
    pub old: Option<u32>,
    pub new: Option<u32>,
    /// The changed parts of each line, in UTF-8 bytes of its text.
    pub old_words: Vec<ByteRange>,
    pub new_words: Vec<ByteRange>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum UnifiedKind {
    Unchanged,
    Added,
    Removed,
}

/// A line of the unified view: in each block the removed lines, then the
/// added ones.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct UnifiedDiffLine {
    pub kind: UnifiedKind,
    pub old: Option<u32>,
    pub new: Option<u32>,
    pub words: Vec<ByteRange>,
}

/// A block of changed lines: what "next change" moves between.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Record)]
pub struct DiffChange {
    /// Its first row and its rows, in the split view.
    pub first_row: u32,
    pub row_count: u32,
    /// Its first line in the unified view, where it has `removed + added`
    /// lines.
    pub first_line: u32,
    pub added: u32,
    pub removed: u32,
}

/// A text file's diff, ready to draw in either layout.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct TextDiffInfo {
    pub old_lines: Vec<DiffLine>,
    pub new_lines: Vec<DiffLine>,
    pub rows: Vec<DiffRow>,
    pub unified: Vec<UnifiedDiffLine>,
    pub changes: Vec<DiffChange>,
    /// The widest line of either side, in columns.
    pub longest_line: u32,
    pub added: u32,
    pub removed: u32,
    /// Each side's last line has no line ending.
    pub old_missing_final_newline: bool,
    pub new_missing_final_newline: bool,
    /// A large file, opened without syntax colours and word marks.
    pub large: bool,
    /// The language's name, when Scry has a grammar for the file.
    pub language: Option<String>,
}

/// One file's diff.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum FileDiffInfo {
    Text {
        diff: TextDiffInfo,
    },
    /// Sizes in bytes; `None` for a side the file is not on.
    Binary {
        old_size: Option<u64>,
        new_size: Option<u64>,
    },
    Submodule,
    /// Left with conflicts by a merge: the merge editor resolves it.
    Conflicted,
}

/// What waits in a repository, for the list of recent ones.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum RepositoryWaiting {
    /// A merge, rebase, cherry-pick or revert in progress, or files left
    /// with conflicts without one: `headline` says which, `unresolved`
    /// how many files still have conflicts.
    Resolving {
        headline: String,
        unresolved: u32,
    },
    /// Files not committed, untracked ones included.
    NotCommitted {
        files: u32,
    },
    Nothing,
    /// The folder is gone, or is no longer a repository.
    Missing,
}

/// What waits in the repository holding `path`: what its window would
/// open on. Runs git a few times: call it off the main thread.
#[uniffi::export]
pub fn repository_waiting(path: String) -> RepositoryWaiting {
    let folder = PathBuf::from(&path);
    if !folder.is_dir() {
        return RepositoryWaiting::Missing;
    }
    let Ok(root) = overview::root(&GitCli::discover(folder)) else {
        return RepositoryWaiting::Missing;
    };
    let git = GitCli::discover(root.clone());
    let current = operation::current(&git).ok().flatten();
    let unresolved = operation::conflicted_paths(&git).map_or(0, |paths| paths.len());
    if current.is_some() || unresolved > 0 {
        return RepositoryWaiting::Resolving {
            headline: current.map_or_else(
                || String::from("Conflicts left"),
                |operation| operation.headline(),
            ),
            unresolved: unresolved as u32,
        };
    }
    let files = Comparison::uncommitted(&git)
        .and_then(|comparison| diff::changed_files(&git, &root, &comparison, &[]))
        .map_or(0, |files| files.len());
    if files > 0 {
        RepositoryWaiting::NotCommitted {
            files: files as u32,
        }
    } else {
        RepositoryWaiting::Nothing
    }
}

/// Where the files come from.
enum Source {
    /// Two versions of a repository.
    Repository {
        git: GitCli,
        root: PathBuf,
        /// Empty when git could not tell: nothing is then marked viewed.
        git_dir: PathBuf,
        scope: diff::Scope,
        paths: Vec<String>,
    },
    /// Two files `git difftool` handed over; `name` is the file's path in
    /// the repository, when git gave it.
    Files {
        git: GitCli,
        old: PathBuf,
        new: PathBuf,
        name: String,
    },
    /// Two folders, from `git difftool --dir-diff`.
    Folders {
        git: GitCli,
        old: PathBuf,
        new: PathBuf,
    },
    /// One file at a point of its history.
    Point {
        git: GitCli,
        root: PathBuf,
        /// `None` when the point is the pin: nothing differs.
        sides: Option<history::Sides>,
        header: ChangesHeader,
    },
}

/// What differs between two versions of one repository, or between the
/// two files or folders of `git difftool`.
#[derive(uniffi::Object)]
pub struct ChangeSet {
    source: Source,
    /// The files as last listed: `file_diff` takes one by its index.
    files: Mutex<Vec<Listed>>,
}

/// A file of the list, with its content's id when it was listed.
#[derive(Clone)]
struct Listed {
    file: diff::ChangedFile,
    id: diff::ContentId,
}

/// A git to compare files outside a repository with: where it runs does
/// not matter.
fn plain_git() -> GitCli {
    GitCli::discover(std::env::temp_dir())
}

fn diff_error(error: diff::DiffError) -> ScryError {
    match error {
        diff::DiffError::Git(error) => ScryError::from(error),
        other => failed(other),
    }
}

#[uniffi::export]
impl ChangeSet {
    /// What `git scry diff` compares in the repository holding `path`:
    /// everything not committed, or `revisions` (a commit, `a..b`,
    /// `a...b`), the index in place of the working copy with `staged`,
    /// limited to `paths` (from the repository's root) when there are
    /// some.
    #[uniffi::constructor]
    pub fn open(
        path: String,
        staged: bool,
        revisions: Option<String>,
        paths: Vec<String>,
    ) -> Result<Arc<ChangeSet>, ScryError> {
        let probe = GitCli::discover(PathBuf::from(&path));
        let root = overview::root(&probe).map_err(|error| match error {
            GitError::Spawn(_) => ScryError::from(error),
            _ => ScryError::NotARepository {
                message: format!("{path} is not in a git repository."),
            },
        })?;
        let git = GitCli::discover(root.clone());
        let scope =
            diff::Scope::of(&git, staged, revisions.as_deref()).map_err(|error| match error {
                diff::ScopeError::Git(error) => ScryError::from(error),
                refused => ScryError::Refused {
                    message: crate::sentence(&refused),
                },
            })?;
        Ok(Arc::new(ChangeSet {
            source: Source::Repository {
                git_dir: scry_core::asks::git_dir(&git).unwrap_or_default(),
                git,
                root,
                scope,
                paths,
            },
            files: Mutex::new(Vec::new()),
        }))
    }

    /// One commit of the repository holding `path`, against its parent:
    /// what "Show the Whole Commit" opens.
    #[uniffi::constructor]
    pub fn open_commit(path: String, commit: String) -> Result<Arc<ChangeSet>, ScryError> {
        let probe = GitCli::discover(PathBuf::from(&path));
        let root = overview::root(&probe)?;
        let git = GitCli::discover(root.clone());
        let scope = diff::Scope::commit(&git, &commit)?;
        Ok(Arc::new(ChangeSet {
            source: Source::Repository {
                git_dir: scry_core::asks::git_dir(&git).unwrap_or_default(),
                git,
                root,
                scope,
                paths: Vec::new(),
            },
            files: Mutex::new(Vec::new()),
        }))
    }

    /// The two files `git difftool` hands over, or its two folders with
    /// `--dir-diff`; `name` is the file's path in the repository
    /// (`$MERGED`), empty when git gave none.
    #[uniffi::constructor]
    pub fn open_difftool(old: String, new: String, name: String) -> Arc<ChangeSet> {
        let (old, new) = (PathBuf::from(old), PathBuf::from(new));
        let source = if old.is_dir() && new.is_dir() {
            Source::Folders {
                git: plain_git(),
                old,
                new,
            }
        } else {
            let name = if name.is_empty() {
                new.file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default()
            } else {
                name
            };
            Source::Files {
                git: plain_git(),
                old,
                new,
                name,
            }
        };
        Arc::new(ChangeSet {
            source,
            files: Mutex::new(Vec::new()),
        })
    }

    pub fn header(&self) -> ChangesHeader {
        match &self.source {
            Source::Repository {
                root,
                scope,
                git_dir,
                ..
            } => ChangesHeader {
                git_dir: git_dir.display().to_string(),
                repository: root
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                root: root.display().to_string(),
                title: scope.title.clone(),
                detail: scope.detail.clone(),
                sentence: scope.sentence(),
                branch: scope.branch.clone(),
                old_label: scope.old_label.clone(),
                new_label: scope.new_label.clone(),
                grouped: matches!(scope.comparison, Comparison::WorkingCopy { .. }),
                live: !matches!(scope.comparison, Comparison::Commits { .. }),
                reduced: false,
                single_file: false,
                has_history: true,
                history_from: match &scope.comparison {
                    Comparison::Commits { new, .. } => Some(new.clone()),
                    _ => None,
                },
            },
            Source::Point { header, .. } => header.clone(),
            Source::Files { old, new, name, .. } => ChangesHeader {
                repository: Path::new(name)
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| name.clone()),
                root: String::new(),
                title: name.clone(),
                detail: String::from("git difftool · two files"),
                sentence: String::from("git difftool · two files"),
                branch: None,
                // The two paths as git gave them.
                old_label: old.display().to_string(),
                new_label: new.display().to_string(),
                grouped: false,
                live: false,
                reduced: true,
                single_file: true,
                git_dir: String::new(),
                has_history: false,
                history_from: None,
            },
            Source::Folders { .. } => ChangesHeader {
                repository: String::from("git difftool"),
                root: String::new(),
                title: String::from("git difftool"),
                detail: String::from("Two folders, from --dir-diff"),
                sentence: String::from("git difftool · two folders"),
                branch: None,
                old_label: String::from("Before"),
                new_label: String::from("After"),
                grouped: false,
                live: false,
                reduced: true,
                single_file: false,
                git_dir: String::new(),
                has_history: false,
                history_from: None,
            },
        }
    }

    /// The files that differ, read again.
    pub fn files(&self) -> Result<Vec<ChangesFile>, ScryError> {
        let files = match &self.source {
            Source::Repository {
                git,
                root,
                scope,
                paths,
                ..
            } => diff::changed_files(git, root, &scope.comparison, paths)?,
            Source::Files {
                git,
                old,
                new,
                name,
            } => {
                // One entry, with the counts of its own diff.
                let read = diff::plain_files(git, old, new, Marks::Auto).map_err(diff_error)?;
                let mut file = diff::ChangedFile {
                    path: name.clone(),
                    old_path: None,
                    name: Path::new(name)
                        .file_name()
                        .map(|name| name.to_string_lossy().into_owned())
                        .unwrap_or_else(|| name.clone()),
                    directory: match name.rfind('/') {
                        Some(slash) => name[..=slash].to_string(),
                        None => String::new(),
                    },
                    state: diff::FileState::Modified,
                    binary: false,
                    submodule: false,
                    added: None,
                    removed: None,
                    group: None,
                    partly_staged: false,
                    old_id: None,
                    new_id: None,
                };
                match read {
                    diff::FileDiff::Text(text) => {
                        let (added, removed) = text.counts();
                        file.added = Some(added);
                        file.removed = Some(removed);
                    }
                    diff::FileDiff::Binary { .. } => file.binary = true,
                    diff::FileDiff::Submodule | diff::FileDiff::Conflicted => {}
                }
                vec![file]
            }
            Source::Folders { git, old, new } => {
                diff::folder_files(git, old, new).map_err(diff_error)?
            }
            Source::Point {
                git, root, sides, ..
            } => match sides {
                Some(sides) => history::sides_file(git, root, sides)?.into_iter().collect(),
                None => Vec::new(),
            },
        };
        let marks = self.marks();
        let listed: Vec<Listed> = files
            .into_iter()
            .map(|file| {
                let id = match &self.source {
                    Source::Repository {
                        git, root, scope, ..
                    } => diff::content_id(git, root, &scope.comparison, &file),
                    _ => diff::ContentId::default(),
                };
                Listed { file, id }
            })
            .collect();
        let infos = listed
            .iter()
            .enumerate()
            .map(|(index, listed)| {
                let state = marks.as_ref().map_or(viewed::State::NotViewed, |marks| {
                    marks.state(&listed.file.path, &listed.id)
                });
                changed_file(index, listed, state)
            })
            .collect();
        *self.lock() = listed;
        Ok(infos)
    }

    /// Marks the file at `index` of the last list viewed, with the
    /// content it had when listed, or removes its mark.
    pub fn set_viewed(&self, index: u32, viewed: bool) -> Result<(), ScryError> {
        let Some(mut marks) = self.marks() else {
            return Err(ScryError::Refused {
                message: String::from("Files cannot be marked viewed in this window."),
            });
        };
        let listed = self.lock().clone();
        let Some(file) = listed.get(index as usize) else {
            return Err(ScryError::Refused {
                message: String::from("This file is no longer in the list."),
            });
        };
        // With paths asked, the list is not the whole change: the marks
        // of the files left out are kept.
        let whole = matches!(&self.source, Source::Repository { paths, .. } if paths.is_empty());
        let paths: Vec<&str> = listed
            .iter()
            .map(|listed| listed.file.path.as_str())
            .collect();
        marks
            .set(
                &file.file.path,
                viewed.then_some(&file.id),
                whole.then_some(paths.as_slice()),
            )
            .map_err(failed)
    }

    /// The diff of the file at `index` of the last list. A large file
    /// comes without colours and word marks unless `all_marks` asks for
    /// them.
    pub fn file_diff(&self, index: u32, all_marks: bool) -> Result<FileDiffInfo, ScryError> {
        let Some(file) = self
            .lock()
            .get(index as usize)
            .map(|listed| listed.file.clone())
        else {
            return Err(ScryError::Refused {
                message: String::from("This file is no longer in the list."),
            });
        };
        let marks = if all_marks {
            Marks::Always
        } else {
            Marks::Auto
        };
        let read = match &self.source {
            Source::Repository {
                git, root, scope, ..
            } => diff::file_diff(git, root, &scope.comparison, &file, marks),
            Source::Files { git, old, new, .. } => diff::plain_files(git, old, new, marks),
            Source::Folders { git, old, new } => {
                diff::folder_file_diff(git, old, new, &file, marks)
            }
            Source::Point {
                git,
                root,
                sides: Some(sides),
                ..
            } => history::sides_diff(git, root, sides, &file, marks),
            Source::Point { .. } => {
                return Err(ScryError::Refused {
                    message: String::from("This file is no longer in the list."),
                });
            }
        }
        .map_err(diff_error)?;
        Ok(file_diff(read, Path::new(&file.path)))
    }
}

#[uniffi::export]
impl ChangeSet {
    /// The blame of the new side of the file at `index` of the last
    /// list, not read yet. Refused where there is nothing to blame: no
    /// repository (`git difftool`), or a file with no new side.
    pub fn blame(&self, index: u32) -> Result<Arc<FileBlame>, ScryError> {
        let nothing = || ScryError::Refused {
            message: String::from("There is no history to read for this file."),
        };
        let file = self
            .lock()
            .get(index as usize)
            .map(|listed| listed.file.clone())
            .ok_or_else(nothing)?;
        if file.state == diff::FileState::Deleted {
            return Err(nothing());
        }
        let (git, target, path) = match &self.source {
            Source::Repository { git, scope, .. } => {
                (git, Target::of(&scope.comparison), file.path.clone())
            }
            Source::Point {
                git,
                sides: Some(sides),
                ..
            } => match sides {
                history::Sides::Compared { comparison, .. } => {
                    (git, Target::of(comparison), file.path.clone())
                }
                history::Sides::Blobs {
                    new: None,
                    new_path,
                    ..
                } => (git, Target::WorkingCopy, new_path.clone()),
                history::Sides::Blobs {
                    new: Some(blob),
                    new_path,
                    ..
                } => {
                    // `commit:path`: the commit is what comes before the
                    // path.
                    let commit = blob
                        .strip_suffix(new_path.as_str())
                        .and_then(|commit| commit.strip_suffix(':'))
                        .ok_or_else(nothing)?;
                    (git, Target::Commit(commit.to_string()), new_path.clone())
                }
            },
            _ => return Err(nothing()),
        };
        Ok(FileBlame::new(git.clone(), target, path))
    }
}

impl ChangeSet {
    /// One file at a point of its history, read as `sides` says.
    pub(crate) fn point(
        git: GitCli,
        root: PathBuf,
        sides: Option<history::Sides>,
        header: ChangesHeader,
    ) -> Arc<ChangeSet> {
        Arc::new(ChangeSet {
            source: Source::Point {
                git,
                root,
                sides,
                header,
            },
            files: Mutex::new(Vec::new()),
        })
    }

    /// This change's "Viewed" marks, read again; `None` without a
    /// repository.
    fn marks(&self) -> Option<Viewed> {
        match &self.source {
            Source::Repository { git_dir, scope, .. } if !git_dir.as_os_str().is_empty() => {
                Some(Viewed::load(git_dir, &scope.key))
            }
            _ => None,
        }
    }

    fn lock(&self) -> MutexGuard<'_, Vec<Listed>> {
        // The list is replaced whole: a panic elsewhere leaves it usable.
        self.files
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

fn changed_file(index: usize, listed: &Listed, state: viewed::State) -> ChangesFile {
    let file = &listed.file;
    ChangesFile {
        content_id: format!("{}>{}", listed.id.old, listed.id.new),
        viewed: match state {
            viewed::State::NotViewed => ViewedState::NotViewed,
            viewed::State::Viewed => ViewedState::Viewed,
            viewed::State::ChangedSince => ViewedState::ChangedSince,
        },
        index: index as u32,
        path: file.path.clone(),
        old_path: file.old_path.clone(),
        name: file.name.clone(),
        directory: file.directory.clone(),
        state: match file.state {
            diff::FileState::Added => ChangeState::Added,
            diff::FileState::Modified => ChangeState::Modified,
            diff::FileState::Deleted => ChangeState::Deleted,
            diff::FileState::Renamed => ChangeState::Renamed,
            diff::FileState::Untracked => ChangeState::Untracked,
            diff::FileState::TypeChanged => ChangeState::TypeChanged,
            diff::FileState::Conflicted => ChangeState::Conflicted,
        },
        binary: file.binary,
        submodule: file.submodule,
        added: file.added.map(|count| count as u32),
        removed: file.removed.map(|count| count as u32),
        group: file.group.map(|group| match group {
            diff::Group::Staged => ChangeGroup::Staged,
            diff::Group::NotStaged => ChangeGroup::NotStaged,
            diff::Group::Untracked => ChangeGroup::Untracked,
        }),
        partly_staged: file.partly_staged,
    }
}

fn ranges(words: Vec<std::ops::Range<usize>>) -> Vec<ByteRange> {
    words
        .into_iter()
        .map(|range| ByteRange {
            start: range.start as u32,
            end: range.end as u32,
        })
        .collect()
}

fn number(number: Option<usize>) -> Option<u32> {
    number.map(|number| number as u32)
}

fn file_diff(read: diff::FileDiff, path: &Path) -> FileDiffInfo {
    let text = match read {
        diff::FileDiff::Text(text) => text,
        diff::FileDiff::Binary { old_size, new_size } => {
            return FileDiffInfo::Binary { old_size, new_size };
        }
        diff::FileDiff::Submodule => return FileDiffInfo::Submodule,
        diff::FileDiff::Conflicted => return FileDiffInfo::Conflicted,
    };
    let (added, removed) = text.counts();
    let shown = text.shown(path);
    let lines = |lines: Vec<diff::ShownLine>| -> Vec<DiffLine> {
        lines
            .into_iter()
            .map(|line| DiffLine {
                text: line.text,
                columns: line.columns as u32,
                spans: line.spans.into_iter().map(span).collect(),
            })
            .collect()
    };
    FileDiffInfo::Text {
        diff: TextDiffInfo {
            old_lines: lines(shown.old_lines),
            new_lines: lines(shown.new_lines),
            rows: shown
                .rows
                .into_iter()
                .map(|row| DiffRow {
                    kind: match row.kind {
                        diff::RowKind::Unchanged => DiffRowKind::Unchanged,
                        diff::RowKind::Added => DiffRowKind::Added,
                        diff::RowKind::Removed => DiffRowKind::Removed,
                        diff::RowKind::Changed => DiffRowKind::Changed,
                    },
                    old: number(row.old),
                    new: number(row.new),
                    old_words: ranges(row.old_words),
                    new_words: ranges(row.new_words),
                })
                .collect(),
            unified: shown
                .unified
                .into_iter()
                .map(|line| UnifiedDiffLine {
                    kind: match line.kind {
                        diff::LineKind::Unchanged => UnifiedKind::Unchanged,
                        diff::LineKind::Added => UnifiedKind::Added,
                        diff::LineKind::Removed => UnifiedKind::Removed,
                    },
                    old: number(line.old),
                    new: number(line.new),
                    words: ranges(line.words),
                })
                .collect(),
            changes: shown
                .changes
                .into_iter()
                .map(|change| DiffChange {
                    first_row: change.first_row as u32,
                    row_count: change.row_count as u32,
                    first_line: change.first_line as u32,
                    added: change.added as u32,
                    removed: change.removed as u32,
                })
                .collect(),
            longest_line: shown.longest_line as u32,
            added: added as u32,
            removed: removed as u32,
            old_missing_final_newline: text.old_missing_final_newline,
            new_missing_final_newline: text.new_missing_final_newline,
            large: text.marks_skipped,
            language: Language::from_path(path).map(|language| language.name().to_string()),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::process::Command;

    /// A repository with one commit, in a folder of its own.
    fn repository(files: &[(&str, &str)]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for (path, content) in files {
            fs::write(dir.path().join(path), content).unwrap();
        }
        for args in [
            &["init", "--quiet", "--initial-branch=main"][..],
            &["add", "."],
            &[
                "-c",
                "user.name=Scry Test",
                "-c",
                "user.email=test@example.com",
                "-c",
                "commit.gpgsign=false",
                "commit",
                "--quiet",
                "-m",
                "base",
            ],
        ] {
            let status = Command::new("git")
                .current_dir(dir.path())
                .args(args)
                .status();
            assert!(status.unwrap().success(), "git {args:?}");
        }
        dir
    }

    /// The window gets, from any folder of the repository, what is
    /// compared in words, the files with their place in the list, and a
    /// file's diff ready to draw in both layouts.
    #[test]
    fn a_change_set_lists_files_and_gives_a_diff_ready_to_draw() {
        let dir = repository(&[("a.rs", "fn main() {\n\tlet a = 1;\n}\n")]);
        fs::write(dir.path().join("a.rs"), "fn main() {\n\tlet a = 2;\n}\n").unwrap();
        fs::write(dir.path().join("new.txt"), "x\n").unwrap();
        fs::create_dir(dir.path().join("sub")).unwrap();

        let path = dir.path().join("sub").display().to_string();
        let changes = ChangeSet::open(path, false, None, Vec::new()).unwrap();
        let header = changes.header();
        assert_eq!(header.sentence, "Not committed, against HEAD");
        assert_eq!(header.branch.as_deref(), Some("main"));
        assert_eq!(
            (header.old_label.as_str(), header.new_label.as_str()),
            ("HEAD", "Working copy")
        );
        assert!(header.grouped && header.live);
        assert!(!header.reduced && !header.single_file);

        let files = changes.files().unwrap();
        let seen: Vec<_> = files
            .iter()
            .map(|file| (file.index, file.path.as_str(), file.state, file.group))
            .collect();
        assert_eq!(
            seen,
            [
                (
                    0,
                    "a.rs",
                    ChangeState::Modified,
                    Some(ChangeGroup::NotStaged)
                ),
                (
                    1,
                    "new.txt",
                    ChangeState::Untracked,
                    Some(ChangeGroup::Untracked)
                ),
            ]
        );
        assert_eq!((files[0].added, files[0].removed), (Some(1), Some(1)));

        let FileDiffInfo::Text { diff } = changes.file_diff(0, false).unwrap() else {
            panic!("not a text diff");
        };
        assert_eq!((diff.added, diff.removed), (1, 1));
        assert_eq!(diff.language.as_deref(), Some("Rust"));
        assert_eq!(diff.new_lines[1].text, "    let a = 2;");
        assert_eq!(diff.rows[1].kind, DiffRowKind::Changed);
        let word = &diff.rows[1].new_words[0];
        assert_eq!(
            &diff.new_lines[1].text[word.start as usize..word.end as usize],
            "2"
        );
        assert!(!diff.new_lines[1].spans.is_empty());
        let kinds: Vec<_> = diff.unified.iter().map(|line| line.kind).collect();
        assert_eq!(
            kinds,
            [
                UnifiedKind::Unchanged,
                UnifiedKind::Removed,
                UnifiedKind::Added,
                UnifiedKind::Unchanged
            ]
        );
        assert_eq!(diff.changes.len(), 1);
        assert_eq!(
            (diff.changes[0].first_row, diff.changes[0].first_line),
            (1, 1)
        );
    }

    /// A file marked viewed stays so until its content changes, then is
    /// "changed since viewed"; the marks live under the git folder, per
    /// scope, and a window on two files has none.
    #[test]
    fn a_file_is_marked_viewed_until_it_changes() {
        let dir = repository(&[("a.rs", "one\n"), ("b.rs", "one\n")]);
        fs::write(dir.path().join("a.rs"), "two\n").unwrap();
        fs::write(dir.path().join("b.rs"), "two\n").unwrap();
        let path = dir.path().display().to_string();
        let changes = ChangeSet::open(path.clone(), false, None, Vec::new()).unwrap();
        assert!(changes.header().git_dir.ends_with(".git"));
        let files = changes.files().unwrap();
        assert_eq!(files[0].viewed, ViewedState::NotViewed);

        changes.set_viewed(0, true).unwrap();
        let viewed = changes.files().unwrap();
        assert_eq!(viewed[0].viewed, ViewedState::Viewed);
        assert_eq!(viewed[1].viewed, ViewedState::NotViewed);
        assert_eq!(viewed[0].content_id, files[0].content_id);
        assert!(dir.path().join(".git/scry/viewed.json").is_file());
        // Nothing is written to the working folder.
        assert_eq!(viewed.len(), 2);

        // Another window on the same change reads the mark; another
        // scope does not.
        let other = ChangeSet::open(path.clone(), false, None, Vec::new()).unwrap();
        assert_eq!(other.files().unwrap()[0].viewed, ViewedState::Viewed);
        let against = ChangeSet::open(path, false, Some(String::from("HEAD")), Vec::new()).unwrap();
        assert_eq!(against.files().unwrap()[0].viewed, ViewedState::NotViewed);

        fs::write(dir.path().join("a.rs"), "three\n").unwrap();
        let changed = changes.files().unwrap();
        assert_eq!(changed[0].viewed, ViewedState::ChangedSince);
        assert_ne!(changed[0].content_id, files[0].content_id);
        changes.set_viewed(0, true).unwrap();
        assert_eq!(changes.files().unwrap()[0].viewed, ViewedState::Viewed);
        changes.set_viewed(0, false).unwrap();
        assert_eq!(changes.files().unwrap()[0].viewed, ViewedState::NotViewed);
        assert!(matches!(
            changes.set_viewed(9, true),
            Err(ScryError::Refused { .. })
        ));

        let two = ChangeSet::open_difftool(
            dir.path().join("a.rs").display().to_string(),
            dir.path().join("b.rs").display().to_string(),
            String::new(),
        );
        two.files().unwrap();
        assert!(two.header().git_dir.is_empty());
        assert!(matches!(
            two.set_viewed(0, true),
            Err(ScryError::Refused { .. })
        ));
    }

    /// A range has no groups and cannot change under the window; a file
    /// asked by an index the list no longer has and `--staged` with a
    /// range are refused with a sentence to show.
    #[test]
    fn a_change_set_refuses_what_it_cannot_read() {
        let dir = repository(&[("a.txt", "one\n")]);
        let path = dir.path().display().to_string();
        let range = ChangeSet::open(
            path.clone(),
            false,
            Some(String::from("HEAD..HEAD")),
            Vec::new(),
        )
        .unwrap();
        let header = range.header();
        assert!(!header.grouped && !header.live);
        assert_eq!(header.sentence, "HEAD, against HEAD");
        assert!(range.files().unwrap().is_empty());
        assert!(matches!(
            range.file_diff(0, false),
            Err(ScryError::Refused { .. })
        ));

        let staged = ChangeSet::open(path, true, Some(String::from("a..b")), Vec::new());
        match staged {
            Err(ScryError::Refused { message }) => {
                assert_eq!(message, "--staged takes one commit, not a range");
            }
            _ => panic!("--staged with a range was accepted"),
        }
    }

    /// `git difftool` gives two files: the window gets one file under the
    /// name git gave, compared by the two paths as they came, with no
    /// repository behind it. With `--dir-diff` it gives two folders, and
    /// the files that differ are listed.
    #[test]
    fn a_difftool_change_set_reads_two_files_or_two_folders() {
        let dir = tempfile::tempdir().unwrap();
        let path = |name: &str| dir.path().join(name).display().to_string();
        fs::write(dir.path().join("old.rs"), "let a = 1;\n").unwrap();
        fs::write(dir.path().join("new.rs"), "let a = 2;\nlet b = 3;\n").unwrap();

        let files =
            ChangeSet::open_difftool(path("old.rs"), path("new.rs"), String::from("src/a.rs"));
        let header = files.header();
        assert!(header.reduced && header.single_file && !header.live && !header.grouped);
        assert_eq!(header.title, "src/a.rs");
        assert_eq!(header.repository, "a.rs");
        assert_eq!(header.old_label, path("old.rs"));
        let list = files.files().unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(
            (list[0].name.as_str(), list[0].directory.as_str()),
            ("a.rs", "src/")
        );
        assert_eq!((list[0].added, list[0].removed), (Some(2), Some(1)));
        let FileDiffInfo::Text { diff } = files.file_diff(0, false).unwrap() else {
            panic!("not a text diff");
        };
        // Coloured by the name git gave, not by the temporary file's.
        assert_eq!(diff.language.as_deref(), Some("Rust"));
        assert_eq!((diff.added, diff.removed), (2, 1));

        // Without a name, the new file's own.
        let unnamed = ChangeSet::open_difftool(path("old.rs"), path("new.rs"), String::new());
        assert_eq!(unnamed.header().title, "new.rs");

        for folder in ["left", "right"] {
            fs::create_dir(dir.path().join(folder)).unwrap();
        }
        fs::write(dir.path().join("left/a.txt"), "one\n").unwrap();
        fs::write(dir.path().join("right/a.txt"), "two\n").unwrap();
        fs::write(dir.path().join("right/b.txt"), "new\n").unwrap();
        let folders = ChangeSet::open_difftool(path("left"), path("right"), String::new());
        let header = folders.header();
        assert!(header.reduced && !header.single_file);
        assert_eq!(
            (header.old_label.as_str(), header.new_label.as_str()),
            ("Before", "After")
        );
        let list = folders.files().unwrap();
        let seen: Vec<_> = list
            .iter()
            .map(|file| (file.path.as_str(), file.state))
            .collect();
        assert_eq!(
            seen,
            [
                ("a.txt", ChangeState::Modified),
                ("b.txt", ChangeState::Added)
            ]
        );
        assert!(matches!(
            folders.file_diff(1, false).unwrap(),
            FileDiffInfo::Text { .. }
        ));
    }

    /// The list of recent repositories says what waits in each: files
    /// not committed, a merge that stopped on conflicts, nothing; and a
    /// folder that is gone, or is not a repository, is told apart.
    #[test]
    fn a_repository_says_what_waits_in_it() {
        let dir = repository(&[("a.txt", "base\n")]);
        let path = dir.path().display().to_string();
        assert_eq!(repository_waiting(path.clone()), RepositoryWaiting::Nothing);

        fs::write(dir.path().join("a.txt"), "changed\n").unwrap();
        fs::write(dir.path().join("new.txt"), "x\n").unwrap();
        assert_eq!(
            repository_waiting(path.clone()),
            RepositoryWaiting::NotCommitted { files: 2 }
        );

        let git = |args: &[&str]| {
            Command::new("git")
                .current_dir(dir.path())
                .args([
                    "-c",
                    "user.name=Scry Test",
                    "-c",
                    "user.email=test@example.com",
                ])
                .args(["-c", "commit.gpgsign=false"])
                .args(args)
                .output()
                .unwrap()
                .status
                .success()
        };
        fs::remove_file(dir.path().join("new.txt")).unwrap();
        assert!(git(&["switch", "--quiet", "-c", "feature"]));
        assert!(git(&["commit", "--quiet", "-am", "feature"]));
        assert!(git(&["switch", "--quiet", "main"]));
        fs::write(dir.path().join("a.txt"), "main\n").unwrap();
        assert!(git(&["commit", "--quiet", "-am", "main"]));
        assert!(!git(&["merge", "--quiet", "feature"]));
        match repository_waiting(path) {
            RepositoryWaiting::Resolving {
                headline,
                unresolved,
            } => {
                assert_eq!(unresolved, 1);
                assert!(headline.contains("feature"), "{headline}");
            }
            other => panic!("{other:?}"),
        }

        let gone = dir.path().join("no-such-folder").display().to_string();
        assert_eq!(repository_waiting(gone), RepositoryWaiting::Missing);
    }
}
