//! Detecting the operation in progress on real repositories, built with the
//! `git` executable in temporary directories.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

use std::path::Path;

use scry_core::git::GitCli;
use scry_core::operation::{self, OperationKind, Progress, SideRole};
use scry_core::presentation;
use scry_core::session::MergeSession;
use tempfile::TempDir;

/// A throwaway repository whose git commands ignore the user's own
/// configuration (signing, hooks, default branch).
struct Repo {
    root: TempDir,
}

impl Repo {
    fn new() -> Repo {
        Repo::with_format("files")
    }

    /// A repository storing its refs as `format` says: `files`, or
    /// `reftable`, where HEAD and most pseudorefs are not files.
    fn with_format(format: &str) -> Repo {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("gitconfig"), "").unwrap();
        fs::create_dir(root.path().join("repo")).unwrap();
        let repo = Repo { root };
        let format = format!("--ref-format={format}");
        assert!(repo.git(&["init", "--quiet", "--initial-branch=main", &format]));
        repo
    }

    fn path(&self) -> PathBuf {
        self.root.path().join("repo")
    }

    /// Runs git and tells whether it succeeded.
    fn git(&self, args: &[&str]) -> bool {
        let output = Command::new("git")
            .arg("-C")
            .arg(self.path())
            .args(args)
            .env("GIT_CONFIG_GLOBAL", self.root.path().join("gitconfig"))
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_AUTHOR_NAME", "Scry Test")
            .env("GIT_AUTHOR_EMAIL", "test@example.com")
            .env("GIT_COMMITTER_NAME", "Scry Test")
            .env("GIT_COMMITTER_EMAIL", "test@example.com")
            .env("GIT_EDITOR", "true")
            .output()
            .unwrap();
        output.status.success()
    }

    fn commit(&self, file: &str, content: &str, message: &str) {
        fs::write(self.path().join(file), content).unwrap();
        assert!(self.git(&["add", file]));
        assert!(self.git(&["commit", "--quiet", "-m", message]));
    }

    fn scry_git(&self) -> GitCli {
        GitCli::new(self.path())
            .with_env(
                "GIT_CONFIG_GLOBAL",
                &self.root.path().join("gitconfig").display().to_string(),
            )
            .with_env("GIT_CONFIG_NOSYSTEM", "1")
    }
}

/// main and feature both change the one line of `file.txt`; feature first
/// adds an unrelated file, so a rebase has two commits to replay.
fn diverged() -> Repo {
    diverged_with_format("files")
}

fn diverged_with_format(format: &str) -> Repo {
    let repo = Repo::with_format(format);
    repo.commit("file.txt", "line\n", "base");
    assert!(repo.git(&["switch", "--quiet", "-c", "feature"]));
    repo.commit("other.txt", "other\n", "add other");
    repo.commit("file.txt", "feature line\n", "feature change");
    assert!(repo.git(&["switch", "--quiet", "main"]));
    repo.commit("file.txt", "main line\n", "main change");
    repo
}

/// A repository with nothing in progress reports no operation.
#[test]
fn no_operation() {
    let repo = Repo::new();
    repo.commit("file.txt", "line\n", "base");
    assert_eq!(operation::current(&repo.scry_git()).unwrap(), None);
}

/// A real conflicted merge: the current branch is ours, the merged branch theirs, with
/// their commits and the conflicted file.
#[test]
fn merge() {
    let repo = diverged();
    assert!(repo.git(&["switch", "--quiet", "feature"]));
    assert!(!repo.git(&["merge", "main"]));

    let git = repo.scry_git();
    let operation = operation::current(&git).unwrap().unwrap();
    assert_eq!(operation.kind, OperationKind::Merge);
    assert_eq!(operation.branch.as_deref(), Some("feature"));
    assert_eq!(operation.ours.role, SideRole::CurrentBranch);
    assert_eq!(operation.ours.label(), "feature · current branch");
    assert_eq!(operation.ours.commit.subject, "feature change");
    let theirs = operation.theirs.as_ref().unwrap();
    assert_eq!(theirs.label(), "main · being merged");
    assert_eq!(theirs.commit.subject, "main change");
    assert_eq!(operation.summary(), "Merging main into feature");
    assert_eq!(operation::conflicted_paths(&git).unwrap(), ["file.txt"]);
}

/// A real conflicted rebase: ours is the target plus the commits already replayed, theirs
/// is the user's commit, with the rebase progress.
#[test]
fn rebase() {
    let repo = diverged();
    assert!(repo.git(&["switch", "--quiet", "feature"]));
    assert!(!repo.git(&["rebase", "main"]));

    let git = repo.scry_git();
    let operation = operation::current(&git).unwrap().unwrap();
    assert_eq!(operation.kind, OperationKind::Rebase);
    assert_eq!(operation.branch.as_deref(), Some("feature"));
    // The header's line, without the step shown on its own.
    assert_eq!(operation.headline(), "Rebasing feature onto main");
    assert_eq!(
        operation.progress,
        Some(Progress {
            current: 2,
            total: 2
        })
    );
    // Git's "ours" is main plus the commit already replayed.
    assert_eq!(operation.ours.role, SideRole::RebaseTarget { replayed: 1 });
    assert_eq!(
        operation.ours.label(),
        "main + 1 rebased commit · rebase target"
    );
    assert_eq!(operation.ours.commit.subject, "add other");
    // Git's "theirs" is the user's own commit.
    let theirs = operation.theirs.as_ref().unwrap();
    assert_eq!(theirs.role, SideRole::CommitBeingRebased);
    assert_eq!(theirs.commit.subject, "feature change");
    assert_eq!(
        operation.summary(),
        format!(
            "Rebasing feature onto main · commit 2 of 2: {} feature change",
            theirs.commit.short_id
        )
    );
    // The panes: the commit being rebased is named by its subject alone,
    // the target with its commit id, each with git's own term.
    let session = MergeSession::from_text("<<<<<<< a\nx\n=======\ny\n>>>>>>> b\n", None).unwrap();
    let header = presentation::header(Path::new("file.txt"), &session, Some(&operation));
    assert_eq!(
        header.theirs.detail,
        "git calls this side \u{201c}theirs\u{201d}"
    );
    assert_eq!(
        header.ours.detail,
        format!(
            "{} · git calls this side \u{201c}ours\u{201d}",
            operation.ours.commit.short_id
        )
    );
    assert_eq!(operation::conflicted_paths(&git).unwrap(), ["file.txt"]);
}

/// A real conflicted cherry-pick: theirs is the picked commit.
#[test]
fn cherry_pick() {
    let repo = diverged();
    assert!(!repo.git(&["cherry-pick", "feature"]));

    let operation = operation::current(&repo.scry_git()).unwrap().unwrap();
    assert_eq!(operation.kind, OperationKind::CherryPick);
    assert_eq!(operation.ours.label(), "main · current branch");
    let theirs = operation.theirs.as_ref().unwrap();
    assert_eq!(theirs.role, SideRole::CherryPickedCommit);
    assert_eq!(theirs.commit.subject, "feature change");
}

/// A real conflicted revert: theirs is the commit being reverted.
#[test]
fn revert() {
    let repo = diverged();
    repo.commit("file.txt", "main line 2\n", "second main change");
    assert!(!repo.git(&["revert", "--no-edit", "HEAD~1"]));

    let operation = operation::current(&repo.scry_git()).unwrap().unwrap();
    assert_eq!(operation.kind, OperationKind::Revert);
    let theirs = operation.theirs.as_ref().unwrap();
    assert_eq!(theirs.role, SideRole::RevertedCommit);
    assert_eq!(theirs.commit.subject, "main change");
    assert!(operation.summary().starts_with("Reverting "));
}

/// In a reftable repository (git 3.0's default), HEAD's file names a
/// placeholder branch and the pseudorefs of a cherry-pick, a revert or a
/// rebase are not files: each operation is still found, on its branch.
#[test]
fn operations_in_a_reftable_repository() {
    let repo = diverged_with_format("reftable");
    assert!(!repo.git(&["cherry-pick", "feature"]));
    let operation = operation::current(&repo.scry_git()).unwrap().unwrap();
    assert_eq!(operation.kind, OperationKind::CherryPick);
    assert_eq!(operation.ours.label(), "main · current branch");
    assert_eq!(operation.theirs.unwrap().commit.subject, "feature change");
    assert!(repo.git(&["cherry-pick", "--abort"]));

    repo.commit("file.txt", "main line 2\n", "second main change");
    assert!(!repo.git(&["revert", "--no-edit", "HEAD~1"]));
    let operation = operation::current(&repo.scry_git()).unwrap().unwrap();
    assert_eq!(operation.kind, OperationKind::Revert);
    assert_eq!(operation.theirs.unwrap().commit.subject, "main change");
    assert!(repo.git(&["revert", "--abort"]));

    assert!(repo.git(&["switch", "--quiet", "feature"]));
    assert!(!repo.git(&["merge", "main"]));
    let operation = operation::current(&repo.scry_git()).unwrap().unwrap();
    assert_eq!(operation.kind, OperationKind::Merge);
    assert_eq!(operation.branch.as_deref(), Some("feature"));
    assert_eq!(operation.summary(), "Merging main into feature");
    assert!(repo.git(&["merge", "--abort"]));

    assert!(!repo.git(&["rebase", "main"]));
    let operation = operation::current(&repo.scry_git()).unwrap().unwrap();
    assert_eq!(operation.kind, OperationKind::Rebase);
    assert_eq!(operation.theirs.unwrap().commit.subject, "feature change");
}

/// Outside a repository git fails, and the error comes back instead of a wrong answer.
#[test]
fn not_a_repository() {
    let dir = tempfile::tempdir().unwrap();
    let git = GitCli::new(dir.path().to_path_buf())
        .with_env("GIT_CONFIG_GLOBAL", "/dev/null")
        .with_env("GIT_CONFIG_NOSYSTEM", "1");
    assert!(operation::current(&git).is_err());
}

/// Ours, in a rebase, is the target plus every commit already replayed,
/// counted: the label says how many, so the user knows ours is no longer
/// main as it was.
#[test]
fn the_rebase_target_counts_the_commits_replayed() {
    let repo = Repo::new();
    repo.commit("file.txt", "line\n", "base");
    assert!(repo.git(&["switch", "--quiet", "-c", "feature"]));
    repo.commit("one.txt", "one\n", "add one");
    repo.commit("two.txt", "two\n", "add two");
    repo.commit("file.txt", "feature line\n", "feature change");
    assert!(repo.git(&["switch", "--quiet", "main"]));
    repo.commit("file.txt", "main line\n", "main change");
    assert!(repo.git(&["switch", "--quiet", "feature"]));
    assert!(!repo.git(&["rebase", "main"]));

    let operation = operation::current(&repo.scry_git()).unwrap().unwrap();
    assert_eq!(operation.ours.role, SideRole::RebaseTarget { replayed: 2 });
    assert_eq!(
        operation.ours.label(),
        "main + 2 rebased commits · rebase target"
    );
}
