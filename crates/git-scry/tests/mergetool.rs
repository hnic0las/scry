//! `git mergetool` running `git-scry resolve` on a real rebase conflict.

use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};

use tempfile::TempDir;

struct Repo {
    root: TempDir,
}

impl Repo {
    /// A rebase of `feature` onto `main` stopped on a conflict in
    /// `file.txt`, with git configured to use git-scry as its merge tool.
    /// Git's configuration is isolated from the user's. `scry_args` are
    /// appended to the merge tool command.
    fn stopped_rebase(conflict_style: &str, scry_args: &str) -> Repo {
        let root = tempfile::tempdir().unwrap();
        // Forward slashes: the command runs in git's shell, also on Windows.
        let binary = env!("CARGO_BIN_EXE_git-scry").replace('\\', "/");
        let config = format!(
            "[merge]\n\ttool = scry\n\tconflictStyle = {conflict_style}\n\
             [mergetool]\n\tprompt = false\n\tkeepBackup = false\n\
             [mergetool \"scry\"]\n\
             \tcmd = \\\"{binary}\\\" resolve \\\"$BASE\\\" \\\"$LOCAL\\\" \\\"$REMOTE\\\" \\\"$MERGED\\\" {scry_args}\n\
             \ttrustExitCode = true\n"
        );
        fs::write(root.path().join("gitconfig"), config).unwrap();
        fs::create_dir(root.path().join("repo")).unwrap();
        let repo = Repo { root };

        repo.ok(&["init", "--quiet", "--initial-branch=main"]);
        repo.commit("line\nend\n", "base");
        repo.ok(&["switch", "--quiet", "-c", "feature"]);
        repo.commit("feature line\nend\n", "feature change");
        repo.ok(&["switch", "--quiet", "main"]);
        repo.commit("main line\nend\n", "main change");
        repo.ok(&["switch", "--quiet", "feature"]);
        assert!(!repo.git(&["rebase", "main"]).status.success());
        repo
    }

    fn path(&self) -> PathBuf {
        self.root.path().join("repo")
    }

    fn git(&self, args: &[&str]) -> Output {
        Command::new("git")
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
            .unwrap()
    }

    fn ok(&self, args: &[&str]) {
        let output = self.git(args);
        assert!(output.status.success(), "git {args:?}: {output:?}");
    }

    fn commit(&self, content: &str, message: &str) {
        fs::write(self.path().join("file.txt"), content).unwrap();
        self.ok(&["add", "file.txt"]);
        self.ok(&["commit", "--quiet", "-m", message]);
    }

    fn file(&self) -> String {
        fs::read_to_string(self.path().join("file.txt")).unwrap()
    }

    fn unmerged(&self) -> String {
        let output = self.git(&["diff", "--name-only", "--diff-filter=U"]);
        String::from_utf8(output.stdout).unwrap()
    }
}

/// Through the real `git mergetool`: a resolved file is written and git stages it.
#[test]
fn resolved_file_is_written_and_staged() {
    let repo = Repo::stopped_rebase("merge", "--take 1=theirs-then-ours");
    let output = repo.git(&["mergetool"]);
    assert!(output.status.success(), "{output:?}");
    // Rebuilt with the base, then "your commit" first.
    assert_eq!(repo.file(), "feature line\nmain line\nend\n");
    assert_eq!(repo.unmerged(), "");
}

/// With a conflict left and no terminal to ask in, the file stays untouched and still
/// conflicted.
#[test]
fn unresolved_file_is_left_as_it_was() {
    let repo = Repo::stopped_rebase("zdiff3", "");
    let before = repo.file();
    // No choice and no terminal: nothing to write.
    let output = repo.git(&["mergetool"]);
    assert!(!output.status.success());
    assert_eq!(repo.file(), before);
    assert!(before.contains("<<<<<<<"));
    assert_eq!(repo.unmerged(), "file.txt\n");
}

/// An unknown candidate in `--take` fails with a message and writes nothing.
#[test]
fn invalid_choice_fails_without_writing() {
    let repo = Repo::stopped_rebase("zdiff3", "--take 1=nonsense");
    let before = repo.file();
    let output = repo.git(&["mergetool"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("unknown candidate"));
    assert_eq!(repo.file(), before);
}

/// Through the real `git mergetool`, with git's default `merge` style: a
/// line the user fixed in the conflicted file before launching the tool is
/// still there once the conflict is resolved, instead of being lost to a
/// rebuild from git's three versions.
#[test]
fn edits_made_before_launching_are_kept() {
    let repo = Repo::stopped_rebase("merge", "--take 1=theirs");
    let edited = repo.file().replace("end\n", "end, edited\n");
    fs::write(repo.path().join("file.txt"), &edited).unwrap();

    let output = repo.git(&["mergetool"]);
    assert!(output.status.success(), "{output:?}");
    assert_eq!(repo.file(), "feature line\nend, edited\n");
    assert_eq!(repo.unmerged(), "");
}
