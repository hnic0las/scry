//! Whole-file blame read with the `git` executable, on real repositories
//! in temporary directories.

use std::cell::Cell;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

use scry_core::file_blame::{self, BlameCommit, Lines, Target};
use scry_core::git::GitCli;
use tempfile::TempDir;

/// A throwaway repository whose git commands ignore the user's own
/// configuration (signing, hooks, default branch).
struct Repo {
    root: TempDir,
    author: Cell<&'static str>,
}

impl Repo {
    fn new() -> Repo {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("gitconfig"), "").unwrap();
        fs::create_dir(root.path().join("repo")).unwrap();
        let repo = Repo {
            root,
            author: Cell::new("Alex Martin"),
        };
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
            .env("GIT_AUTHOR_NAME", self.author.get())
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
}

/// Each line's author, "-" for a line not committed; then the commits.
fn authors(repo: &Repo, target: &Target, path: &str) -> (Vec<String>, Vec<BlameCommit>) {
    let mut found: Vec<Lines> = Vec::new();
    let mut commits = Vec::new();
    file_blame::blame(&repo.scry_git(), target, path, &mut |lines, named| {
        found.push(lines);
        commits = named.to_vec();
        true
    })
    .unwrap();
    let count = found
        .iter()
        .map(|l| l.first + l.count - 1)
        .max()
        .unwrap_or(0);
    let mut authors = vec![String::from("?"); count];
    for lines in found {
        for line in lines.first..lines.first + lines.count {
            authors[line - 1] = match lines.commit {
                Some(place) => commits[place].author.clone(),
                None => String::from("-"),
            };
        }
    }
    (authors, commits)
}

fn history() -> Repo {
    let repo = Repo::new();
    repo.commit("old.txt", "one\ntwo\nthree\nfour\nfive\nsix\n", "first");
    repo.author.set("Priya Shah");
    repo.git(&["mv", "old.txt", "new.txt"]);
    repo.commit(
        "new.txt",
        "one\nTWO\nthree\nfour\nfive\nsix\n",
        "second: two",
    );
    repo
}

#[test]
fn a_commit_is_blamed_across_a_rename() {
    let repo = history();
    let (authors, commits) = authors(&repo, &Target::Commit(String::from("HEAD")), "new.txt");
    assert_eq!(
        authors,
        [
            "Alex Martin",
            "Priya Shah",
            "Alex Martin",
            "Alex Martin",
            "Alex Martin",
            "Alex Martin"
        ]
    );
    let first = commits.iter().find(|c| c.subject == "first").unwrap();
    assert_eq!(first.path, "old.txt");
    assert!(first.time > 0);
    assert_eq!(first.id.len(), 40);
}

#[test]
fn the_working_copy_and_the_index_have_lines_not_committed() {
    let repo = history();
    repo.write("new.txt", "one\nTWO\nthree\nstaged\nfive\nsix\n");
    repo.git(&["add", "--", "new.txt"]);
    repo.write("new.txt", "one\nTWO\nthree\nstaged\nfive\nsix\non disk\n");
    let (index, _) = authors(&repo, &Target::Index, "new.txt");
    assert_eq!(index.len(), 6);
    assert_eq!(index[3], "-");
    assert_eq!(index[1], "Priya Shah");
    let (disk, _) = authors(&repo, &Target::WorkingCopy, "new.txt");
    assert_eq!(disk.len(), 7);
    assert_eq!(disk[3], "-");
    assert_eq!(disk[6], "-");
    assert_eq!(disk[0], "Alex Martin");
}

#[test]
fn a_file_never_committed_has_no_blame() {
    let repo = history();
    repo.write("fresh.txt", "a\n");
    let read = file_blame::blame(
        &repo.scry_git(),
        &Target::WorkingCopy,
        "fresh.txt",
        &mut |_, _| true,
    );
    assert!(read.is_err());
}

#[test]
fn reading_stops_when_asked() {
    let repo = history();
    let mut seen = 0;
    file_blame::blame(
        &repo.scry_git(),
        &Target::Commit(String::from("HEAD")),
        "new.txt",
        &mut |_, _| {
            seen += 1;
            false
        },
    )
    .unwrap();
    assert_eq!(seen, 1);
}
