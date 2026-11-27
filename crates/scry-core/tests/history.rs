//! One file's history read with the `git` executable, on real
//! repositories in temporary directories: its points, and what each one
//! changed, alone or against a pin.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

use scry_core::diff::{Comparison, FileDiff, FileState, Marks, Scope};
use scry_core::git::GitCli;
use scry_core::history::Sides;
use scry_core::history::{self, Order, Pin, Point, PointKind};
use tempfile::TempDir;

/// A throwaway repository whose git commands ignore the user's own
/// configuration (signing, hooks, default branch).
struct Repo {
    root: TempDir,
}

impl Repo {
    fn new() -> Repo {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("gitconfig"), "").unwrap();
        fs::create_dir(root.path().join("repo")).unwrap();
        let repo = Repo { root };
        repo.git(&["init", "--quiet", "--initial-branch=main"]);
        repo
    }

    fn path(&self) -> PathBuf {
        self.root.path().join("repo")
    }

    fn git(&self, args: &[&str]) {
        let done = Command::new("git")
            .arg("-C")
            .arg(self.path())
            .args(args)
            .env("GIT_CONFIG_GLOBAL", self.root.path().join("gitconfig"))
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_AUTHOR_NAME", "Alex Martin")
            .env("GIT_AUTHOR_EMAIL", "alex@example.com")
            .env("GIT_COMMITTER_NAME", "Alex Martin")
            .env("GIT_COMMITTER_EMAIL", "alex@example.com")
            .output()
            .unwrap();
        assert!(done.status.success(), "git {args:?}");
    }

    fn write(&self, path: &str, content: &str) {
        let path = self.path().join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }

    /// Writes the file, stages it and commits.
    fn commit(&self, path: &str, content: &str, message: &str) {
        self.write(path, content);
        self.git(&["add", "--", path]);
        self.git(&["commit", "--quiet", "-m", message]);
    }

    fn scry_git(&self) -> GitCli {
        GitCli::new(self.path())
            .with_env(
                "GIT_CONFIG_GLOBAL",
                &self.root.path().join("gitconfig").display().to_string(),
            )
            .with_env("GIT_CONFIG_NOSYSTEM", "1")
    }

    /// Every point of the file at `path`: what is not committed, then
    /// its commits.
    fn points(&self, path: &str) -> Vec<Point> {
        let git = self.scry_git();
        let mut points = history::uncommitted(&git, &self.path(), path).unwrap();
        let followed = history::committed_path(&points, path);
        points.extend(
            history::commits(&git, None, &followed, 0, history::PAGE)
                .unwrap()
                .points,
        );
        points
    }

    /// "+2 −1" lines of the file's diff at `point`, or "same".
    fn counts(&self, point: &Point, pin: Option<&Pin>, order: Order) -> String {
        let read = history::point_diff(
            &self.scry_git(),
            &self.path(),
            point,
            pin,
            order,
            Marks::Auto,
        )
        .unwrap();
        match read {
            Some((_, FileDiff::Text(text))) => {
                let (added, removed) = text.counts();
                format!("+{added} −{removed}")
            }
            Some(other) => format!("{other:?}"),
            None => String::from("same"),
        }
    }
}

const ONE: &str = "one\ntwo\nthree\nfour\nfive\nsix\nseven\neight\n";
const TWO: &str = "one\ntwo\nthree\nfour\nfive\nsix\nseven\neight\nnine\n";
const THREE: &str = "one\nTWO\nthree\nfour\nfive\nsix\nseven\neight\nnine\n";

/// Three commits on one file, renamed by the second.
fn renamed() -> Repo {
    let repo = Repo::new();
    repo.commit("src/old.txt", ONE, "Start");
    repo.commit("other.txt", "x\n", "Something else");
    repo.git(&["mv", "src/old.txt", "src/new.txt"]);
    repo.commit("src/new.txt", TWO, "Rename and add a line");
    repo.commit("src/new.txt", THREE, "Shout");
    repo
}

#[test]
fn the_commits_that_touched_a_file_are_followed_through_its_rename() {
    let repo = renamed();
    let points = repo.points("src/new.txt");
    let subjects: Vec<&str> = points.iter().map(|point| point.subject.as_str()).collect();
    assert_eq!(subjects, ["Shout", "Rename and add a line", "Start"]);
    assert!(points.iter().all(|point| point.kind == PointKind::Commit));
    assert!(points[0].head && !points[1].head);
    assert_eq!(points[0].author, "Alex Martin");
    assert!(points[0].time > 1_600_000_000);
    assert_eq!(points[0].parent.as_deref(), Some(points[1].id.as_str()));
    assert_eq!((points[0].added, points[0].removed), (Some(1), Some(1)));
    assert_eq!(points[1].path, "src/new.txt");
    assert_eq!(points[1].old_path.as_deref(), Some("src/old.txt"));
    assert_eq!((points[1].added, points[1].removed), (Some(1), Some(0)));
    assert_eq!(points[2].path, "src/old.txt");
    assert_eq!(points[2].parent, None);
    assert_eq!((points[2].added, points[2].removed), (Some(8), Some(0)));
    assert_eq!(
        history::count(&repo.scry_git(), None, "src/new.txt").unwrap(),
        3
    );
}

#[test]
fn each_point_shows_what_it_changed() {
    let repo = renamed();
    let points = repo.points("src/new.txt");
    assert_eq!(repo.counts(&points[0], None, Order::Older), "+1 −1");
    assert_eq!(repo.counts(&points[1], None, Order::Older), "+1 −0");
    // The first commit, against nothing.
    assert_eq!(repo.counts(&points[2], None, Order::Older), "+8 −0");
    let (sides, file) = history::point_file(
        &repo.scry_git(),
        &repo.path(),
        &points[1],
        None,
        Order::Older,
    )
    .unwrap()
    .unwrap();
    assert!(matches!(
        sides,
        Sides::Compared {
            comparison: Comparison::Commits { .. },
            ..
        }
    ));
    assert_eq!(file.state, FileState::Renamed);
    assert_eq!(file.old_path.as_deref(), Some("src/old.txt"));
}

#[test]
fn a_pin_shows_everything_between_it_and_each_point() {
    let repo = renamed();
    let points = repo.points("src/new.txt");
    let pin = |index: usize| Pin::Commit {
        id: points[index].id.clone(),
        path: points[index].path.clone(),
    };
    // Against the newest commit: the older point first.
    assert_eq!(
        repo.counts(&points[2], Some(&pin(0)), Order::Older),
        "+2 −1"
    );
    assert_eq!(
        repo.counts(&points[1], Some(&pin(0)), Order::Older),
        "+1 −1"
    );
    assert_eq!(repo.counts(&points[0], Some(&pin(0)), Order::Older), "same");
    // Against the oldest: the pin is the old side.
    assert_eq!(
        repo.counts(&points[0], Some(&pin(2)), Order::Newer),
        "+2 −1"
    );
    let (_, file) = history::point_file(
        &repo.scry_git(),
        &repo.path(),
        &points[0],
        Some(&pin(2)),
        Order::Newer,
    )
    .unwrap()
    .unwrap();
    assert_eq!(file.path, "src/new.txt");
    assert_eq!(file.old_path.as_deref(), Some("src/old.txt"));
}

#[test]
fn what_is_not_committed_comes_first() {
    let repo = renamed();
    repo.write("src/new.txt", &THREE.replace("nine", "NINE"));
    repo.git(&["add", "--", "src/new.txt"]);
    repo.write("src/new.txt", &THREE.replace("nine", "NINE\nten"));
    let points = repo.points("src/new.txt");
    let kinds: Vec<PointKind> = points.iter().map(|point| point.kind).collect();
    assert_eq!(
        kinds[..3],
        [
            PointKind::NotCommitted,
            PointKind::Staged,
            PointKind::Commit
        ]
    );
    assert_eq!(points.len(), 5);
    assert_eq!((points[0].added, points[0].removed), (Some(1), Some(0)));
    assert_eq!((points[1].added, points[1].removed), (Some(1), Some(1)));
    assert_eq!(repo.counts(&points[0], None, Order::Older), "+1 −0");
    assert_eq!(repo.counts(&points[1], None, Order::Older), "+1 −1");
    // Against the working copy.
    let working = Pin::WorkingCopy {
        path: String::from("src/new.txt"),
    };
    assert_eq!(
        repo.counts(&points[0], Some(&working), Order::Older),
        "same"
    );
    assert_eq!(
        repo.counts(&points[1], Some(&working), Order::Older),
        "+1 −0"
    );
    assert_eq!(
        repo.counts(&points[2], Some(&working), Order::Older),
        "+2 −1"
    );
    // Against a commit, what is not committed is the new side.
    let start = Pin::Commit {
        id: points[4].id.clone(),
        path: points[4].path.clone(),
    };
    assert_eq!(repo.counts(&points[0], Some(&start), Order::Newer), "+3 −1");
    assert_eq!(repo.counts(&points[1], Some(&start), Order::Newer), "+2 −1");
}

#[test]
fn a_rename_not_committed_yet_is_followed_under_the_old_name() {
    let repo = renamed();
    repo.git(&["mv", "src/new.txt", "src/newest.txt"]);
    let points = repo.points("src/newest.txt");
    assert_eq!(points[0].kind, PointKind::Staged);
    assert_eq!(points[0].old_path.as_deref(), Some("src/new.txt"));
    assert_eq!(points.len(), 4);
    assert_eq!(points[1].subject, "Shout");
}

#[test]
fn a_file_never_committed_has_one_point() {
    let repo = Repo::new();
    repo.write("new.txt", "a\nb\n");
    // No commit at all.
    let points = repo.points("new.txt");
    assert_eq!(points.len(), 1);
    assert_eq!(points[0].kind, PointKind::NotCommitted);
    assert_eq!(repo.counts(&points[0], None, Order::Older), "+2 −0");
    assert_eq!(
        history::count(&repo.scry_git(), None, "new.txt").unwrap(),
        0
    );
    // Untracked beside commits.
    let repo = renamed();
    repo.write("fresh.txt", "a\n");
    let points = repo.points("fresh.txt");
    assert_eq!(points.len(), 1);
    assert_eq!(points[0].added, Some(1));
    // A file that does not change has only its commits.
    assert!(
        repo.points("other.txt")
            .iter()
            .all(|point| point.kind == PointKind::Commit)
    );
}

#[test]
fn commits_come_by_pages_and_from_another_commit() {
    let repo = Repo::new();
    for number in 0..5 {
        repo.commit("a.txt", &format!("{number}\n"), &format!("Change {number}"));
    }
    let git = repo.scry_git();
    let first = history::commits(&git, None, "a.txt", 0, 2).unwrap();
    assert!(first.more);
    let subjects = |points: &[Point]| -> Vec<String> {
        points.iter().map(|point| point.subject.clone()).collect()
    };
    assert_eq!(subjects(&first.points), ["Change 4", "Change 3"]);
    let last = history::commits(&git, None, "a.txt", 4, 2).unwrap();
    assert!(!last.more);
    assert_eq!(subjects(&last.points), ["Change 0"]);
    // From an older commit: the history as it was there.
    let from = history::commits(&git, Some("HEAD~3"), "a.txt", 0, 50).unwrap();
    assert_eq!(subjects(&from.points), ["Change 1", "Change 0"]);
    assert!(!from.points[0].head);
    assert_eq!(history::count(&git, Some("HEAD~3"), "a.txt").unwrap(), 2);
    assert!(
        history::commits(&git, Some("no-such"), "a.txt", 0, 50)
            .unwrap()
            .points
            .is_empty()
    );
}

#[test]
fn a_whole_commit_is_compared_with_its_parent() {
    let repo = renamed();
    let git = repo.scry_git();
    let points = repo.points("src/new.txt");
    let scope = Scope::commit(&git, &points[1].id).unwrap();
    assert_eq!(scope.title, format!("Commit {}", points[1].short_id()));
    assert!(scope.detail.starts_with("Against its parent "));
    assert!(scope.detail.ends_with("· Rename and add a line"));
    let first = Scope::commit(&git, &points[2].id).unwrap();
    assert_eq!(first.detail, "The first commit · Start");
    let files = scry_core::diff::changed_files(&git, &repo.path(), &first.comparison, &[]).unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].state, FileState::Added);
    assert!(Scope::commit(&git, "no-such").is_err());
}

/// Far apart in the history, the file under its old name and under its
/// new one are no longer alike: against a pin they are still one file,
/// compared by content, not one deleted and one added.
#[test]
fn a_pin_across_a_rename_compares_the_content_whatever_git_finds_alike() {
    let repo = Repo::new();
    repo.commit("old.txt", "a\nb\nc\nd\n", "Start");
    repo.git(&["mv", "old.txt", "new.txt"]);
    repo.commit("new.txt", "a\nb\nc\nd\ne\n", "Rename");
    repo.commit("new.txt", "a\nw\nx\ny\nz\n", "Rewrite");
    repo.write("new.txt", "a\nw\nx\ny\nz\nmore\n");
    let points = repo.points("new.txt");
    assert_eq!(points.len(), 4);
    let pin = Pin::Commit {
        id: points[1].id.clone(),
        path: points[1].path.clone(),
    };
    let (sides, file) = history::point_file(
        &repo.scry_git(),
        &repo.path(),
        &points[3],
        Some(&pin),
        Order::Older,
    )
    .unwrap()
    .unwrap();
    assert!(matches!(sides, Sides::Blobs { .. }));
    assert_eq!(file.state, FileState::Renamed);
    assert_eq!(
        (file.path.as_str(), file.old_path.as_deref()),
        ("new.txt", Some("old.txt"))
    );
    assert_eq!((file.added, file.removed), (Some(4), Some(3)));
    assert_eq!(repo.counts(&points[3], Some(&pin), Order::Older), "+4 −3");
    // The working copy pinned, against the first commit.
    let working = Pin::WorkingCopy {
        path: String::from("new.txt"),
    };
    assert_eq!(
        repo.counts(&points[3], Some(&working), Order::Older),
        "+5 −3"
    );
    // The oldest commit pinned: it is the old side.
    let start = Pin::Commit {
        id: points[3].id.clone(),
        path: points[3].path.clone(),
    };
    assert_eq!(repo.counts(&points[0], Some(&start), Order::Newer), "+5 −3");
    assert_eq!(repo.counts(&points[1], Some(&start), Order::Newer), "+4 −3");
}
