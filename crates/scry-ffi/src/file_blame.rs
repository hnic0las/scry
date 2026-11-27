//! The blame gutter of the Changes window: who last wrote each line of
//! the file's new side, read while the window stays usable.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use scry_core::file_blame::{self, Target};
use scry_core::git::{Git, GitCli};

/// A commit that wrote lines of the file.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct BlameCommitInfo {
    pub id: String,
    pub short_id: String,
    pub author: String,
    /// The author is the user, by their name in git.
    pub mine: bool,
    /// Author time, in seconds since the Unix epoch.
    pub time: i64,
    pub subject: String,
    /// The file's name in that commit.
    pub path: String,
}

/// Consecutive lines one commit wrote.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Record)]
pub struct BlameLines {
    /// The first line, from 1, of the new side.
    pub first: u32,
    pub count: u32,
    /// The commit's place among the commits; `None` for lines not
    /// committed yet.
    pub commit: Option<u32>,
}

/// What a blame found since the last time it was asked.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct BlameProgress {
    /// The commits named after the ones already given.
    pub commits: Vec<BlameCommitInfo>,
    /// The lines found after the ones already given.
    pub lines: Vec<BlameLines>,
    /// Nothing more will come.
    pub done: bool,
    /// The file is in no commit: every line is not committed.
    pub never_committed: bool,
    /// Why git gave no blame, as a sentence.
    pub failure: Option<String>,
}

#[derive(Default)]
struct Found {
    commits: Vec<BlameCommitInfo>,
    lines: Vec<BlameLines>,
    done: bool,
    never_committed: bool,
    failure: Option<String>,
}

/// One file's blame. `read` blocks until git is done, on a thread of the
/// caller's; `progress` gives what is known meanwhile.
#[derive(uniffi::Object)]
pub struct FileBlame {
    git: GitCli,
    target: Target,
    path: String,
    found: Mutex<Found>,
    stopped: AtomicBool,
}

impl FileBlame {
    pub(crate) fn new(git: GitCli, target: Target, path: String) -> Arc<FileBlame> {
        Arc::new(FileBlame {
            git,
            target,
            path,
            found: Mutex::new(Found::default()),
            stopped: AtomicBool::new(false),
        })
    }

    fn lock(&self) -> MutexGuard<'_, Found> {
        // Only appended to: a panic elsewhere leaves it usable.
        self.found
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

#[uniffi::export]
impl FileBlame {
    /// Reads the blame, until git is done or `stop` is called.
    pub fn read(&self) {
        let me = self
            .git
            .run(&["config", "user.name"])
            .map(|name| name.trim().to_string())
            .unwrap_or_default();
        let read = file_blame::blame(
            &self.git,
            &self.target,
            &self.path,
            &mut |lines, commits| {
                let mut found = self.lock();
                for commit in commits.iter().skip(found.commits.len()) {
                    found.commits.push(BlameCommitInfo {
                        id: commit.id.clone(),
                        short_id: commit.short_id().to_string(),
                        author: commit.author.clone(),
                        mine: !me.is_empty() && commit.author == me,
                        time: commit.time,
                        subject: commit.subject.clone(),
                        path: commit.path.clone(),
                    });
                }
                found.lines.push(BlameLines {
                    first: lines.first as u32,
                    count: lines.count as u32,
                    commit: lines.commit.map(|place| place as u32),
                });
                !self.stopped.load(Ordering::Relaxed)
            },
        );
        let mut found = self.lock();
        if let Err(error) = read {
            // Git blames from HEAD: a file that is not in it yet has no
            // history, and all of it is new.
            if self.target.can_be_uncommitted() && found.lines.is_empty() {
                found.never_committed = true;
            } else {
                found.failure = Some(error.to_string());
            }
        }
        found.done = true;
    }

    /// Stops `read` at the next lines git gives.
    pub fn stop(&self) {
        self.stopped.store(true, Ordering::Relaxed);
    }

    /// What was found after the first `commits` commits and `lines`
    /// groups of lines, which the caller already has.
    pub fn progress(&self, commits: u32, lines: u32) -> BlameProgress {
        let found = self.lock();
        BlameProgress {
            commits: found
                .commits
                .iter()
                .skip(commits as usize)
                .cloned()
                .collect(),
            lines: found.lines.iter().skip(lines as usize).copied().collect(),
            done: found.done,
            never_committed: found.never_committed,
            failure: found.failure.clone(),
        }
    }
}

/// The age as the gutter shows it: "8 d", "3 wk", "2 mo".
#[uniffi::export]
pub fn blame_age(time: i64, now: i64) -> String {
    file_blame::short_age(time, now)
}

#[cfg(test)]
mod tests {
    use crate::ChangeSet;
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

    /// The window asks for the blame of a listed file, reads it, and
    /// takes what was found in two parts.
    #[test]
    fn a_listed_file_is_blamed() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        git(root, &["init", "--quiet", "--initial-branch=main"]);
        git(root, &["config", "user.name", "Scry Test"]);
        fs::write(root.join("a.txt"), "one\ntwo\nthree\n").unwrap();
        git(root, &["add", "."]);
        git(root, &["commit", "--quiet", "-m", "Start"]);
        fs::write(root.join("a.txt"), "one\ntwo\nthree\nfour\n").unwrap();
        fs::write(root.join("new.txt"), "fresh\n").unwrap();

        let changes = ChangeSet::open(root.display().to_string(), false, None, Vec::new()).unwrap();
        let files = changes.files().unwrap();
        let place = |name: &str| files.iter().position(|file| file.path == name).unwrap() as u32;

        let blame = changes.blame(place("a.txt")).unwrap();
        assert!(!blame.progress(0, 0).done);
        blame.read();
        let all = blame.progress(0, 0);
        assert!(all.done && !all.never_committed && all.failure.is_none());
        assert_eq!(all.commits.len(), 1);
        assert!(all.commits[0].mine);
        assert_eq!(all.commits[0].subject, "Start");
        let lines: u32 = all.lines.iter().map(|lines| lines.count).sum();
        assert_eq!(lines, 4);
        assert!(all.lines.iter().any(|l| l.first == 4 && l.commit.is_none()));
        let rest = blame.progress(1, all.lines.len() as u32);
        assert!(rest.commits.is_empty() && rest.lines.is_empty() && rest.done);

        let fresh = changes.blame(place("new.txt")).unwrap();
        fresh.read();
        let found = fresh.progress(0, 0);
        assert!(found.done && found.never_committed && found.failure.is_none());
    }
}
