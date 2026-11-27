//! Reading a change: the files that differ between two versions of a
//! repository, and one file's rows, old beside new.
//!
//! Git finds the changed lines (`git diff`), so the blocks are git's own
//! and the user's `diff.algorithm` is honoured; inside a block, `words`
//! pairs the lines and marks the changed words. Nothing is ever written:
//! a diff is read only.
//!
//! Every function expects `git` to run at the repository's root, and paths
//! relative to it, as git prints them.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::fs;
use std::io;
use std::ops::Range;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::git::{Git, GitError};
use crate::highlight::{Highlighter, Span};
use crate::layout::Shown;
use crate::syntax::Language;
use crate::words;

/// What is compared: the old side and the new side.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Comparison {
    /// The working copy against a commit, untracked files included: what
    /// is not committed, when the commit is HEAD.
    WorkingCopy { base: String },
    /// The index against a commit: what the next commit holds.
    Staged { base: String },
    /// The working copy against the index.
    Unstaged,
    /// One commit, or tree, against another.
    Commits { old: String, new: String },
}

impl Comparison {
    /// Everything not committed: the working copy against HEAD, or against
    /// nothing in a repository with no commit yet.
    pub fn uncommitted(git: &dyn Git) -> Result<Comparison, GitError> {
        Ok(Comparison::WorkingCopy { base: head(git)? })
    }

    /// What is staged: the index against HEAD.
    pub fn staged(git: &dyn Git) -> Result<Comparison, GitError> {
        Ok(Comparison::Staged { base: head(git)? })
    }

    /// `new` against its merge base with `old` (`old...new`): what the
    /// branch `new` changed on its own.
    pub fn since_merge_base(git: &dyn Git, old: &str, new: &str) -> Result<Comparison, GitError> {
        let base = git.run(&["merge-base", old, new])?;
        Ok(Comparison::Commits {
            old: base.trim().to_string(),
            new: new.to_string(),
        })
    }

    /// The arguments of `git diff` that select the two sides.
    fn sides(&self) -> Vec<&str> {
        match self {
            Comparison::WorkingCopy { base } => vec![base],
            Comparison::Staged { base } => vec!["--cached", base],
            Comparison::Unstaged => Vec::new(),
            Comparison::Commits { old, new } => vec![old, new],
        }
    }

    /// The old side's blob for `path`, as `git cat-file` names it.
    fn old_blob(&self, path: &str) -> String {
        match self {
            Comparison::WorkingCopy { base } | Comparison::Staged { base } => {
                format!("{base}:{path}")
            }
            Comparison::Unstaged => format!(":0:{path}"),
            Comparison::Commits { old, .. } => format!("{old}:{path}"),
        }
    }

    /// The new side's blob for `path`; `None` when it is the file on disk.
    fn new_blob(&self, path: &str) -> Option<String> {
        match self {
            Comparison::WorkingCopy { .. } | Comparison::Unstaged => None,
            Comparison::Staged { .. } => Some(format!(":0:{path}")),
            Comparison::Commits { new, .. } => Some(format!("{new}:{path}")),
        }
    }
}

/// A comparison with the words that name it: what the window's header and
/// the bar above the file say.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scope {
    pub comparison: Comparison,
    /// What was asked, in a fixed form: "uncommitted", "staged",
    /// "main..HEAD". The "Viewed" marks are kept under it.
    pub key: String,
    /// What is read: "Not committed", "Staged", a branch or a commit.
    pub title: String,
    /// What it is compared with: "Against HEAD".
    pub detail: String,
    /// The branch checked out, when the working copy or the index is
    /// read; `None` on a detached HEAD.
    pub branch: Option<String>,
    /// The old side's name, for the bar above the file: "HEAD".
    pub old_label: String,
    /// The new side's name: "Working copy".
    pub new_label: String,
}

/// Why what was asked cannot be compared.
#[derive(Debug)]
pub enum ScopeError {
    Git(GitError),
    /// `--staged` with `a..b`: the index is compared with one commit.
    StagedRange,
}

impl fmt::Display for ScopeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ScopeError::Git(error) => error.fmt(f),
            ScopeError::StagedRange => f.write_str("--staged takes one commit, not a range"),
        }
    }
}

impl std::error::Error for ScopeError {}

impl From<GitError> for ScopeError {
    fn from(error: GitError) -> ScopeError {
        ScopeError::Git(error)
    }
}

impl Scope {
    /// What `git scry diff` compares, from its arguments: nothing for
    /// everything not committed, a commit for the working copy against
    /// it, `a..b` for two commits, `a...b` for b against its merge base
    /// with a; `staged` reads the index in place of the working copy. A
    /// missing end of a range is HEAD, as for git.
    pub fn of(git: &dyn Git, staged: bool, revisions: Option<&str>) -> Result<Scope, ScopeError> {
        let or_head = |name: &str| if name.is_empty() { "HEAD" } else { name }.to_string();
        if let Some((old, new)) = revisions.and_then(|revisions| revisions.split_once("...")) {
            if staged {
                return Err(ScopeError::StagedRange);
            }
            let (old, new) = (or_head(old), or_head(new));
            let comparison = Comparison::since_merge_base(git, &old, &new)?;
            let base = match &comparison {
                Comparison::Commits { old, .. } => short_id(git, old),
                _ => String::new(),
            };
            return Ok(Scope {
                comparison,
                key: format!("{old}...{new}"),
                detail: format!("Against its merge base with {old} ({base})"),
                branch: None,
                old_label: base,
                new_label: new.clone(),
                title: new,
            });
        }
        if let Some((old, new)) = revisions.and_then(|revisions| revisions.split_once("..")) {
            if staged {
                return Err(ScopeError::StagedRange);
            }
            let (old, new) = (or_head(old), or_head(new));
            return Ok(Scope {
                comparison: Comparison::Commits {
                    old: old.clone(),
                    new: new.clone(),
                },
                key: format!("{old}..{new}"),
                detail: format!("Against {old}"),
                branch: None,
                old_label: old,
                new_label: new.clone(),
                title: new,
            });
        }
        // The working copy or the index, against one commit.
        let (base, name) = match revisions {
            Some(base) => (base.to_string(), base.to_string()),
            None => {
                let base = head(git)?;
                let name = if base == "HEAD" {
                    base.clone()
                } else {
                    String::from("nothing: no commit yet")
                };
                (base, name)
            }
        };
        let branch = git
            .run(&["branch", "--show-current"])
            .ok()
            .map(|branch| branch.trim().to_string())
            .filter(|branch| !branch.is_empty());
        let (comparison, title, new_label) = if staged {
            (Comparison::Staged { base }, "Staged", "Index")
        } else if revisions.is_none() {
            (
                Comparison::WorkingCopy { base },
                "Not committed",
                "Working copy",
            )
        } else {
            (
                Comparison::WorkingCopy { base },
                "Working copy",
                "Working copy",
            )
        };
        let side = if staged { "staged" } else { "uncommitted" };
        Ok(Scope {
            comparison,
            key: match revisions {
                Some(base) => format!("{side}:{base}"),
                None => side.to_string(),
            },
            title: title.to_string(),
            detail: format!("Against {name}"),
            branch,
            old_label: name,
            new_label: new_label.to_string(),
        })
    }

    /// The title and the detail in one line: "Not committed, against
    /// HEAD".
    pub fn sentence(&self) -> String {
        let mut detail = self.detail.chars();
        let lowered: String = match detail.next() {
            Some(first) => first.to_lowercase().chain(detail).collect(),
            None => String::new(),
        };
        format!("{}, {lowered}", self.title)
    }
}

/// A commit's abbreviated id, or its name as given when git cannot tell.
fn short_id(git: &dyn Git, commit: &str) -> String {
    git.run(&["rev-parse", "--short", commit])
        .map(|id| id.trim().to_string())
        .unwrap_or_else(|_| commit.to_string())
}

/// HEAD, or the empty tree while the repository has no commit: everything
/// is then new.
fn head(git: &dyn Git) -> Result<String, GitError> {
    let output = git.output(&["rev-parse", "--verify", "--quiet", "HEAD^{commit}"])?;
    if output.status == Some(0) {
        return Ok(String::from("HEAD"));
    }
    // The empty tree's id depends on the repository's hash; nothing is
    // written to compute it.
    let tree = git.run_with_input(&["hash-object", "-t", "tree", "--stdin"], b"")?;
    Ok(tree.trim().to_string())
}

/// What happened to a file between the two sides.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum FileState {
    Added,
    Modified,
    Deleted,
    /// Moved or renamed, with or without changes; `old_path` is set.
    Renamed,
    /// New and not known to git yet.
    Untracked,
    /// A file that became a link, or the reverse.
    TypeChanged,
    /// Left with conflicts by a merge in progress: it has no diff to
    /// show, the merge editor resolves it.
    Conflicted,
}

/// Where a file of the working copy's change stands with the index.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Group {
    /// Every change of the file is staged.
    Staged,
    /// Some or all of its changes are not staged.
    NotStaged,
    Untracked,
}

/// A file that differs between the two sides.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ChangedFile {
    pub path: String,
    /// The path on the old side, for a renamed file.
    pub old_path: Option<String>,
    /// The last component of the path.
    pub name: String,
    /// The folder, with a trailing slash; empty at the root.
    pub directory: String,
    pub state: FileState,
    pub binary: bool,
    /// A repository inside the repository: it has commits, not lines.
    pub submodule: bool,
    /// Lines added and removed; `None` when they are not counted (a
    /// binary file, a conflicted one, an untracked file too large to
    /// read for the list).
    pub added: Option<usize>,
    pub removed: Option<usize>,
    /// Only for `Comparison::WorkingCopy`.
    pub group: Option<Group>,
    /// Listed under `NotStaged` with part of its changes staged.
    pub partly_staged: bool,
    /// The ids git gives the file's content on each side; `None` for a
    /// side the file is not on, or that git has not hashed (the file on
    /// disk).
    #[serde(skip)]
    pub old_id: Option<String>,
    #[serde(skip)]
    pub new_id: Option<String>,
}

/// An untracked file larger than this is listed without its line count:
/// the list must not read a build folder someone forgot to ignore.
const MAX_COUNTED_UNTRACKED: u64 = 2_000_000;

/// How many bytes tell a binary file from text, as git does: a NUL among
/// the first of them.
const BINARY_PROBE: usize = 8_000;

/// The options every `git diff` gets, so that the user's configuration
/// changes the algorithm and nothing else: no colours, no external tool,
/// no text conversion, paths from the root, renames found.
const DIFF_OPTIONS: [&str; 6] = [
    "diff",
    "--no-color",
    "--no-ext-diff",
    "--no-textconv",
    "--no-relative",
    "--find-renames",
];

/// The files that differ in `comparison`, limited to `paths` (files or
/// folders) when there are some. For the working copy they come grouped:
/// staged, not staged, untracked; inside a group, by path.
///
/// A file whose changes are partly staged is listed once, under not
/// staged: its diff is the whole difference with the commit.
pub fn changed_files(
    git: &dyn Git,
    root: &Path,
    comparison: &Comparison,
    paths: &[String],
) -> Result<Vec<ChangedFile>, GitError> {
    let mut files = tracked_changes(git, comparison, paths)?;
    if let Comparison::WorkingCopy { base } = comparison {
        let staged = changed_paths(git, &["--cached", base], paths)?;
        // Against a commit, git calls a file left with conflicts
        // modified; against the index it says which ones they are.
        let against_index = tracked_changes(git, &Comparison::Unstaged, paths)?;
        let conflicted: HashSet<&str> = against_index
            .iter()
            .filter(|file| file.state == FileState::Conflicted)
            .map(|file| file.path.as_str())
            .collect();
        for file in &mut files {
            if conflicted.contains(file.path.as_str()) {
                *file = changed_file(&file.path, FileState::Conflicted);
            }
        }
        let unstaged: HashSet<String> =
            against_index.iter().map(|file| file.path.clone()).collect();
        // Staged, then put back by hand: nothing differs from the commit,
        // but the index still holds a change the next commit would take.
        let listed: HashSet<String> = files.iter().map(|file| file.path.clone()).collect();
        for path in staged.intersection(&unstaged) {
            if !listed.contains(path) {
                files.push(ChangedFile {
                    added: Some(0),
                    removed: Some(0),
                    ..changed_file(path, FileState::Modified)
                });
            }
        }
        for file in &mut files {
            let is_staged = staged.contains(&file.path);
            let is_unstaged = unstaged.contains(&file.path);
            file.group = Some(if is_unstaged || !is_staged {
                Group::NotStaged
            } else {
                Group::Staged
            });
            // A conflicted file is in neither state: it is unresolved.
            file.partly_staged = is_staged && is_unstaged && file.state != FileState::Conflicted;
        }
        files.extend(untracked_files(git, root, paths)?);
    }
    files.sort_by(|a, b| (a.group, &a.path).cmp(&(b.group, &b.path)));
    Ok(files)
}

/// A file with nothing known but its path and state.
fn changed_file(path: &str, state: FileState) -> ChangedFile {
    let (directory, name) = match path.rfind('/') {
        Some(slash) => (&path[..=slash], &path[slash + 1..]),
        None => ("", path),
    };
    ChangedFile {
        path: path.to_string(),
        old_path: None,
        name: name.to_string(),
        directory: directory.to_string(),
        state,
        binary: false,
        submodule: false,
        added: None,
        removed: None,
        group: None,
        partly_staged: false,
        old_id: None,
        new_id: None,
    }
}

/// `git diff` with the fixed options, `arguments`, then `paths`.
fn diff_arguments<'a>(arguments: &[&'a str], paths: &'a [String]) -> Vec<&'a str> {
    let mut all: Vec<&str> = DIFF_OPTIONS.to_vec();
    all.extend_from_slice(arguments);
    all.push("--");
    all.extend(paths.iter().map(String::as_str));
    all
}

/// The files git tracks that differ, with their states and counts.
fn tracked_changes(
    git: &dyn Git,
    comparison: &Comparison,
    paths: &[String],
) -> Result<Vec<ChangedFile>, GitError> {
    let mut arguments = vec!["--raw", "--no-abbrev", "--numstat", "-z"];
    arguments.extend(comparison.sides());
    let output = git.run(&diff_arguments(&arguments, paths))?;
    Ok(parse_changes(&output))
}

/// The paths that differ between two sides, renamed files under their new
/// path.
fn changed_paths(
    git: &dyn Git,
    sides: &[&str],
    paths: &[String],
) -> Result<HashSet<String>, GitError> {
    let mut arguments = vec!["--raw", "--no-abbrev", "--numstat", "-z"];
    arguments.extend_from_slice(sides);
    let output = git.run(&diff_arguments(&arguments, paths))?;
    Ok(parse_changes(&output)
        .into_iter()
        .map(|file| file.path)
        .collect())
}

/// The output of `git diff --raw --numstat -z`: first a record per file,
///
/// ```text
/// :100644 100644 1a2b3c4 5d6e7f8 M NUL path NUL
/// :100644 100644 1a2b3c4 5d6e7f8 R087 NUL old path NUL new path NUL
/// ```
///
/// then the counts, "added TAB removed TAB path NUL", or for a renamed
/// file "added TAB removed TAB NUL old path NUL new path NUL"; "-" counts
/// for a binary file.
pub fn parse_changes(text: &str) -> Vec<ChangedFile> {
    /// A gitlink: the mode of a submodule's entry.
    const SUBMODULE: &str = "160000";
    let mut tokens = text.split('\0').peekable();
    let mut files: Vec<ChangedFile> = Vec::new();
    let mut index: HashMap<String, usize> = HashMap::new();
    while let Some(record) = tokens.next_if(|token| token.starts_with(':')) {
        let mut fields = record[1..].split(' ');
        let old_mode = fields.next().unwrap_or("");
        let new_mode = fields.next().unwrap_or("");
        // An id of zeros stands for no content, or content not hashed.
        let mut id = || {
            fields
                .next()
                .filter(|id| id.contains(|digit| digit != '0'))
                .map(str::to_string)
        };
        let (old_id, new_id) = (id(), id());
        let status = fields.next().unwrap_or("");
        let Some(first) = tokens.next() else {
            break;
        };
        let renamed = status.starts_with('R') || status.starts_with('C');
        let (old_path, path) = if renamed {
            match tokens.next() {
                Some(second) => (Some(first), second),
                None => break,
            }
        } else {
            (None, first)
        };
        let state = match status.chars().next() {
            Some('A') => FileState::Added,
            Some('D') => FileState::Deleted,
            Some('R') => FileState::Renamed,
            Some('T') => FileState::TypeChanged,
            Some('U') => FileState::Conflicted,
            // A copy is a new file: git is not asked to find copies.
            Some('C') => FileState::Added,
            _ => FileState::Modified,
        };
        let file = ChangedFile {
            old_path: old_path
                .filter(|_| state == FileState::Renamed)
                .map(str::to_string),
            submodule: old_mode == SUBMODULE || new_mode == SUBMODULE,
            old_id,
            new_id,
            ..changed_file(path, state)
        };
        match index.get(path) {
            // A conflicted file comes twice, once as conflicted: that
            // state wins.
            Some(&at) => {
                if state == FileState::Conflicted {
                    files[at] = file;
                }
            }
            None => {
                index.insert(path.to_string(), files.len());
                files.push(file);
            }
        }
    }
    while let Some(entry) = tokens.next() {
        let mut parts = entry.splitn(3, '\t');
        let (Some(added), Some(removed), Some(path)) = (parts.next(), parts.next(), parts.next())
        else {
            continue;
        };
        // An empty path announces the two paths of a renamed file.
        let path = if path.is_empty() {
            tokens.next();
            match tokens.next() {
                Some(new_path) => new_path,
                None => break,
            }
        } else {
            path
        };
        let Some(file) = index.get(path).map(|&at| &mut files[at]) else {
            continue;
        };
        if file.state == FileState::Conflicted || file.submodule {
            continue;
        }
        file.binary = added == "-" && removed == "-";
        file.added = added.parse().ok();
        file.removed = removed.parse().ok();
    }
    files
}

/// The files git does not know and does not ignore, each with its line
/// count.
fn untracked_files(
    git: &dyn Git,
    root: &Path,
    paths: &[String],
) -> Result<Vec<ChangedFile>, GitError> {
    let mut arguments = vec![
        "ls-files",
        "--others",
        "--exclude-standard",
        "--full-name",
        "-z",
        "--",
    ];
    arguments.extend(paths.iter().map(String::as_str));
    let output = git.run(&arguments)?;
    Ok(output
        .split('\0')
        .filter(|path| !path.is_empty())
        .map(|path| {
            let mut file = ChangedFile {
                group: Some(Group::Untracked),
                ..changed_file(path.trim_end_matches('/'), FileState::Untracked)
            };
            // A folder with a repository of its own is listed as one entry.
            if path.ends_with('/') {
                file.submodule = true;
                return file;
            }
            let on_disk = root.join(path);
            let small = fs::metadata(&on_disk)
                .map(|metadata| metadata.is_file() && metadata.len() <= MAX_COUNTED_UNTRACKED)
                .unwrap_or(false);
            if small && let Ok(bytes) = fs::read(&on_disk) {
                if is_binary(&bytes) {
                    file.binary = true;
                } else {
                    file.added = Some(Text::of(&bytes).lines.len());
                    file.removed = Some(0);
                }
            }
            file
        })
        .collect())
}

/// What identifies a file's change: the content of its two sides. Equal
/// ids mean the same diff, so a "Viewed" mark holds while they are equal.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ContentId {
    /// Empty for a file with no old side.
    pub old: String,
    pub new: String,
}

/// A file on disk larger than this is identified by its size and date,
/// not read: the list is identified again at every refresh.
const MAX_HASHED: u64 = 16_000_000;

/// The id of `file`'s change in `comparison`. The index and commits are
/// named by git's own ids; the file on disk is read, whether staged or
/// not, so that staging a file does not change its id.
pub fn content_id(
    git: &dyn Git,
    root: &Path,
    comparison: &Comparison,
    file: &ChangedFile,
) -> ContentId {
    let old = file.old_id.clone().unwrap_or_default();
    let on_disk = matches!(
        comparison,
        Comparison::WorkingCopy { .. } | Comparison::Unstaged
    );
    let new = if on_disk {
        disk_id(git, &root.join(&file.path), &file.path)
    } else {
        file.new_id.clone().unwrap_or_default()
    };
    ContentId { old, new }
}

/// What identifies the content at `path` on disk: a link's target, a
/// repository's commit, a file's bytes; "gone" when nothing is there.
fn disk_id(git: &dyn Git, path: &Path, name: &str) -> String {
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return String::from("gone");
    };
    if metadata.is_symlink() {
        let target = fs::read_link(path).unwrap_or_default();
        return format!("link:{:016x}", fnv(target.to_string_lossy().as_bytes()));
    }
    if metadata.is_dir() {
        return match git.run(&["-C", name, "rev-parse", "HEAD"]) {
            Ok(commit) => format!("commit:{}", commit.trim()),
            Err(_) => String::from("folder"),
        };
    }
    let stamp = || {
        let modified = metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .map_or(0, |since| since.as_nanos());
        format!("stamp:{}:{modified}", metadata.len())
    };
    if metadata.len() > MAX_HASHED {
        return stamp();
    }
    match fs::read(path) {
        Ok(bytes) => format!("bytes:{}:{:016x}", bytes.len(), fnv(&bytes)),
        Err(_) => stamp(),
    }
}

/// FNV-1a, 64 bits: the same on every platform and version, which the
/// standard library's hasher does not promise. It tells an edited file
/// from the one that was viewed; it guards against nothing.
fn fnv(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

/// Whether git would call these bytes binary.
fn is_binary(bytes: &[u8]) -> bool {
    bytes[..bytes.len().min(BINARY_PROBE)].contains(&0)
}

/// What a row of a file's diff is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum RowKind {
    /// The same line on both sides.
    Unchanged,
    /// A line only the new side has.
    Added,
    /// A line only the old side has.
    Removed,
    /// An old line beside the new line that replaces it.
    Changed,
}

/// One row of a file's diff, as the split view shows it: the old line on
/// the left, the new line on the right, either one missing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Row {
    pub kind: RowKind,
    /// The line's number on the old side, from 1.
    pub old: Option<usize>,
    /// The line's number on the new side, from 1.
    pub new: Option<usize>,
    /// The parts of the old line that changed, in UTF-8 bytes from its
    /// start. Empty unless the two lines of a changed row are alike.
    pub old_words: Vec<Range<usize>>,
    /// The parts of the new line that changed.
    pub new_words: Vec<Range<usize>>,
}

/// What a line of the unified view is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum LineKind {
    Unchanged,
    Added,
    Removed,
}

/// One line of the unified view: in each block, the removed lines, then
/// the added ones.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UnifiedLine {
    pub kind: LineKind,
    pub old: Option<usize>,
    pub new: Option<usize>,
    /// The changed parts of the line, in UTF-8 bytes from its start.
    pub words: Vec<Range<usize>>,
}

/// A block of changed rows with unchanged lines, or the file's ends,
/// around it: what "next change" moves between.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Change {
    /// Index of its first row in `TextDiff::rows`.
    pub first_row: usize,
    pub row_count: usize,
    /// Index of its first line in `TextDiff::unified`, where it has
    /// `removed + added` lines.
    pub first_line: usize,
    pub added: usize,
    pub removed: usize,
}

/// Whether to pair the lines of a block by likeness and mark their
/// changed words.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Marks {
    /// Unless the file is large: see `TextDiff::marks_skipped`.
    Auto,
    /// Whatever the file's size: the user asked for this file.
    Always,
}

/// Above this many lines, old and new together, `Marks::Auto` skips the
/// pairing and the word marks, so a large file stays fast to open.
pub const LARGE_FILE_LINES: usize = 40_000;

/// A text file's diff: both sides whole, and the rows that align them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TextDiff {
    /// The old side's lines, without their line endings.
    pub old_lines: Vec<String>,
    pub new_lines: Vec<String>,
    /// Every line of both sides, once, in order.
    pub rows: Vec<Row>,
    /// Whether each side's last line has no line ending. When only one
    /// side has none, git counts that last line as changed.
    pub old_missing_final_newline: bool,
    pub new_missing_final_newline: bool,
    /// The file is large and `Marks::Auto` left the lines of each block
    /// unpaired and unmarked.
    pub marks_skipped: bool,
}

/// One file's diff.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum FileDiff {
    Text(TextDiff),
    /// Sizes in bytes; `None` for a side the file is not on.
    Binary {
        old_size: Option<u64>,
        new_size: Option<u64>,
    },
    /// A repository inside the repository.
    Submodule,
    /// Conflict markers are not a change to read: the merge editor
    /// resolves the file.
    Conflicted,
}

/// Why a file's diff could not be read.
#[derive(Debug)]
pub enum DiffError {
    Git(GitError),
    /// A file of the working copy, or one git handed over, cannot be read.
    Read {
        path: PathBuf,
        error: io::Error,
    },
}

impl fmt::Display for DiffError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DiffError::Git(error) => error.fmt(f),
            DiffError::Read { path, error } => {
                write!(f, "cannot read {}: {error}", path.display())
            }
        }
    }
}

impl std::error::Error for DiffError {}

impl From<GitError> for DiffError {
    fn from(error: GitError) -> DiffError {
        DiffError::Git(error)
    }
}

/// The file at `path`, whole.
fn read(path: &Path) -> Result<Vec<u8>, DiffError> {
    fs::read(path).map_err(|error| DiffError::Read {
        path: path.to_path_buf(),
        error,
    })
}

/// A context this large makes `git diff` print the whole file around its
/// changes, in one hunk.
const WHOLE_FILE: &str = "--unified=100000000";

/// The diff of `file`, a file of `changed_files(comparison)`.
pub fn file_diff(
    git: &dyn Git,
    root: &Path,
    comparison: &Comparison,
    file: &ChangedFile,
    marks: Marks,
) -> Result<FileDiff, DiffError> {
    if file.state == FileState::Conflicted {
        return Ok(FileDiff::Conflicted);
    }
    if file.submodule {
        return Ok(FileDiff::Submodule);
    }
    if file.state == FileState::Untracked {
        let bytes = read(&root.join(&file.path))?;
        if is_binary(&bytes) {
            return Ok(FileDiff::Binary {
                old_size: None,
                new_size: Some(bytes.len() as u64),
            });
        }
        return Ok(FileDiff::Text(TextDiff::added(Text::of(&bytes))));
    }

    let mut arguments = vec![WHOLE_FILE];
    arguments.extend(comparison.sides());
    arguments.push("--");
    arguments.push(&file.path);
    if let Some(old_path) = &file.old_path {
        arguments.push(old_path);
    }
    let mut all: Vec<&str> = DIFF_OPTIONS.to_vec();
    all.extend(arguments);
    let output = git.output(&all)?;
    if output.status != Some(0) {
        return Err(DiffError::Git(GitError::Failed {
            command: all.join(" "),
            stderr: output.stderr,
        }));
    }
    let patch = parse_patch(&String::from_utf8_lossy(&output.stdout));
    if patch.binary {
        let old_path = file.old_path.as_deref().unwrap_or(&file.path);
        // An added file has no old blob, and git then gives no size.
        let old_size = blob_size(git, &comparison.old_blob(old_path));
        let new_size = match (file.state, comparison.new_blob(&file.path)) {
            (FileState::Deleted, _) => None,
            (_, Some(blob)) => blob_size(git, &blob),
            (_, None) => fs::metadata(root.join(&file.path))
                .ok()
                .map(|metadata| metadata.len()),
        };
        return Ok(FileDiff::Binary { old_size, new_size });
    }
    if patch.lines.is_empty() && file.state != FileState::Deleted {
        // Nothing changed in the content (a file only renamed, or made
        // executable): git prints no line, and the file is shown plain.
        let bytes = match comparison.new_blob(&file.path) {
            Some(blob) => git.output(&["cat-file", "blob", &blob])?.stdout,
            None => fs::read(root.join(&file.path)).unwrap_or_default(),
        };
        if is_binary(&bytes) {
            let size = Some(bytes.len() as u64);
            return Ok(FileDiff::Binary {
                old_size: size,
                new_size: size,
            });
        }
        return Ok(FileDiff::Text(TextDiff::unchanged(Text::of(&bytes))));
    }
    Ok(FileDiff::Text(TextDiff::from_patch(patch, marks)))
}

/// The diff of two files that need not be in a repository, as `git
/// difftool` hands them over.
pub fn plain_files(
    git: &dyn Git,
    old: &Path,
    new: &Path,
    marks: Marks,
) -> Result<FileDiff, DiffError> {
    // git would compare a symbolic link's target path, not the file it
    // points to: `git difftool --dir-diff` hands over links to the working
    // copy's files.
    let (old, new) = (&followed(old), &followed(new));
    let (old_text, new_text) = (old.to_string_lossy(), new.to_string_lossy());
    let arguments = [
        "diff",
        "--no-index",
        "--no-color",
        "--no-ext-diff",
        "--no-textconv",
        WHOLE_FILE,
        "--",
        &old_text,
        &new_text,
    ];
    let output = git.output(&arguments)?;
    // 1 says the files differ; anything else is a failure.
    if !matches!(output.status, Some(0) | Some(1)) {
        return Err(DiffError::Git(GitError::Failed {
            command: arguments.join(" "),
            stderr: output.stderr,
        }));
    }
    let patch = parse_patch(&String::from_utf8_lossy(&output.stdout));
    if patch.binary {
        let size = |path: &Path| fs::metadata(path).ok().map(|metadata| metadata.len());
        return Ok(FileDiff::Binary {
            old_size: size(old),
            new_size: size(new),
        });
    }
    if patch.lines.is_empty() {
        let bytes = read(new)?;
        if is_binary(&bytes) {
            let size = Some(bytes.len() as u64);
            return Ok(FileDiff::Binary {
                old_size: size,
                new_size: size,
            });
        }
        return Ok(FileDiff::Text(TextDiff::unchanged(Text::of(&bytes))));
    }
    Ok(FileDiff::Text(TextDiff::from_patch(patch, marks)))
}

/// `path` with its symbolic links followed, or as it is when it cannot be
/// read: git then says why.
fn followed(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// What stands for a file that one side does not have, for `git diff
/// --no-index`: every line of the other side is then added or removed.
const NO_FILE: &str = "/dev/null";

/// The files that differ between two folders, as `git difftool
/// --dir-diff` hands them over: by path from the folders, in order. A
/// file in one folder only is added or deleted; symbolic links are
/// followed.
pub fn folder_files(git: &dyn Git, old: &Path, new: &Path) -> Result<Vec<ChangedFile>, DiffError> {
    let mut paths = Vec::new();
    for folder in [old, new] {
        files_under(folder, "", &mut paths)?;
    }
    paths.sort();
    paths.dedup();
    let mut files = Vec::new();
    for path in paths {
        let (old_file, new_file) = (old.join(&path), new.join(&path));
        let state = match (old_file.exists(), new_file.exists()) {
            (true, true) => FileState::Modified,
            (false, true) => FileState::Added,
            _ => FileState::Deleted,
        };
        let old_bytes = if state == FileState::Added {
            Vec::new()
        } else {
            read(&old_file)?
        };
        let new_bytes = if state == FileState::Deleted {
            Vec::new()
        } else {
            read(&new_file)?
        };
        if state == FileState::Modified && old_bytes == new_bytes {
            continue;
        }
        let mut file = changed_file(&path, state);
        if is_binary(&old_bytes) || is_binary(&new_bytes) {
            file.binary = true;
        } else {
            // git counts, so the list agrees with the file's rows.
            let (old_text, new_text) = (
                followed(&old_file).to_string_lossy().into_owned(),
                followed(&new_file).to_string_lossy().into_owned(),
            );
            let side = |text: &'_ str, missing: bool| {
                if missing {
                    NO_FILE.to_string()
                } else {
                    text.to_string()
                }
            };
            let (old_argument, new_argument) = (
                side(&old_text, state == FileState::Added),
                side(&new_text, state == FileState::Deleted),
            );
            let output = git.output(&[
                "diff",
                "--no-index",
                "--no-color",
                "--no-ext-diff",
                "--no-textconv",
                "--numstat",
                "--",
                &old_argument,
                &new_argument,
            ])?;
            let text = String::from_utf8_lossy(&output.stdout);
            let mut counts = text.split('\t').map(|count| count.trim().parse().ok());
            file.added = counts.next().flatten();
            file.removed = counts.next().flatten();
        }
        files.push(file);
    }
    Ok(files)
}

/// The diff of `file`, a file of `folder_files(old, new)`.
pub fn folder_file_diff(
    git: &dyn Git,
    old: &Path,
    new: &Path,
    file: &ChangedFile,
    marks: Marks,
) -> Result<FileDiff, DiffError> {
    let none = Path::new(NO_FILE);
    let (old_file, new_file) = (old.join(&file.path), new.join(&file.path));
    plain_files(
        git,
        if file.state == FileState::Added {
            none
        } else {
            &old_file
        },
        if file.state == FileState::Deleted {
            none
        } else {
            &new_file
        },
        marks,
    )
}

/// The files under `folder`, by their path from it with `/` between
/// folders, links to files and folders followed.
fn files_under(folder: &Path, prefix: &str, paths: &mut Vec<String>) -> Result<(), DiffError> {
    let failed = |error| DiffError::Read {
        path: folder.to_path_buf(),
        error,
    };
    for entry in fs::read_dir(folder).map_err(failed)? {
        let entry = entry.map_err(failed)?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let path = format!("{prefix}{name}");
        // `metadata` follows links, where the entry's own type does not.
        match fs::metadata(entry.path()) {
            Ok(metadata) if metadata.is_dir() => {
                files_under(&entry.path(), &format!("{path}/"), paths)?;
            }
            Ok(_) => paths.push(path),
            // A link to a file that is gone: nothing to compare.
            Err(_) => {}
        }
    }
    Ok(())
}

/// A blob's size in bytes, when git has it.
fn blob_size(git: &dyn Git, blob: &str) -> Option<u64> {
    git.run(&["cat-file", "-s", blob]).ok()?.trim().parse().ok()
}

/// A text as lines.
struct Text {
    lines: Vec<String>,
    missing_final_newline: bool,
}

impl Text {
    /// Lines end at a line feed; a carriage return before it is dropped,
    /// as is a lone one ending the text.
    fn of(bytes: &[u8]) -> Text {
        let text = String::from_utf8_lossy(bytes);
        let mut lines: Vec<String> = text
            .split('\n')
            .map(|line| line.strip_suffix('\r').unwrap_or(line).to_string())
            .collect();
        // The piece after the last line feed is a line only when it has
        // text: that line then has no line ending.
        let last_is_empty = lines.last().is_some_and(String::is_empty);
        if last_is_empty {
            lines.pop();
        }
        Text {
            missing_final_newline: !last_is_empty,
            lines,
        }
    }
}

/// Which side a line of a patch is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Sign {
    Both,
    Old,
    New,
}

/// What `git diff` printed for one file.
#[derive(Debug, Default)]
pub struct Patch {
    binary: bool,
    lines: Vec<(Sign, String)>,
    old_missing_final_newline: bool,
    new_missing_final_newline: bool,
}

/// One file's patch: its lines after the first hunk header, each on the
/// old side, the new side or both. The header before it is skipped,
/// except where it says the file is binary.
pub fn parse_patch(text: &str) -> Patch {
    let mut patch = Patch::default();
    let mut in_hunk = false;
    for line in text.split('\n') {
        if !in_hunk {
            if line.starts_with("@@ ") {
                in_hunk = true;
            } else if line.starts_with("Binary files ") || line == "GIT binary patch" {
                patch.binary = true;
            }
            continue;
        }
        let line = line.strip_suffix('\r').unwrap_or(line);
        let (sign, rest) = match line.as_bytes().first() {
            Some(b' ') => (Sign::Both, &line[1..]),
            Some(b'-') => (Sign::Old, &line[1..]),
            Some(b'+') => (Sign::New, &line[1..]),
            // "\ No newline at end of file" is about the line before it.
            Some(b'\\') => {
                match patch.lines.last().map(|(sign, _)| *sign) {
                    Some(Sign::Old) => patch.old_missing_final_newline = true,
                    Some(Sign::New) => patch.new_missing_final_newline = true,
                    Some(Sign::Both) => {
                        patch.old_missing_final_newline = true;
                        patch.new_missing_final_newline = true;
                    }
                    None => {}
                }
                continue;
            }
            // Another hunk, which a whole-file context never leaves, or
            // the empty piece after the last line feed.
            _ => continue,
        };
        patch.lines.push((sign, rest.to_string()));
    }
    patch
}

impl TextDiff {
    /// A file with nothing on the old side: every line is added.
    fn added(text: Text) -> TextDiff {
        TextDiff {
            rows: (1..=text.lines.len())
                .map(|number| Row {
                    kind: RowKind::Added,
                    old: None,
                    new: Some(number),
                    old_words: Vec::new(),
                    new_words: Vec::new(),
                })
                .collect(),
            old_lines: Vec::new(),
            new_lines: text.lines,
            old_missing_final_newline: false,
            new_missing_final_newline: text.missing_final_newline,
            marks_skipped: false,
        }
    }

    /// A file whose content is the same on both sides.
    fn unchanged(text: Text) -> TextDiff {
        TextDiff {
            rows: (1..=text.lines.len())
                .map(|number| Row {
                    kind: RowKind::Unchanged,
                    old: Some(number),
                    new: Some(number),
                    old_words: Vec::new(),
                    new_words: Vec::new(),
                })
                .collect(),
            old_lines: text.lines.clone(),
            new_lines: text.lines,
            old_missing_final_newline: text.missing_final_newline,
            new_missing_final_newline: text.missing_final_newline,
            marks_skipped: false,
        }
    }

    /// The rows of a patch: its unchanged lines as they come, and each
    /// run of removed and added lines as one block.
    pub fn from_patch(patch: Patch, marks: Marks) -> TextDiff {
        let marks_skipped = marks == Marks::Auto && {
            let both = patch
                .lines
                .iter()
                .filter(|(sign, _)| *sign == Sign::Both)
                .count();
            patch.lines.len() + both > LARGE_FILE_LINES
        };
        let mut diff = TextDiff {
            old_lines: Vec::new(),
            new_lines: Vec::new(),
            rows: Vec::new(),
            old_missing_final_newline: patch.old_missing_final_newline,
            new_missing_final_newline: patch.new_missing_final_newline,
            marks_skipped,
        };
        let mut removed: Vec<String> = Vec::new();
        let mut added: Vec<String> = Vec::new();
        for (sign, line) in patch.lines {
            match sign {
                Sign::Old => removed.push(line),
                Sign::New => added.push(line),
                Sign::Both => {
                    diff.push_block(&mut removed, &mut added);
                    diff.old_lines.push(line.clone());
                    diff.new_lines.push(line);
                    diff.rows.push(Row {
                        kind: RowKind::Unchanged,
                        old: Some(diff.old_lines.len()),
                        new: Some(diff.new_lines.len()),
                        old_words: Vec::new(),
                        new_words: Vec::new(),
                    });
                }
            }
        }
        diff.push_block(&mut removed, &mut added);
        diff
    }

    /// Adds a block's rows and empties it. Lines that are alike sit on
    /// one row with their changed words marked; the lines between two
    /// such rows sit side by side as far as they go, unmarked, so the
    /// block is no taller than its longer side.
    fn push_block(&mut self, removed: &mut Vec<String>, added: &mut Vec<String>) {
        if removed.is_empty() && added.is_empty() {
            return;
        }
        let pairs = if self.marks_skipped {
            Vec::new()
        } else {
            let old: Vec<&str> = removed.iter().map(String::as_str).collect();
            let new: Vec<&str> = added.iter().map(String::as_str).collect();
            words::paired_lines(&old, &new)
        };
        let old_start = self.old_lines.len();
        let new_start = self.new_lines.len();
        let (mut old_at, mut new_at) = (0, 0);
        let ends = pairs
            .iter()
            .copied()
            .map(|pair| (pair, true))
            .chain(std::iter::once(((removed.len(), added.len()), false)));
        for ((old_end, new_end), alike) in ends {
            // The lines before this pair, side by side as far as they go.
            while old_at < old_end || new_at < new_end {
                let old = (old_at < old_end).then(|| old_start + old_at + 1);
                let new = (new_at < new_end).then(|| new_start + new_at + 1);
                self.rows.push(Row {
                    kind: match (old, new) {
                        (Some(_), Some(_)) => RowKind::Changed,
                        (Some(_), None) => RowKind::Removed,
                        _ => RowKind::Added,
                    },
                    old,
                    new,
                    old_words: Vec::new(),
                    new_words: Vec::new(),
                });
                old_at += usize::from(old.is_some());
                new_at += usize::from(new.is_some());
            }
            if alike {
                let (old_line, new_line) = (&removed[old_end], &added[new_end]);
                self.rows.push(Row {
                    kind: RowKind::Changed,
                    old: Some(old_start + old_end + 1),
                    new: Some(new_start + new_end + 1),
                    old_words: words::changed_words(new_line, old_line),
                    new_words: words::changed_words(old_line, new_line),
                });
                old_at = old_end + 1;
                new_at = new_end + 1;
            }
        }
        self.old_lines.append(removed);
        self.new_lines.append(added);
    }

    /// The blocks of changed rows, in order.
    pub fn changes(&self) -> Vec<Change> {
        let mut changes: Vec<Change> = Vec::new();
        let mut open = false;
        // A row is one unified line, or two when it has both sides.
        let mut line = 0;
        for (index, row) in self.rows.iter().enumerate() {
            let lines = usize::from(row.old.is_some()) + usize::from(row.new.is_some());
            if row.kind == RowKind::Unchanged {
                open = false;
                line += 1;
                continue;
            }
            if !open {
                changes.push(Change {
                    first_row: index,
                    row_count: 0,
                    first_line: line,
                    added: 0,
                    removed: 0,
                });
                open = true;
            }
            line += lines;
            if let Some(change) = changes.last_mut() {
                change.row_count += 1;
                change.removed += usize::from(row.old.is_some());
                change.added += usize::from(row.new.is_some());
            }
        }
        changes
    }

    /// The same rows for the unified view: in each block, every removed
    /// line, then every added one.
    pub fn unified(&self) -> Vec<UnifiedLine> {
        let mut lines = Vec::with_capacity(self.rows.len());
        let mut added: Vec<UnifiedLine> = Vec::new();
        for row in &self.rows {
            if row.kind == RowKind::Unchanged {
                lines.append(&mut added);
                lines.push(UnifiedLine {
                    kind: LineKind::Unchanged,
                    old: row.old,
                    new: row.new,
                    words: Vec::new(),
                });
                continue;
            }
            if row.old.is_some() {
                lines.push(UnifiedLine {
                    kind: LineKind::Removed,
                    old: row.old,
                    new: None,
                    words: row.old_words.clone(),
                });
            }
            if row.new.is_some() {
                added.push(UnifiedLine {
                    kind: LineKind::Added,
                    old: None,
                    new: row.new,
                    words: row.new_words.clone(),
                });
            }
        }
        lines.append(&mut added);
        lines
    }

    /// How many lines the new side adds and the old side loses, as `git
    /// diff --numstat` counts them.
    pub fn counts(&self) -> (usize, usize) {
        self.changes()
            .iter()
            .fold((0, 0), |(added, removed), change| {
                (added + change.added, removed + change.removed)
            })
    }
}

/// A line as a front end draws it: tabs expanded, measured, coloured.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ShownLine {
    pub text: String,
    /// Its width in columns of a monospaced font.
    pub columns: usize,
    /// Syntax colours, in UTF-8 bytes of `text`. Empty for a language
    /// Scry has no grammar for, and in a large file.
    pub spans: Vec<Span>,
}

/// A text diff ready to draw: every offset is in the shown lines, where a
/// tab takes the columns up to its stop.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ShownDiff {
    pub old_lines: Vec<ShownLine>,
    pub new_lines: Vec<ShownLine>,
    /// The split view's rows.
    pub rows: Vec<Row>,
    /// The unified view's lines.
    pub unified: Vec<UnifiedLine>,
    pub changes: Vec<Change>,
    /// The widest line of either side, in columns: how far the view
    /// scrolls sideways.
    pub longest_line: usize,
}

impl TextDiff {
    /// The diff as a front end draws it, with the syntax colours of
    /// `path`'s language. A large file opened without its marks gets no
    /// colours either: parsing both sides is what would make it slow.
    pub fn shown(&self, path: &Path) -> ShownDiff {
        let highlighter = Language::from_path(path)
            .filter(|_| !self.marks_skipped)
            .and_then(Highlighter::new);
        let side = |lines: &[String]| -> (Vec<ShownLine>, Vec<Shown>) {
            let spans = highlighter
                .as_ref()
                .map(|highlighter| highlighter.lines(&lines.join("\n")))
                // One list per line, or the colours would sit on the
                // wrong lines.
                .filter(|spans| spans.len() == lines.len());
            let shown: Vec<Shown> = lines.iter().map(|line| Shown::new(line)).collect();
            let drawn = shown
                .iter()
                .enumerate()
                .map(|(index, line)| ShownLine {
                    text: line.text.clone(),
                    columns: line.columns,
                    spans: spans
                        .as_ref()
                        .map(|spans| {
                            spans[index]
                                .iter()
                                .map(|span| Span {
                                    start: line.offset(span.start),
                                    end: line.offset(span.end),
                                    role: span.role,
                                })
                                .collect()
                        })
                        .unwrap_or_default(),
                })
                .collect();
            (drawn, shown)
        };
        let (old_lines, old_shown) = side(&self.old_lines);
        let (new_lines, new_shown) = side(&self.new_lines);
        let moved = |words: &[Range<usize>], number: Option<usize>, shown: &[Shown]| match number {
            Some(number) => words
                .iter()
                .map(|word| shown[number - 1].range(word.clone()))
                .collect(),
            None => Vec::new(),
        };
        let rows: Vec<Row> = self
            .rows
            .iter()
            .map(|row| Row {
                old_words: moved(&row.old_words, row.old, &old_shown),
                new_words: moved(&row.new_words, row.new, &new_shown),
                ..row.clone()
            })
            .collect();
        let unified = self
            .unified()
            .into_iter()
            .map(|line| UnifiedLine {
                words: match line.kind {
                    LineKind::Removed => moved(&line.words, line.old, &old_shown),
                    _ => moved(&line.words, line.new, &new_shown),
                },
                ..line
            })
            .collect();
        let longest_line = old_lines
            .iter()
            .chain(&new_lines)
            .map(|line| line.columns)
            .max()
            .unwrap_or(0);
        ShownDiff {
            old_lines,
            new_lines,
            rows,
            unified,
            changes: self.changes(),
            longest_line,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What is drawn has its tabs expanded, so every offset moves with
    /// them: the changed word and the syntax colours sit on the same
    /// text as before, in the split rows and in the unified lines, and
    /// the widest line counts the expanded columns.
    #[test]
    fn a_shown_diff_moves_marks_and_colours_past_expanded_tabs() {
        let patch = parse_patch(
            "@@ -1,2 +1,2 @@\n fn main() {\n-\tlet timeout = 30;\n+\tlet timeout = 60;\n",
        );
        let diff = TextDiff::from_patch(patch, Marks::Auto);
        assert_eq!(diff.rows[1].new_words.first(), Some(&(15..17)));
        let shown = diff.shown(Path::new("src/main.rs"));

        assert_eq!(shown.new_lines[1].text, "    let timeout = 60;");
        assert_eq!(shown.longest_line, 21);
        let word = shown.rows[1].new_words[0].clone();
        assert_eq!(&shown.new_lines[1].text[word], "60");
        let word = shown.rows[1].old_words[0].clone();
        assert_eq!(&shown.old_lines[1].text[word], "30");
        // Unified: line 2 removed, then line 2 added, each with its mark.
        let removed = &shown.unified[1];
        assert_eq!(&shown.old_lines[1].text[removed.words[0].clone()], "30");
        let added = &shown.unified[2];
        assert_eq!(&shown.new_lines[1].text[added.words[0].clone()], "60");

        let keyword = shown.new_lines[1]
            .spans
            .iter()
            .find(|span| span.role == crate::highlight::SyntaxRole::Keyword)
            .expect("a keyword on the line");
        assert_eq!(&shown.new_lines[1].text[keyword.start..keyword.end], "let");
        assert_eq!(shown.changes, diff.changes());
    }

    /// A file in a language without a grammar is drawn plain, and so is a
    /// large file opened without its marks: no colours rather than slow
    /// ones.
    #[test]
    fn a_shown_diff_is_plain_without_a_grammar_or_in_a_large_file() {
        let patch = || parse_patch("@@ -1 +1 @@\n-let a = 1;\n+let a = 2;\n");
        let text = TextDiff::from_patch(patch(), Marks::Auto);
        assert!(
            text.shown(Path::new("notes.unknown")).new_lines[0]
                .spans
                .is_empty()
        );
        assert!(!text.shown(Path::new("a.rs")).new_lines[0].spans.is_empty());

        let mut large = TextDiff::from_patch(patch(), Marks::Auto);
        large.marks_skipped = true;
        assert!(large.shown(Path::new("a.rs")).new_lines[0].spans.is_empty());
    }

    /// The list is read from git's raw records and counts: a renamed file
    /// keeps its old path and takes its counts from the entry with two
    /// paths, a binary file has no counts, and a submodule is flagged from
    /// its mode and never counted as one line changed.
    #[test]
    fn changes_are_read_from_raw_records_and_counts() {
        let files = parse_changes(concat!(
            ":100644 100644 1a2b3c4 5d6e7f8 M\0src/a.rs\0",
            ":100644 100644 1a2b3c4 5d6e7f8 R087\0old name.rs\0src/new.rs\0",
            ":000000 100644 0000000 5d6e7f8 A\0icon.png\0",
            ":160000 160000 1a2b3c4 5d6e7f8 M\0vendor/lib\0",
            ":100644 120000 1a2b3c4 5d6e7f8 T\0link\0",
            ":100644 000000 1a2b3c4 0000000 D\0gone.rs\0",
            ":000000 160000 0000000 5d6e7f8 A\0vendor/new\0",
            ":160000 000000 1a2b3c4 0000000 D\0vendor/old\0",
            ":100644 100644 1a2b3c4 1a2b3c4 C075\0src/a.rs\0src/copy.rs\0",
            "3\t1\tsrc/a.rs\0",
            "2\t5\t\0old name.rs\0src/new.rs\0",
            "-\t-\ticon.png\0",
            "1\t1\tvendor/lib\0",
            "1\t4\tlink\0",
            "0\t9\tgone.rs\0",
            "1\t0\tvendor/new\0",
            "0\t1\tvendor/old\0",
            "7\t0\t\0src/a.rs\0src/copy.rs\0",
        ));
        let seen: Vec<_> = files
            .iter()
            .map(|file| (file.path.as_str(), file.state, file.added, file.removed))
            .collect();
        assert_eq!(
            seen,
            [
                ("src/a.rs", FileState::Modified, Some(3), Some(1)),
                ("src/new.rs", FileState::Renamed, Some(2), Some(5)),
                ("icon.png", FileState::Added, None, None),
                ("vendor/lib", FileState::Modified, None, None),
                ("link", FileState::TypeChanged, Some(1), Some(4)),
                ("gone.rs", FileState::Deleted, Some(0), Some(9)),
                ("vendor/new", FileState::Added, None, None),
                ("vendor/old", FileState::Deleted, None, None),
                // A copy, which git is not asked to find, is a new file.
                ("src/copy.rs", FileState::Added, Some(7), Some(0)),
            ]
        );
        assert_eq!(files[1].old_path.as_deref(), Some("old name.rs"));
        assert_eq!(
            (files[1].name.as_str(), files[1].directory.as_str()),
            ("new.rs", "src/")
        );
        assert!(files[2].binary && !files[0].binary);
        assert!(files[3].submodule && !files[3].binary);
        // A submodule added or removed has the mode on one side only.
        assert!(files[6].submodule && files[7].submodule);
        assert_eq!(files[8].old_path, None);
        assert!(files.iter().all(|file| file.group.is_none()));
    }

    /// Against the index, git lists a conflicted file twice, as unmerged
    /// and as modified: it is one conflicted entry, without counts, and
    /// the order of the two records does not matter.
    #[test]
    fn a_file_listed_twice_is_conflicted_once() {
        for records in [
            ":000000 000000 0000000 0000000 U\0a.txt\0:100644 100644 1a2b3c4 0000000 M\0a.txt\0",
            ":100644 100644 1a2b3c4 0000000 M\0a.txt\0:000000 000000 0000000 0000000 U\0a.txt\0",
        ] {
            let files = parse_changes(&format!("{records}4\t0\ta.txt\0"));
            assert_eq!(files.len(), 1, "{files:?}");
            assert_eq!(files[0].state, FileState::Conflicted);
            assert_eq!((files[0].added, files[0].removed), (None, None));
        }
    }

    /// Output cut short, or that is not git's, gives the files read so far
    /// and never a panic: the list comes from a process that can be killed.
    #[test]
    fn cut_or_foreign_output_is_read_as_far_as_it_goes() {
        assert!(parse_changes("").is_empty());
        assert!(parse_changes(":100644 100644 1a2b3c4 5d6e7f8 M").is_empty());
        assert!(parse_changes(":100644 100644 1a2b3c4 5d6e7f8 R100\0old").is_empty());
        assert!(parse_changes("not git\0at all\0").is_empty());
        let files = parse_changes(concat!(":1 2 3 4 M\0a\0:\0b\0", "3\t\0x"));
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].added, None);
        // Only "-" for both counts says binary.
        let odd = parse_changes(":1 2 3 4 M\0a\0-\t3\ta\0");
        assert!(!odd[0].binary);
        assert_eq!((odd[0].added, odd[0].removed), (None, Some(3)));
    }

    /// The marks are skipped above the limit and not at it, counting the
    /// lines of both sides: a file one line under opens with its marks.
    #[test]
    fn the_large_file_limit_counts_both_sides_lines() {
        let patch = |unchanged: usize| {
            let mut text = String::from("@@ -1 +1 @@\n-let timeout = 30\n+let timeout = 60\n");
            text.push_str(&" same\n".repeat(unchanged));
            parse_patch(&text)
        };
        // One changed line and `unchanged` others on each side.
        let at_limit = LARGE_FILE_LINES / 2 - 1;
        let diff = TextDiff::from_patch(patch(at_limit), Marks::Auto);
        assert_eq!(
            diff.old_lines.len() + diff.new_lines.len(),
            LARGE_FILE_LINES
        );
        assert!(!diff.marks_skipped);
        assert!(!diff.rows[0].new_words.is_empty());

        let over = TextDiff::from_patch(patch(at_limit + 1), Marks::Auto);
        assert!(over.marks_skipped);
        assert!(over.rows[0].new_words.is_empty());
        assert!(!TextDiff::from_patch(patch(at_limit + 1), Marks::Always).marks_skipped);
    }

    /// An error names the file that could not be read, or says what git
    /// said: the front end shows it as it is.
    #[test]
    fn an_error_says_which_file_or_what_git_said() {
        let read = DiffError::Read {
            path: PathBuf::from("src/a.rs"),
            error: io::Error::new(io::ErrorKind::NotFound, "gone"),
        };
        assert_eq!(read.to_string(), "cannot read src/a.rs: gone");
        let git = GitError::Failed {
            command: String::from("diff"),
            stderr: String::from("bad revision"),
        };
        assert_eq!(
            DiffError::from(git).to_string(),
            GitError::Failed {
                command: String::from("diff"),
                stderr: String::from("bad revision"),
            }
            .to_string()
        );
    }

    /// A patch's header is skipped whatever it holds, the lines after the
    /// hunk header go to their side with their sign removed, and "No
    /// newline at end of file" flags the side of the line before it.
    #[test]
    fn a_patch_is_read_after_its_header() {
        let patch = parse_patch(concat!(
            "diff --git a/a.txt b/a.txt\nindex 1a2b3c4..5d6e7f8 100644\n",
            "--- a/a.txt\n+++ b/a.txt\n@@ -1,3 +1,3 @@ heading\n",
            " same\n-old\n+new\n \n-last\n\\ No newline at end of file\n+last\n",
        ));
        assert!(!patch.binary);
        assert_eq!(
            patch.lines,
            [
                (Sign::Both, String::from("same")),
                (Sign::Old, String::from("old")),
                (Sign::New, String::from("new")),
                (Sign::Both, String::new()),
                (Sign::Old, String::from("last")),
                (Sign::New, String::from("last")),
            ]
        );
        assert!(patch.old_missing_final_newline);
        assert!(!patch.new_missing_final_newline);

        // On an unchanged last line, the note is about both sides.
        let both = parse_patch("@@ -1 +1,2 @@\n+new\n last\n\\ No newline at end of file\n");
        assert!(both.old_missing_final_newline && both.new_missing_final_newline);
        let new = parse_patch("@@ -1 +1 @@\n-a\n+b\n\\ No newline at end of file\n");
        assert!(!new.old_missing_final_newline && new.new_missing_final_newline);
    }

    /// A binary file's patch says so in its header, in either of git's two
    /// forms, and a text line that only looks like that note does not.
    #[test]
    fn a_binary_patch_is_recognised_in_the_header_only() {
        assert!(parse_patch("diff --git a/i b/i\nBinary files a/i and b/i differ\n").binary);
        assert!(parse_patch("diff --git a/i b/i\nGIT binary patch\nliteral 3\n").binary);
        let text = parse_patch("@@ -1 +1 @@\n-Binary files a and b differ\n+GIT binary patch\n");
        assert!(!text.binary);
        assert_eq!(text.lines.len(), 2);
    }

    /// Lines end at a line feed with the carriage return before it
    /// dropped; a last line without a line ending is a line and is
    /// flagged, and an empty file has no line and nothing missing.
    #[test]
    fn a_text_is_split_into_lines_without_their_endings() {
        let lines = |bytes: &[u8]| {
            let text = Text::of(bytes);
            (text.lines, text.missing_final_newline)
        };
        assert_eq!(
            lines(b"a\nb\n"),
            (vec!["a".to_string(), "b".to_string()], false)
        );
        assert_eq!(
            lines(b"a\r\nb"),
            (vec!["a".to_string(), "b".to_string()], true)
        );
        assert_eq!(
            lines(b"a\n\n"),
            (vec!["a".to_string(), String::new()], false)
        );
        assert_eq!(lines(b""), (Vec::new(), false));
        assert_eq!(lines(b"\n"), (vec![String::new()], false));
    }

    /// Like git, a NUL among a file's first bytes makes it binary, and one
    /// far into a long text does not.
    #[test]
    fn a_nul_near_the_start_makes_a_file_binary() {
        assert!(is_binary(b"PNG\0data"));
        assert!(!is_binary(b"plain text"));
        assert!(!is_binary(b""));
        let mut late = vec![b'a'; BINARY_PROBE];
        late.push(0);
        assert!(!is_binary(&late));
        late[BINARY_PROBE - 1] = 0;
        assert!(is_binary(&late));
    }
}
