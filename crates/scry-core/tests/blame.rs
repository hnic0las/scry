//! The commits shown in the lens rows, found with the `git` executable on
//! real repositories in temporary directories.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

use scry_core::blame::{self, LineCommit};
use scry_core::git::GitCli;
use scry_core::operation;
use scry_core::session::MergeSession;
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
        assert!(repo.git(&["init", "--quiet", "--initial-branch=main"], "Scry Test"));
        repo
    }

    fn path(&self) -> PathBuf {
        self.root.path().join("repo")
    }

    /// Runs git as `author` and tells whether it succeeded.
    fn git(&self, args: &[&str], author: &str) -> bool {
        let output = Command::new("git")
            .arg("-C")
            .arg(self.path())
            .args(args)
            .env("GIT_CONFIG_GLOBAL", self.root.path().join("gitconfig"))
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_AUTHOR_NAME", author)
            .env("GIT_AUTHOR_EMAIL", "test@example.com")
            .env("GIT_COMMITTER_NAME", author)
            .env("GIT_COMMITTER_EMAIL", "test@example.com")
            .env("GIT_EDITOR", "true")
            .output()
            .unwrap();
        output.status.success()
    }

    fn commit(&self, content: &str, message: &str, author: &str) {
        fs::write(self.path().join("file.txt"), content).unwrap();
        assert!(self.git(&["add", "file.txt"], author));
        assert!(self.git(&["commit", "--quiet", "-m", message], author));
    }

    /// The commits of every conflict of `file.txt`, as the app computes them.
    fn conflict_commits(&self) -> Vec<blame::ConflictCommits> {
        let git = GitCli::new(self.path());
        let operation = operation::current(&git).unwrap().unwrap();
        let text = fs::read_to_string(self.path().join("file.txt")).unwrap();
        let session = MergeSession::from_text(&text, None).unwrap();
        blame::conflict_commits(&git, &operation, "file.txt", &session)
    }
}

fn subject_and_author(commit: &Option<LineCommit>) -> Option<(&str, &str)> {
    commit
        .as_ref()
        .map(|commit| (commit.subject.as_str(), commit.author.as_str()))
}

/// During a rebase, ours shows the target branch's commit that changed the
/// line and theirs the user's own commit, each with its author, even when
/// later commits touched other lines of the file.
#[test]
fn each_side_shows_the_commit_that_changed_its_lines() {
    let repo = Repo::new();
    repo.commit("top\na\nb\nc\nline\nbottom\n", "base", "Scry Test");
    assert!(repo.git(&["switch", "--quiet", "-c", "feature"], "Scry Test"));
    repo.commit(
        "top\na\nb\nc\nfeature line\nbottom\n",
        "Retry failed uploads",
        "You",
    );
    assert!(repo.git(&["switch", "--quiet", "main"], "Scry Test"));
    repo.commit(
        "top\na\nb\nc\nmain line\nbottom\n",
        "Add a timeout",
        "Alex Martin",
    );
    // Far enough from the conflicting line for git to merge it cleanly.
    repo.commit(
        "top changed\na\nb\nc\nmain line\nbottom\n",
        "Rename top",
        "Priya Shah",
    );
    assert!(repo.git(&["switch", "--quiet", "feature"], "Scry Test"));
    assert!(!repo.git(&["rebase", "main"], "Scry Test"));

    let commits = repo.conflict_commits();
    assert_eq!(commits.len(), 1);
    assert_eq!(
        subject_and_author(&commits[0].ours),
        Some(("Add a timeout", "Alex Martin"))
    );
    assert_eq!(
        subject_and_author(&commits[0].theirs),
        Some(("Retry failed uploads", "You"))
    );
    let short_id = &commits[0].theirs.as_ref().unwrap().short_id;
    assert!(short_id.len() >= 7 && short_id.chars().all(|c| c.is_ascii_hexdigit()));
}

/// A side that deleted the lines has none to blame; it shows the last commit
/// that touched the file on that side, here the deletion itself.
#[test]
fn deleting_side_shows_its_last_commit_of_the_file() {
    let repo = Repo::new();
    repo.commit("top\nline\nbottom\n", "base", "Scry Test");
    assert!(repo.git(&["switch", "--quiet", "-c", "feature"], "Scry Test"));
    repo.commit("top\nbottom\n", "Remove the line", "You");
    assert!(repo.git(&["switch", "--quiet", "main"], "Scry Test"));
    repo.commit("top\nmain line\nbottom\n", "Change the line", "Alex Martin");
    assert!(!repo.git(&["merge", "feature"], "Scry Test"));

    let commits = repo.conflict_commits();
    assert_eq!(
        subject_and_author(&commits[0].ours),
        Some(("Change the line", "Alex Martin"))
    );
    assert_eq!(
        subject_and_author(&commits[0].theirs),
        Some(("Remove the line", "You"))
    );
}
