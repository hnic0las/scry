//! `git scry wait`, run as the binary on a real repository: what Scry's
//! Claude Code plugin runs as a monitor.

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::mpsc;
use std::time::{Duration, SystemTime};

use scry_core::asks;
use scry_core::git::GitCli;
use scry_core::records::{Answer, Ask, AskKind, Store};

const PATIENCE: Duration = Duration::from_secs(10);

fn git(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .current_dir(dir)
        .args(args)
        .env("GIT_CONFIG_GLOBAL", dir.join("../gitconfig"))
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .status()
        .unwrap();
    assert!(status.success(), "git {args:?}");
}

/// A repository with one commit, in `dir/repo`.
fn repository(dir: &Path) -> PathBuf {
    std::fs::write(dir.join("gitconfig"), "[user]\n\tname = T\n\temail = t@e\n").unwrap();
    let repo = dir.join("repo");
    std::fs::create_dir(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("a.txt"), "a\n").unwrap();
    git(&repo, &["add", "a.txt"]);
    git(&repo, &["commit", "-q", "-m", "a"]);
    repo
}

/// A `git-scry wait` that is killed when the test ends, passed or not: a
/// failing test, or a mutant that never exits, leaves no process behind.
struct Running(Child);

impl std::ops::Deref for Running {
    type Target = Child;
    fn deref(&self) -> &Child {
        &self.0
    }
}

impl std::ops::DerefMut for Running {
    fn deref_mut(&mut self) -> &mut Child {
        &mut self.0
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn wait(dir: &Path, args: &[&str]) -> Running {
    Running(
        Command::new(env!("CARGO_BIN_EXE_git-scry"))
            .current_dir(dir)
            .arg("wait")
            .args(args)
            .env("GIT_CONFIG_GLOBAL", dir.join("../gitconfig"))
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    )
}

/// The lines `child` prints, as they come.
fn printed(child: &mut Child) -> mpsc::Receiver<String> {
    let stdout = child.stdout.take().unwrap();
    let (send, receive) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            if send.send(line.unwrap()).is_err() {
                return;
            }
        }
    });
    receive
}

/// How `child` exited, failing rather than hanging when it does not.
fn finish(child: &mut Child) -> ExitStatus {
    let deadline = SystemTime::now() + PATIENCE;
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            return status;
        }
        if SystemTime::now() > deadline {
            child.kill().unwrap();
            panic!("git-scry wait did not exit");
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// When the watcher `child` last said it was there.
fn last_beat(repo: &Path, child: &Child) -> Option<SystemTime> {
    std::fs::metadata(
        git_dir(repo)
            .join("scry/watchers")
            .join(child.id().to_string()),
    )
    .ok()?
    .modified()
    .ok()
}

fn store(repo: &Path) -> Store {
    Store::open(&GitCli::new(repo.to_path_buf())).unwrap()
}

fn ask(conflict: &str, at: u64) -> Ask {
    Ask {
        conflict: conflict.to_string(),
        kind: AskKind::Explain,
        at,
    }
}

fn git_dir(repo: &Path) -> PathBuf {
    repo.join(".git")
}

/// `wait` says it is there while it waits, prints the ask the app makes,
/// and exits; an ask still open when it starts is printed at once, and
/// once answered it is not printed again.
#[test]
fn wait_prints_the_next_ask_and_exits() {
    let dir = tempfile::tempdir().unwrap();
    let repo = repository(dir.path());
    let mut child = wait(&repo, &[]);
    let lines = printed(&mut child);
    let deadline = SystemTime::now() + PATIENCE;
    while !asks::watching(&git_dir(&repo), SystemTime::now()) {
        assert!(SystemTime::now() < deadline, "no presence file");
        std::thread::sleep(Duration::from_millis(50));
    }
    store(&repo)
        .ask("src/My File.swift", ask("c12", 1_000))
        .unwrap();
    assert_eq!(
        lines.recv_timeout(PATIENCE).unwrap(),
        r#"Scry asks: explain "src/My File.swift" c12"#
    );
    assert!(finish(&mut child).success());
    assert!(!asks::watching(&git_dir(&repo), SystemTime::now()));

    let mut again = wait(&repo, &[]);
    let lines = printed(&mut again);
    assert_eq!(
        lines.recv_timeout(PATIENCE).unwrap(),
        r#"Scry asks: explain "src/My File.swift" c12"#
    );
    assert!(finish(&mut again).success());

    store(&repo)
        .answer(
            "src/My File.swift",
            Answer {
                conflict: String::from("c12"),
                kind: AskKind::Explain,
                by: String::from("claude-code 2.1"),
                text: String::from("Main renamed it; the feature changed the timeout."),
                at: 2_000,
            },
        )
        .unwrap();
    let mut answered = wait(&repo, &[]);
    let lines = printed(&mut answered);
    assert!(lines.recv_timeout(Duration::from_millis(1500)).is_err());
    answered.kill().unwrap();
    answered.wait().unwrap();
}

/// With `--follow`, each new ask is printed once, two asks made together
/// both, an ask made again printed again; the watch goes on.
#[test]
fn follow_prints_every_new_ask() {
    let dir = tempfile::tempdir().unwrap();
    let repo = repository(dir.path());
    let mut child = wait(&repo, &["--follow"]);
    let lines = printed(&mut child);
    // It says it is there, and not at every look at the asks: once per
    // beat, ten seconds apart.
    let deadline = SystemTime::now() + PATIENCE;
    let first = loop {
        if let Some(beat) = last_beat(&repo, &child) {
            break beat;
        }
        assert!(SystemTime::now() < deadline, "no presence file");
        std::thread::sleep(Duration::from_millis(50));
    };
    std::thread::sleep(Duration::from_millis(1200));
    assert_eq!(last_beat(&repo, &child), Some(first));
    let store = store(&repo);
    store.ask("a.txt", ask("c1", 1_000)).unwrap();
    assert_eq!(
        lines.recv_timeout(PATIENCE).unwrap(),
        "Scry asks: explain a.txt c1"
    );
    store.ask("a.txt", ask("c2", 2_000)).unwrap();
    store.ask("b.txt", ask("c3", 2_001)).unwrap();
    assert_eq!(
        lines.recv_timeout(PATIENCE).unwrap(),
        "Scry asks: explain a.txt c2"
    );
    assert_eq!(
        lines.recv_timeout(PATIENCE).unwrap(),
        "Scry asks: explain b.txt c3"
    );
    store.ask("a.txt", ask("c1", 3_000)).unwrap();
    assert_eq!(
        lines.recv_timeout(PATIENCE).unwrap(),
        "Scry asks: explain a.txt c1"
    );
    assert!(lines.recv_timeout(Duration::from_millis(1500)).is_err());
    assert!(child.try_wait().unwrap().is_none());
    child.kill().unwrap();
    child.wait().unwrap();
}

/// Outside a repository, `wait` fails, while `wait --follow` (the plugin's
/// monitor, started in every folder) stays quiet and running.
#[test]
fn outside_a_repository_follow_stays_quiet() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("gitconfig"), "").unwrap();
    let folder = dir.path().join("folder");
    std::fs::create_dir(&folder).unwrap();
    assert_eq!(finish(&mut wait(&folder, &[])).code(), Some(2));

    let mut child = wait(&folder, &["--follow"]);
    let lines = printed(&mut child);
    assert!(lines.recv_timeout(Duration::from_millis(1500)).is_err());
    assert!(child.try_wait().unwrap().is_none());
    child.kill().unwrap();
    child.wait().unwrap();
}
