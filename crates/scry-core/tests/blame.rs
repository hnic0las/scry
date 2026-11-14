//! The commits shown in the lens rows, found with the `git` executable on
//! real repositories in temporary directories.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

use scry_core::blame::{self, LineCommit};
use scry_core::git::{Git, GitCli, GitError, GitOutput};
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
        self.conflict_commits_with(&GitCli::new(self.path()))
    }

    fn conflict_commits_with(&self, git: &dyn Git) -> Vec<blame::ConflictCommits> {
        let operation = operation::current(git).unwrap().unwrap();
        let text = fs::read_to_string(self.path().join("file.txt")).unwrap();
        let session = MergeSession::from_text(&text, None).unwrap();
        blame::conflict_commits(git, &operation, "file.txt", &session)
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

/// The common text of the conflicted file holds both sides' clean
/// changes: lines theirs added above the conflict are not in ours' file.
/// Ours must still show the commit of its own conflicting line, not of
/// the line that many lines below it.
#[test]
fn lines_added_above_by_the_other_side_do_not_shift_the_blame() {
    let repo = Repo::new();
    let base = "h1\nh2\nh3\nh4\nline\nk1\nk2\nk3\nk4\n";
    repo.commit(base, "base", "Scry Test");
    assert!(repo.git(&["switch", "--quiet", "-c", "feature"], "Scry Test"));
    repo.commit(
        "new1\nnew2\nnew3\nh1\nh2\nh3\nh4\nfeature line\nk1\nk2\nk3\nk4\n",
        "Retry failed uploads",
        "You",
    );
    assert!(repo.git(&["switch", "--quiet", "main"], "Scry Test"));
    repo.commit(
        "h1\nh2\nh3\nh4\nmain line\nk1\nk2\nk3\nk4\n",
        "Add a timeout",
        "Alex Martin",
    );
    // Three lines below the conflict in main's file: where the conflict
    // would be if the lines feature added were counted.
    repo.commit(
        "h1\nh2\nh3\nh4\nmain line\nk1\nk2\nk3 renamed\nk4\n",
        "Rename k3",
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

/// git before 2.41, as Xcode 15 and 16 ship it: `blame --contents` with a revision
/// fails with "cannot use --contents with final commit object name".
struct OldGit(GitCli);

impl Git for OldGit {
    fn output(&self, args: &[&str]) -> Result<GitOutput, GitError> {
        self.0.output(args)
    }

    fn output_with_input(&self, args: &[&str], input: &[u8]) -> Result<GitOutput, GitError> {
        if args.contains(&"--contents") {
            return Ok(GitOutput {
                status: Some(128),
                stdout: Vec::new(),
                stderr: String::from("fatal: cannot use --contents with final commit object name"),
            });
        }
        self.0.output_with_input(args, input)
    }

    fn output_with_env(&self, args: &[&str], env: &[(&str, &str)]) -> Result<GitOutput, GitError> {
        self.0.output_with_env(args, env)
    }
}

/// With a git that refuses `--contents` with a revision (checked with Debian's 2.39.5),
/// each side's revision is blamed as it is and the conflict's lines found by their text:
/// the lens still shows each side's commit, even with lines added above by the other side.
#[test]
fn an_older_git_still_finds_each_sides_commit() {
    let repo = Repo::new();
    repo.commit(
        "h1\nh2\nh3\nh4\nline\nk1\nk2\nk3\nk4\n",
        "base",
        "Scry Test",
    );
    assert!(repo.git(&["switch", "--quiet", "-c", "feature"], "Scry Test"));
    repo.commit(
        "new1\nnew2\nnew3\nh1\nh2\nh3\nh4\nfeature line\nk1\nk2\nk3\nk4\n",
        "Retry failed uploads",
        "You",
    );
    assert!(repo.git(&["switch", "--quiet", "main"], "Scry Test"));
    repo.commit(
        "h1\nh2\nh3\nh4\nmain line\nk1\nk2\nk3\nk4\n",
        "Add a timeout",
        "Alex Martin",
    );
    repo.commit(
        "h1\nh2\nh3\nh4\nmain line\nk1\nk2\nk3 renamed\nk4\n",
        "Rename k3",
        "Priya Shah",
    );
    assert!(repo.git(&["switch", "--quiet", "feature"], "Scry Test"));
    assert!(!repo.git(&["rebase", "main"], "Scry Test"));

    let commits = repo.conflict_commits_with(&OldGit(GitCli::new(repo.path())));
    assert_eq!(
        subject_and_author(&commits[0].ours),
        Some(("Add a timeout", "Alex Martin"))
    );
    assert_eq!(
        subject_and_author(&commits[0].theirs),
        Some(("Retry failed uploads", "You"))
    );
}
