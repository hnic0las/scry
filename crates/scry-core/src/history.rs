//! One file's history: the points where it changed, newest first, and
//! what each point changed.
//!
//! The points are what is not committed, what is staged, then the commits
//! that touched the file (`git log --follow`, so the file is followed
//! under the names it had). A point is read against the point before it;
//! with a pin, against the pinned side instead. Nothing is ever written.
//!
//! Every function expects `git` to run at the repository's root, and
//! paths relative to it.

use std::path::Path;

use crate::diff::{
    self, ChangedFile, Comparison, DiffError, FileDiff, FileState, Marks, parse_changes,
};
use crate::git::{Git, GitError};

/// How many commits a page of the history holds.
pub const PAGE: usize = 50;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PointKind {
    /// The working copy, where it differs from the index.
    NotCommitted,
    /// The index, where it differs from HEAD.
    Staged,
    Commit,
}

/// A point of the file's history.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Point {
    pub kind: PointKind,
    /// The commit's id; empty for what is not committed.
    pub id: String,
    /// The first parent; `None` for a first commit, and off a commit.
    pub parent: Option<String>,
    pub subject: String,
    pub author: String,
    /// When the commit was written, in seconds since 1970; 0 off a commit.
    pub time: i64,
    /// The file's path at this point.
    pub path: String,
    /// The path before, when this point renamed the file.
    pub old_path: Option<String>,
    /// Lines this point added to the file and removed from it; `None`
    /// for a binary file.
    pub added: Option<usize>,
    pub removed: Option<usize>,
    /// The commit is HEAD.
    pub head: bool,
}

impl Point {
    /// The commit's id cut to seven characters.
    pub fn short_id(&self) -> &str {
        self.id.get(..7).unwrap_or(&self.id)
    }
}

/// Some commits of the history, and whether older ones are left.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Page {
    pub points: Vec<Point>,
    pub more: bool,
}

/// One side held still while the points are stepped through.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pin {
    /// The working copy, with the file's path there.
    WorkingCopy { path: String },
    /// A commit of the history, with the file's path there.
    Commit { id: String, path: String },
}

/// Where a point stands in the list relative to a pinned commit: git
/// compares the older one with the newer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Order {
    /// The point is older than the pin, or is the pin.
    Older,
    Newer,
}

/// The commit `name` stands for, or `None` when there is none: HEAD in a
/// repository with no commit yet.
fn commit_id(git: &dyn Git, name: &str) -> Result<Option<String>, GitError> {
    let name = format!("{name}^{{commit}}");
    let output = git.output(&[
        "rev-parse",
        "--verify",
        "--quiet",
        "--end-of-options",
        &name,
    ])?;
    Ok((output.status == Some(0))
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string()))
}

/// The points before any commit: what is not committed of the file at
/// `path`, then what is staged, each when there is something.
pub fn uncommitted(git: &dyn Git, root: &Path, path: &str) -> Result<Vec<Point>, GitError> {
    let paths = [path.to_string()];
    let point = |kind: PointKind, file: &ChangedFile| Point {
        kind,
        id: String::new(),
        parent: None,
        subject: String::new(),
        author: String::new(),
        time: 0,
        path: file.path.clone(),
        old_path: file.old_path.clone(),
        added: file.added,
        removed: file.removed,
        head: false,
    };
    let mut points = Vec::new();
    // Untracked files are listed with the working copy, against a commit.
    let whole = diff::changed_files(git, root, &Comparison::uncommitted(git)?, &paths)?;
    let untracked = whole
        .iter()
        .find(|file| file.path == path && file.state == FileState::Untracked);
    let unstaged = diff::changed_files(git, root, &Comparison::Unstaged, &paths)?;
    if let Some(file) = untracked.or(unstaged.iter().find(|file| file.path == path)) {
        points.push(point(PointKind::NotCommitted, file));
    }
    // A staged rename is found only when both names are looked at.
    let staged = diff::changed_files(git, root, &Comparison::staged(git)?, &[])?;
    if let Some(file) = staged.iter().find(|file| file.path == path) {
        points.push(point(PointKind::Staged, file));
    }
    Ok(points)
}

/// The path to follow the commits of: the file's name in HEAD, which a
/// rename not committed yet has changed.
pub fn committed_path(points: &[Point], path: &str) -> String {
    points
        .iter()
        .find(|point| point.kind == PointKind::Staged)
        .and_then(|point| point.old_path.clone())
        .unwrap_or_else(|| path.to_string())
}

const RECORD: char = '\u{1}';
const FIELD: char = '\u{1f}';
const END: char = '\u{2}';

/// The arguments of `git log` that list the file's commits with what
/// each did to it. A merge is compared with its first parent.
fn log_arguments<'a>(range: &'a [String], path: &'a str) -> Vec<&'a str> {
    let mut arguments = vec![
        "log",
        "--follow",
        "--format=%x01%H%x1f%P%x1f%an%x1f%at%x1f%s%x02",
        "--raw",
        "--no-abbrev",
        "--numstat",
        "-z",
        "--diff-merges=first-parent",
        "--no-color",
        "--no-ext-diff",
        "--no-textconv",
    ];
    arguments.extend(range.iter().map(String::as_str));
    arguments.push("--");
    arguments.push(path);
    arguments
}

/// `limit` commits that touched the file at `path`, after the `skip`
/// newest ones, going back from the commit `from` (HEAD without), where
/// the file has that name.
pub fn commits(
    git: &dyn Git,
    from: Option<&str>,
    path: &str,
    skip: usize,
    limit: usize,
) -> Result<Page, GitError> {
    let none = Page {
        points: Vec::new(),
        more: false,
    };
    let Some(head) = commit_id(git, "HEAD")? else {
        return Ok(none);
    };
    let start = match from {
        Some(from) => match commit_id(git, from)? {
            Some(start) => start,
            None => return Ok(none),
        },
        None => head.clone(),
    };
    // One more than asked tells whether older ones are left.
    let range = [
        format!("--skip={skip}"),
        format!("--max-count={}", limit + 1),
        start,
    ];
    let output = git.run(&log_arguments(&range, path))?;
    let mut points = parse_log(&output, &head);
    let more = points.len() > limit;
    points.truncate(limit);
    Ok(Page { points, more })
}

/// How many commits touched the file at `path`, under all its names,
/// going back from `from` (HEAD without).
pub fn count(git: &dyn Git, from: Option<&str>, path: &str) -> Result<usize, GitError> {
    let Some(start) = commit_id(git, from.unwrap_or("HEAD"))? else {
        return Ok(0);
    };
    let output = git.run(&["log", "--follow", "--format=%x01", &start, "--", path])?;
    Ok(output.matches(RECORD).count())
}

/// The output of `git log` as `log_arguments` asks for it: per commit a
/// record of fields, then the file's `--raw --numstat -z` entry.
pub fn parse_log(text: &str, head: &str) -> Vec<Point> {
    text.split(RECORD)
        .filter_map(|record| {
            let (fields, change) = record.split_once(END)?;
            let mut fields = fields.split(FIELD);
            let id = fields.next()?.to_string();
            // Anything else is not git's output for a commit.
            if id.is_empty() || !id.chars().all(|c| c.is_ascii_hexdigit()) {
                return None;
            }
            let parents = fields.next()?;
            let author = fields.next()?.to_string();
            let time = fields.next()?.parse().ok()?;
            let subject = fields.next()?.to_string();
            let change = change.trim_start_matches(['\0', '\n']);
            // A commit git lists with no change to the file (a merge that
            // took the other side whole) is still a point.
            let file = parse_changes(change).into_iter().next();
            Some(Point {
                kind: PointKind::Commit,
                head: id == head,
                parent: parents
                    .split(' ')
                    .next()
                    .filter(|id| !id.is_empty())
                    .map(str::to_string),
                id,
                subject,
                author,
                time,
                path: file
                    .as_ref()
                    .map(|file| file.path.clone())
                    .unwrap_or_default(),
                old_path: file.as_ref().and_then(|file| file.old_path.clone()),
                added: file.as_ref().and_then(|file| file.added),
                removed: file.as_ref().and_then(|file| file.removed),
            })
        })
        .collect()
}

/// The empty tree's id, to compare a first commit with.
fn empty_tree(git: &dyn Git) -> Result<String, GitError> {
    let tree = git.run_with_input(&["hash-object", "-t", "tree", "--stdin"], b"")?;
    Ok(tree.trim().to_string())
}

/// What a point is compared with, and the names the file can have on the
/// two sides. `None` when the point is the pin itself.
///
/// Without a pin: the commit against its first parent, the index against
/// HEAD, the working copy against the index. With one, the point against
/// the pinned side, the older of the two first; `order` says where the
/// point stands relative to a pinned commit.
pub fn comparison(
    git: &dyn Git,
    point: &Point,
    pin: Option<&Pin>,
    order: Order,
) -> Result<Option<(Comparison, Vec<String>)>, GitError> {
    let mut paths = vec![point.path.clone()];
    let mut also = |path: &str| {
        if !paths.iter().any(|known| known == path) {
            paths.push(path.to_string());
        }
    };
    if let Some(old_path) = &point.old_path {
        also(old_path);
    }
    let commit = |id: &str| id.to_string();
    let comparison = match (point.kind, pin) {
        (PointKind::Commit, None) => Comparison::Commits {
            old: match &point.parent {
                Some(parent) => commit(parent),
                None => empty_tree(git)?,
            },
            new: commit(&point.id),
        },
        (PointKind::Staged, None) => Comparison::staged(git)?,
        (PointKind::NotCommitted, None) => Comparison::Unstaged,

        (PointKind::NotCommitted, Some(Pin::WorkingCopy { .. })) => return Ok(None),
        (PointKind::Staged, Some(Pin::WorkingCopy { path })) => {
            also(path);
            Comparison::Unstaged
        }
        (PointKind::Commit, Some(Pin::WorkingCopy { path })) => {
            also(path);
            Comparison::WorkingCopy {
                base: commit(&point.id),
            }
        }

        (PointKind::NotCommitted, Some(Pin::Commit { id, path })) => {
            also(path);
            Comparison::WorkingCopy { base: commit(id) }
        }
        (PointKind::Staged, Some(Pin::Commit { id, path })) => {
            also(path);
            Comparison::Staged { base: commit(id) }
        }
        (PointKind::Commit, Some(Pin::Commit { id, path })) => {
            if *id == point.id {
                return Ok(None);
            }
            also(path);
            match order {
                Order::Older => Comparison::Commits {
                    old: commit(&point.id),
                    new: commit(id),
                },
                Order::Newer => Comparison::Commits {
                    old: commit(id),
                    new: commit(&point.id),
                },
            }
        }
    };
    Ok(Some((comparison, paths)))
}

/// How the file at a point is read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Sides {
    /// As a file of a change: git lists it, renamed or not.
    Compared {
        comparison: Comparison,
        /// The names the file can have on the two sides, the point's own
        /// first.
        paths: Vec<String>,
    },
    /// Two versions under two names, compared by their content. Far
    /// apart in the history, git no longer finds them alike enough to
    /// call the file renamed, and would show one deleted and one added.
    Blobs {
        /// The old side's blob, `commit:path`.
        old: String,
        /// The new side's blob; `None` for the file in the working copy.
        new: Option<String>,
        old_path: String,
        new_path: String,
    },
}

/// How to read the file at `point`, alone or against `pin`. `None` when
/// the point is the pin itself.
pub fn sides(
    git: &dyn Git,
    root: &Path,
    point: &Point,
    pin: Option<&Pin>,
    order: Order,
) -> Result<Option<Sides>, GitError> {
    let Some((comparison, paths)) = comparison(git, point, pin, order)? else {
        return Ok(None);
    };
    let Some(pin) = pin else {
        if point.kind == PointKind::NotCommitted
            && diff::changed_files(git, root, &comparison, &paths)?.is_empty()
        {
            // An untracked file is not in the index: it is listed with
            // the working copy against a commit.
            return Ok(Some(Sides::Compared {
                comparison: Comparison::uncommitted(git)?,
                paths,
            }));
        }
        return Ok(Some(Sides::Compared { comparison, paths }));
    };
    // Each side's blob (`None` in the working copy) and path.
    let of_point = match point.kind {
        PointKind::Commit => (Some(format!("{}:{}", point.id, point.path)), &point.path),
        PointKind::Staged => (Some(format!(":0:{}", point.path)), &point.path),
        PointKind::NotCommitted => (None, &point.path),
    };
    let (of_pin, pin_is_new) = match pin {
        Pin::WorkingCopy { path } => ((None, path), true),
        Pin::Commit { id, path } => (
            (Some(format!("{id}:{path}")), path),
            point.kind == PointKind::Commit && order == Order::Older,
        ),
    };
    let (old, new) = if pin_is_new {
        (of_point, of_pin)
    } else {
        (of_pin, of_point)
    };
    let exists = |blob: &str| -> Result<bool, GitError> {
        Ok(git.output(&["cat-file", "-e", blob])?.status == Some(0))
    };
    if old.1 != new.1
        && let (Some(old_blob), new_blob) = (&old.0, &new.0)
        && exists(old_blob)?
        && match new_blob {
            Some(blob) => exists(blob)?,
            None => root.join(new.1).is_file(),
        }
    {
        return Ok(Some(Sides::Blobs {
            old: old_blob.clone(),
            new: new_blob.clone(),
            old_path: old.1.clone(),
            new_path: new.1.clone(),
        }));
    }
    Ok(Some(Sides::Compared { comparison, paths }))
}

/// The file's entry (state, names, counts) as `sides` reads it. `None`
/// when nothing differs.
pub fn sides_file(
    git: &dyn Git,
    root: &Path,
    sides: &Sides,
) -> Result<Option<ChangedFile>, GitError> {
    match sides {
        Sides::Compared { comparison, paths } => {
            let files = diff::changed_files(git, root, comparison, paths)?;
            // One entry when git pairs the two names as a rename;
            // otherwise the file under the point's own name.
            Ok(paths
                .iter()
                .find_map(|path| files.iter().find(|file| file.path == *path))
                .or(files.first())
                .cloned())
        }
        Sides::Blobs {
            old,
            new,
            old_path,
            new_path,
        } => {
            let Some((added, removed)) = diff::blob_counts(git, old, new.as_deref(), new_path)?
            else {
                return Ok(None);
            };
            let slash = new_path.rfind('/').map_or(0, |slash| slash + 1);
            Ok(Some(ChangedFile {
                path: new_path.clone(),
                old_path: Some(old_path.clone()),
                name: new_path[slash..].to_string(),
                directory: new_path[..slash].to_string(),
                state: FileState::Renamed,
                binary: added.is_none(),
                submodule: false,
                added,
                removed,
                group: None,
                partly_staged: false,
                old_id: None,
                new_id: None,
            }))
        }
    }
}

/// The diff of `file`, the entry `sides_file` gave.
pub fn sides_diff(
    git: &dyn Git,
    root: &Path,
    sides: &Sides,
    file: &ChangedFile,
    marks: Marks,
) -> Result<FileDiff, DiffError> {
    match sides {
        Sides::Compared { comparison, .. } => diff::file_diff(git, root, comparison, file, marks),
        Sides::Blobs {
            old, new, new_path, ..
        } => diff::blob_diff(git, root, old, new.as_deref(), new_path, marks),
    }
}

/// The file as it differs at a point: how it is read and its entry.
/// `None` when nothing differs: the point is the pin, or the file is the
/// same on both sides.
pub fn point_file(
    git: &dyn Git,
    root: &Path,
    point: &Point,
    pin: Option<&Pin>,
    order: Order,
) -> Result<Option<(Sides, ChangedFile)>, GitError> {
    let Some(sides) = sides(git, root, point, pin, order)? else {
        return Ok(None);
    };
    Ok(sides_file(git, root, &sides)?.map(|file| (sides, file)))
}

/// The diff of the file at a point, as `point_file` finds it.
pub fn point_diff(
    git: &dyn Git,
    root: &Path,
    point: &Point,
    pin: Option<&Pin>,
    order: Order,
    marks: Marks,
) -> Result<Option<(ChangedFile, FileDiff)>, DiffError> {
    let Some((sides, file)) = point_file(git, root, point, pin, order)? else {
        return Ok(None);
    };
    let read = sides_diff(git, root, &sides, &file, marks)?;
    Ok(Some((file, read)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_log_is_read_into_points_with_their_change() {
        let text = "\u{1}aaa\u{1f}bbb ccc\u{1f}Alex Martin\u{1f}1700000000\u{1f}Rename it\u{2}\0\n\
                    :100644 100644 111 222 R091\0old.rs\0new.rs\x001\t2\t\0old.rs\0new.rs\0\
                    \u{1}bbb\u{1f}\u{1f}Sam Lee\u{1f}1600000000\u{1f}Start\u{2}\0\n\
                    :000000 100644 000 111 A\0old.rs\x0015\t0\told.rs\0";
        let points = parse_log(text, "aaa");
        assert_eq!(points.len(), 2);
        let first = &points[0];
        assert_eq!(first.kind, PointKind::Commit);
        assert!(first.head);
        assert_eq!(first.parent.as_deref(), Some("bbb"));
        assert_eq!(
            (first.author.as_str(), first.time),
            ("Alex Martin", 1_700_000_000)
        );
        assert_eq!(first.subject, "Rename it");
        assert_eq!(first.path, "new.rs");
        assert_eq!(first.old_path.as_deref(), Some("old.rs"));
        assert_eq!((first.added, first.removed), (Some(1), Some(2)));
        let second = &points[1];
        assert!(!second.head);
        assert_eq!(second.parent, None);
        assert_eq!(second.path, "old.rs");
        assert_eq!((second.added, second.removed), (Some(15), Some(0)));
    }

    #[test]
    fn cut_or_foreign_output_gives_the_points_read_so_far() {
        assert!(parse_log("", "x").is_empty());
        assert!(parse_log("fatal: not a log", "x").is_empty());
        let cut = "\u{1}aaa\u{1f}\u{1f}A\u{1f}1\u{1f}s\u{2}\0\u{1}bbb\u{1f}";
        let points = parse_log(cut, "x");
        assert_eq!(points.len(), 1);
        assert_eq!(points[0].path, "");
        assert_eq!(points[0].short_id(), "aaa");
    }

    /// Found by fuzzing: a record whose id is not a commit's (here with a
    /// letter of two bytes before the seventh) was kept, and cutting its
    /// id to seven characters panicked.
    #[test]
    fn a_record_without_a_commit_id_is_left_out() {
        let text = "\u{1}fe436fǜ94\u{1f}\u{1f}A\u{1f}1\u{1f}s\u{2}\0\
                    \u{1}\u{1f}\u{1f}A\u{1f}1\u{1f}s\u{2}\0\
                    \u{1}abc1234ff\u{1f}\u{1f}A\u{1f}1\u{1f}s\u{2}\0";
        let points = parse_log(text, "x");
        assert_eq!(points.len(), 1);
        assert_eq!(points[0].short_id(), "abc1234");
    }
}
