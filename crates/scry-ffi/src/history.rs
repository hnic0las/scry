//! One file's history for the Changes window: its points, and the file
//! at each one as a `ChangeSet` of a single file (`scry_core::history`
//! does the reading).

use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};

use scry_core::git::{Git, GitCli, GitError};
use scry_core::history::{self, Order, Pin, Point, PointKind};
use scry_core::overview;

use crate::ScryError;
use crate::changes::{ChangeSet, ChangesHeader};

/// What the history is of.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct HistoryHeader {
    /// The repository folder's name.
    pub repository: String,
    pub root: String,
    /// The file's path from the repository's root, as it was asked.
    pub path: String,
    pub name: String,
    /// The user's name in git, to say "You" for their commits; empty
    /// when it is not set.
    pub me: String,
    /// The history has what is not committed before its commits.
    pub from_head: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum HistoryPointKind {
    NotCommitted,
    Staged,
    Commit,
}

/// A point of the history, newest first.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct HistoryPoint {
    /// Its place in the list: what `point_changes` takes.
    pub index: u32,
    pub kind: HistoryPointKind,
    /// The commit's id; empty off a commit.
    pub id: String,
    pub short_id: String,
    pub subject: String,
    pub author: String,
    /// When the commit was written, in seconds since 1970.
    pub time: i64,
    /// The file's path at this point.
    pub path: String,
    /// The path before, when this point renamed the file.
    pub old_path: Option<String>,
    /// Lines this point added and removed; `None` when not counted.
    pub added: Option<u32>,
    pub removed: Option<u32>,
    /// The commit is HEAD.
    pub head: bool,
}

/// The points loaded so far, and whether older commits are left.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct HistoryList {
    pub points: Vec<HistoryPoint>,
    pub more: bool,
}

/// The side held still in "Against a pin".
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum HistoryPin {
    WorkingCopy,
    /// A commit of the list, by its place.
    Point {
        index: u32,
    },
}

/// What differs between a point and the pin.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Record)]
pub struct PointCounts {
    /// `None` when not counted: a binary file.
    pub added: Option<u32>,
    pub removed: Option<u32>,
    /// Nothing differs, or the point is the pin.
    pub same: bool,
}

/// A pin, with the commit it names when it names one.
type Pinned = (Pin, Option<Point>);

struct Loaded {
    points: Vec<Point>,
    /// The name the commits are followed from.
    followed: String,
    /// How many of the points are commits.
    commits: usize,
    more: bool,
}

/// The history of one file of a repository.
#[derive(uniffi::Object)]
pub struct FileHistory {
    git: GitCli,
    root: PathBuf,
    path: String,
    /// The commit to go back from; `None` for HEAD and what is not
    /// committed.
    from: Option<String>,
    loaded: Mutex<Loaded>,
}

#[uniffi::export]
impl FileHistory {
    /// The history of the file at `file` (from the repository's root) in
    /// the repository holding `path`, going back from the commit `from`,
    /// or from what is not committed without one.
    #[uniffi::constructor]
    pub fn open(
        path: String,
        file: String,
        from: Option<String>,
    ) -> Result<Arc<FileHistory>, ScryError> {
        let probe = GitCli::discover(PathBuf::from(&path));
        let root = overview::root(&probe).map_err(|error| match error {
            GitError::Spawn(_) => ScryError::from(error),
            _ => ScryError::NotARepository {
                message: format!("{path} is not in a git repository."),
            },
        })?;
        Ok(Arc::new(FileHistory {
            git: GitCli::discover(root.clone()),
            root,
            loaded: Mutex::new(Loaded {
                points: Vec::new(),
                followed: file.clone(),
                commits: 0,
                more: false,
            }),
            path: file,
            from,
        }))
    }

    pub fn header(&self) -> HistoryHeader {
        let name = |path: &std::path::Path| {
            path.file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default()
        };
        HistoryHeader {
            repository: name(&self.root),
            root: self.root.display().to_string(),
            name: name(std::path::Path::new(&self.path)),
            path: self.path.clone(),
            me: self
                .git
                .run(&["config", "user.name"])
                .map(|name| name.trim().to_string())
                .unwrap_or_default(),
            from_head: self.from.is_none(),
        }
    }

    /// Reads the history from its newest point: what is not committed,
    /// then the first page of commits.
    pub fn start(&self) -> Result<HistoryList, ScryError> {
        let mut points = match self.from {
            None => history::uncommitted(&self.git, &self.root, &self.path)?,
            Some(_) => Vec::new(),
        };
        let followed = history::committed_path(&points, &self.path);
        let page = history::commits(&self.git, self.from.as_deref(), &followed, 0, history::PAGE)?;
        let commits = page.points.len();
        points.extend(page.points);
        let mut loaded = self.lock();
        *loaded = Loaded {
            points,
            followed,
            commits,
            more: page.more,
        };
        Ok(list(&loaded))
    }

    /// Adds the next page of older commits to the list.
    pub fn load_older(&self) -> Result<HistoryList, ScryError> {
        let (followed, skip) = {
            let loaded = self.lock();
            if !loaded.more {
                return Ok(list(&loaded));
            }
            (loaded.followed.clone(), loaded.commits)
        };
        let page = history::commits(
            &self.git,
            self.from.as_deref(),
            &followed,
            skip,
            history::PAGE,
        )?;
        let mut loaded = self.lock();
        // Another call may have added the same page meanwhile.
        if loaded.commits == skip {
            loaded.commits += page.points.len();
            loaded.points.extend(page.points);
            loaded.more = page.more;
        }
        Ok(list(&loaded))
    }

    /// How many commits touched the file, under all its names. Walks the
    /// whole history: call it off the main thread.
    pub fn count(&self) -> Result<u32, ScryError> {
        let followed = self.lock().followed.clone();
        Ok(history::count(&self.git, self.from.as_deref(), &followed)? as u32)
    }

    /// The file at the point `index` of the list, as a change of one
    /// file: against the point before it, or with `pin` against the
    /// pinned side.
    pub fn point_changes(
        &self,
        index: u32,
        pin: Option<HistoryPin>,
    ) -> Result<Arc<ChangeSet>, ScryError> {
        let (point, pinned, order) = self.asked(index, pin)?;
        let sides = history::sides(
            &self.git,
            &self.root,
            &point,
            pinned.as_ref().map(|pin| &pin.0),
            order,
        )?;
        let (old_label, new_label, sentence) = labels(&point, pinned.as_ref(), order);
        let header = self.header();
        Ok(ChangeSet::point(
            self.git.clone(),
            self.root.clone(),
            sides,
            ChangesHeader {
                repository: header.repository,
                root: header.root,
                title: format!("History of {}", header.name),
                detail: sentence.clone(),
                sentence: format!("History of {} · {sentence}", header.name),
                branch: None,
                old_label,
                new_label,
                grouped: false,
                live: false,
                reduced: false,
                single_file: true,
                git_dir: String::new(),
                has_history: false,
                history_from: None,
            },
        ))
    }

    /// What differs between the point `index` and `pin`, for the list.
    pub fn pinned_counts(&self, index: u32, pin: HistoryPin) -> Result<PointCounts, ScryError> {
        let (point, pinned, order) = self.asked(index, Some(pin))?;
        let pin = pinned.as_ref().map(|pin| &pin.0);
        let file = history::point_file(&self.git, &self.root, &point, pin, order)?;
        Ok(match file {
            Some((_, file)) => PointCounts {
                added: file.added.map(|count| count as u32),
                removed: file.removed.map(|count| count as u32),
                same: false,
            },
            None => PointCounts {
                added: Some(0),
                removed: Some(0),
                same: true,
            },
        })
    }
}

impl FileHistory {
    fn lock(&self) -> MutexGuard<'_, Loaded> {
        // The list is replaced or extended whole: a panic elsewhere
        // leaves it usable.
        self.loaded
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn working_copy(&self) -> Pin {
        Pin::WorkingCopy {
            path: self.path.clone(),
        }
    }

    /// The point at `index`, the pin with the point it names, and where
    /// the point stands relative to it.
    fn asked(
        &self,
        index: u32,
        pin: Option<HistoryPin>,
    ) -> Result<(Point, Option<Pinned>, Order), ScryError> {
        let gone = || ScryError::Refused {
            message: String::from("This point is no longer in the history."),
        };
        let loaded = self.lock();
        let point = loaded
            .points
            .get(index as usize)
            .cloned()
            .ok_or_else(gone)?;
        let mut order = Order::Older;
        let pinned = match pin {
            None => None,
            Some(HistoryPin::WorkingCopy) => Some((self.working_copy(), None)),
            Some(HistoryPin::Point { index: pinned }) => {
                let at = loaded
                    .points
                    .get(pinned as usize)
                    .cloned()
                    .ok_or_else(gone)?;
                if index < pinned {
                    order = Order::Newer;
                }
                match at.kind {
                    PointKind::NotCommitted => Some((self.working_copy(), None)),
                    PointKind::Staged => {
                        return Err(ScryError::Refused {
                            message: String::from("What is staged cannot be pinned."),
                        });
                    }
                    PointKind::Commit => Some((
                        Pin::Commit {
                            id: at.id.clone(),
                            path: at.path.clone(),
                        },
                        Some(at),
                    )),
                }
            }
        };
        Ok((point, pinned, order))
    }
}

fn list(loaded: &Loaded) -> HistoryList {
    HistoryList {
        points: loaded
            .points
            .iter()
            .enumerate()
            .map(|(index, point)| HistoryPoint {
                index: index as u32,
                kind: match point.kind {
                    PointKind::NotCommitted => HistoryPointKind::NotCommitted,
                    PointKind::Staged => HistoryPointKind::Staged,
                    PointKind::Commit => HistoryPointKind::Commit,
                },
                id: point.id.clone(),
                short_id: point.short_id().to_string(),
                subject: point.subject.clone(),
                author: point.author.clone(),
                time: point.time,
                path: point.path.clone(),
                old_path: point.old_path.clone(),
                added: point.added.map(|count| count as u32),
                removed: point.removed.map(|count| count as u32),
                head: point.head,
            })
            .collect(),
        more: loaded.more,
    }
}

/// How the compare bar names a point: "9f8e7d6 Retry the upload".
fn name(point: &Point) -> String {
    match point.kind {
        PointKind::NotCommitted => String::from("Working copy"),
        PointKind::Staged => String::from("Index"),
        PointKind::Commit if point.head => format!("{} HEAD · {}", point.short_id(), point.subject),
        PointKind::Commit => format!("{} {}", point.short_id(), point.subject),
    }
}

/// The old side's name, the new side's, and what the comparison shows.
fn labels(point: &Point, pin: Option<&Pinned>, order: Order) -> (String, String, String) {
    let Some((pin, at)) = pin else {
        return match point.kind {
            PointKind::NotCommitted => (
                String::from("Index"),
                String::from("Working copy"),
                String::from("what is not staged"),
            ),
            PointKind::Staged => (
                String::from("HEAD"),
                String::from("Index"),
                String::from("what is staged"),
            ),
            PointKind::Commit => (
                match &point.parent {
                    Some(parent) => format!("{} parent", &parent[..parent.len().min(7)]),
                    None => String::from("nothing: the first commit"),
                },
                name(point),
                String::from("what this commit changed"),
            ),
        };
    };
    let pinned = match (pin, at) {
        (Pin::Commit { .. }, Some(at)) => format!("Pinned: {}", name(at)),
        _ => String::from("Pinned: Working copy"),
    };
    // The older side comes first, whatever is pinned.
    let pin_is_new = match pin {
        Pin::WorkingCopy { .. } => true,
        Pin::Commit { .. } => point.kind == PointKind::Commit && order == Order::Older,
    };
    if pin_is_new {
        (
            name(point),
            pinned,
            String::from("everything since this point"),
        )
    } else {
        (
            pinned,
            name(point),
            String::from("everything since the pin"),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::process::Command;

    fn git(dir: &std::path::Path, args: &[&str]) {
        let status = Command::new("git")
            .current_dir(dir)
            .args([
                "-c",
                "user.name=Scry Test",
                "-c",
                "user.email=test@example.com",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(args)
            .status();
        assert!(status.unwrap().success(), "git {args:?}");
    }

    /// The window gets the points of a file, the file at each one as a
    /// change of one file with both sides named, and the counts against
    /// a pin.
    #[test]
    fn a_history_gives_its_points_and_the_file_at_each() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        git(root, &["init", "--quiet", "--initial-branch=main"]);
        fs::write(root.join("a.txt"), "one\ntwo\nthree\nfour\n").unwrap();
        git(root, &["add", "."]);
        git(root, &["commit", "--quiet", "-m", "Start"]);
        fs::write(root.join("a.txt"), "one\ntwo\nthree\nfour\nfive\n").unwrap();
        git(root, &["commit", "--quiet", "-am", "Add five"]);
        fs::write(root.join("a.txt"), "one\ntwo\nthree\nfour\nfive\nsix\n").unwrap();

        let path = root.display().to_string();
        let history = FileHistory::open(path.clone(), String::from("a.txt"), None).unwrap();
        let header = history.header();
        assert_eq!((header.name.as_str(), header.from_head), ("a.txt", true));
        let list = history.start().unwrap();
        assert!(!list.more);
        let seen: Vec<_> = list
            .points
            .iter()
            .map(|point| (point.index, point.kind, point.subject.as_str(), point.head))
            .collect();
        assert_eq!(
            seen,
            [
                (0, HistoryPointKind::NotCommitted, "", false),
                (1, HistoryPointKind::Commit, "Add five", true),
                (2, HistoryPointKind::Commit, "Start", false),
            ]
        );
        assert_eq!(history.count().unwrap(), 2);
        assert_eq!(history.load_older().unwrap().points.len(), 3);

        // Each change: the commit against its parent.
        let changes = history.point_changes(1, None).unwrap();
        let shown = changes.header();
        assert!(shown.single_file && !shown.has_history && !shown.live);
        assert!(shown.old_label.ends_with(" parent"));
        assert!(shown.new_label.ends_with("HEAD · Add five"));
        assert_eq!(
            shown.sentence,
            "History of a.txt · what this commit changed"
        );
        let files = changes.files().unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!((files[0].added, files[0].removed), (Some(1), Some(0)));
        assert!(matches!(
            changes.file_diff(0, false).unwrap(),
            crate::FileDiffInfo::Text { .. }
        ));
        let first = history.point_changes(2, None).unwrap().header();
        assert_eq!(first.old_label, "nothing: the first commit");

        // Against the newest commit, pinned: the older side first.
        let pin = HistoryPin::Point { index: 1 };
        let pinned = history.point_changes(2, Some(pin)).unwrap();
        assert!(pinned.header().new_label.starts_with("Pinned: "));
        assert_eq!(pinned.files().unwrap()[0].added, Some(1));
        let newer = history.point_changes(0, Some(pin)).unwrap();
        assert!(newer.header().old_label.starts_with("Pinned: "));
        assert_eq!(newer.header().new_label, "Working copy");
        // The pin itself has nothing to show.
        let itself = history.point_changes(1, Some(pin)).unwrap();
        assert!(itself.files().unwrap().is_empty());
        assert!(history.pinned_counts(1, pin).unwrap().same);
        let counts = history.pinned_counts(2, HistoryPin::WorkingCopy).unwrap();
        assert_eq!(
            (counts.added, counts.removed, counts.same),
            (Some(2), Some(0), false)
        );
        assert!(history.point_changes(9, None).is_err());

        // From a commit: no point for what is not committed.
        let from = FileHistory::open(
            path.clone(),
            String::from("a.txt"),
            Some(String::from("HEAD~1")),
        );
        let list = from.unwrap().start().unwrap();
        assert_eq!(list.points.len(), 1);
        assert_eq!(list.points[0].subject, "Start");

        // The whole commit.
        let commit = ChangeSet::open_commit(path, list.points[0].id.clone()).unwrap();
        assert!(commit.header().title.starts_with("Commit "));
        assert!(commit.header().history_from.is_some());
        assert_eq!(commit.files().unwrap().len(), 1);
    }
}
