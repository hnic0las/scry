//! The operation overview on real repositories: a rebase stopped on two
//! conflicted files, resolved, staged and continued through its commands.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

use scry_core::git::GitCli;
use scry_core::overview::{Overview, OverviewCommand, OverviewError, OverviewEvent};
use scry_core::rebase_plan::{Action, StepState};
use scry_core::theme::State;
use tempfile::TempDir;

/// A throwaway repository whose git ignores the user's configuration
/// (signing, hooks, default branch).
struct Repo {
    root: TempDir,
}

impl Repo {
    /// `feature` and `main` both change `a.txt` and `Sources/b.swift`;
    /// rebasing `feature` onto `main` stops on both.
    fn stopped_rebase() -> Repo {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("gitconfig"), "").unwrap();
        fs::create_dir_all(root.path().join("repo/Sources")).unwrap();
        let repo = Repo { root };
        repo.ok(&["init", "--quiet", "--initial-branch=main"]);
        repo.commit("a\n", "let b = 0\n", "base");
        repo.ok(&["switch", "--quiet", "-c", "feature"]);
        repo.commit("a feature\n", "let b = 2\n", "feature change");
        repo.ok(&["switch", "--quiet", "main"]);
        repo.commit("a main\n", "let b = 1\n", "main change");
        repo.ok(&["switch", "--quiet", "feature"]);
        assert!(!repo.git(&["rebase", "main"]));
        repo
    }

    fn path(&self) -> PathBuf {
        self.root.path().join("repo")
    }

    /// Git as Scry runs it, with the same isolated configuration.
    fn cli(&self) -> GitCli {
        Self::environment(self.root.path())
            .into_iter()
            .fold(GitCli::new(self.path()), |git, (key, value)| {
                git.with_env(key, &value)
            })
    }

    fn git(&self, args: &[&str]) -> bool {
        Command::new("git")
            .arg("-C")
            .arg(self.path())
            .args(args)
            .envs(Self::environment(self.root.path()))
            .output()
            .unwrap()
            .status
            .success()
    }

    fn environment(root: &std::path::Path) -> Vec<(&'static str, String)> {
        vec![
            (
                "GIT_CONFIG_GLOBAL",
                root.join("gitconfig").display().to_string(),
            ),
            ("GIT_CONFIG_NOSYSTEM", String::from("1")),
            ("GIT_AUTHOR_NAME", String::from("Scry Test")),
            ("GIT_AUTHOR_EMAIL", String::from("test@example.com")),
            ("GIT_COMMITTER_NAME", String::from("Scry Test")),
            ("GIT_COMMITTER_EMAIL", String::from("test@example.com")),
        ]
    }

    fn ok(&self, args: &[&str]) {
        assert!(self.git(args), "git {args:?}");
    }

    fn commit(&self, a: &str, b: &str, message: &str) {
        fs::write(self.path().join("a.txt"), a).unwrap();
        fs::write(self.path().join("Sources/b.swift"), b).unwrap();
        self.ok(&["add", "."]);
        self.ok(&["commit", "--quiet", "-m", message]);
    }
}

/// The state of the file at `path`, as git orders them.
fn state(overview: &mut Overview, git: &GitCli, path: &str) -> State {
    let snapshot = overview.refresh(git).unwrap();
    snapshot
        .files
        .into_iter()
        .find(|file| file.path == path)
        .unwrap()
        .state
}

/// The whole flow of S1: both files start unresolved; one resolved by hand
/// shows as resolved, then staged; continue is refused until every file is
/// staged, then finishes the rebase.
#[test]
fn files_go_from_unresolved_to_staged_then_the_rebase_continues() {
    let repo = Repo::stopped_rebase();
    let git = repo.cli();
    let mut overview = Overview::new(repo.path());

    let snapshot = overview.refresh(&git).unwrap();
    assert!(snapshot.operation.is_some());
    let paths: Vec<&str> = snapshot
        .files
        .iter()
        .map(|file| file.path.as_str())
        .collect();
    assert_eq!(paths, ["Sources/b.swift", "a.txt"]);
    let b = &snapshot.files[0];
    assert_eq!(
        (b.name.as_str(), b.directory.as_str(), b.kind.as_str()),
        ("b.swift", "Sources", "Swift")
    );
    assert_eq!(b.conflicts, Some(1));
    assert!(
        snapshot
            .files
            .iter()
            .all(|file| file.state == State::Unresolved)
    );

    fs::write(repo.path().join("a.txt"), "a both\n").unwrap();
    assert_eq!(state(&mut overview, &git, "a.txt"), State::Resolved);
    assert_eq!(
        overview
            .apply(&git, OverviewCommand::StageResolved)
            .unwrap(),
        [OverviewEvent::Staged {
            paths: vec![String::from("a.txt")]
        }]
    );
    assert_eq!(state(&mut overview, &git, "a.txt"), State::Staged);

    assert!(matches!(
        overview.apply(&git, OverviewCommand::Continue),
        Err(OverviewError::NotStaged(1))
    ));

    fs::write(repo.path().join("Sources/b.swift"), "let b = 3\n").unwrap();
    overview
        .apply(&git, OverviewCommand::StageResolved)
        .unwrap();
    assert!(overview.refresh(&git).unwrap().can_continue());
    assert_eq!(
        overview.apply(&git, OverviewCommand::Continue).unwrap(),
        [OverviewEvent::Continued]
    );

    let done = overview.refresh(&git).unwrap();
    assert!(done.operation.is_none());
    assert!(done.files.is_empty());
    assert_eq!(
        fs::read_to_string(repo.path().join("a.txt")).unwrap(),
        "a both\n"
    );
}

/// A merge where main deleted a file the branch changed: it is listed as
/// deleted on one side, which the editor cannot open; a merge has no
/// commit to skip; abort gives the repository back as it was.
#[test]
fn a_merge_can_be_aborted_but_not_skipped() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("gitconfig"), "").unwrap();
    fs::create_dir_all(root.path().join("repo/Sources")).unwrap();
    let repo = Repo { root };
    repo.ok(&["init", "--quiet", "--initial-branch=main"]);
    repo.commit("a\n", "let b = 0\n", "base");
    repo.ok(&["switch", "--quiet", "-c", "feature"]);
    repo.commit("a feature\n", "let b = 2\n", "feature change");
    repo.ok(&["switch", "--quiet", "main"]);
    repo.ok(&["rm", "--quiet", "a.txt"]);
    repo.ok(&["commit", "--quiet", "-m", "delete a"]);
    assert!(!repo.git(&["merge", "feature"]));
    let git = repo.cli();
    let mut overview = Overview::new(repo.path());

    let snapshot = overview.refresh(&git).unwrap();
    let a = snapshot
        .files
        .iter()
        .find(|file| file.path == "a.txt")
        .unwrap();
    assert_eq!(a.kind, "Deleted on one side");
    assert!(!a.openable);
    assert!(!snapshot.can_skip());
    assert!(matches!(
        overview.apply(&git, OverviewCommand::Skip),
        Err(OverviewError::CannotSkipMerge)
    ));

    assert_eq!(
        overview.apply(&git, OverviewCommand::Abort).unwrap(),
        [OverviewEvent::Aborted]
    );
    assert!(overview.refresh(&git).unwrap().operation.is_none());
    assert!(!repo.path().join("a.txt").exists());
}

/// An interactive rebase shows its plan: each line's action, the commit a
/// fixup joins, and where git stands. After Continue, git applies the
/// lines without conflicts on its own; the overview says which, so the
/// jump from step 1 to step 5 is explained.
#[test]
fn an_interactive_rebase_shows_its_plan_and_what_git_applied() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("gitconfig"), "").unwrap();
    fs::create_dir_all(root.path().join("repo/Sources")).unwrap();
    let repo = Repo { root };
    repo.ok(&["init", "--quiet", "--initial-branch=main"]);
    repo.commit("a\n", "let b = 0\n", "base");
    repo.ok(&["switch", "--quiet", "-c", "feature"]);
    // Commits a change and gives its id.
    let write = |path: &str, text: &str, message: &str| {
        fs::write(repo.path().join(path), text).unwrap();
        repo.ok(&["add", "."]);
        repo.ok(&["commit", "--quiet", "-m", message]);
        let output = Command::new("git")
            .arg("-C")
            .arg(repo.path())
            .args(["rev-parse", "HEAD"])
            .output()
            .unwrap();
        String::from_utf8(output.stdout).unwrap().trim().to_string()
    };
    let a = write("a.txt", "a feature\n", "change a");
    let notes = write("notes.txt", "notes\n", "add notes");
    let fixup = write("notes.txt", "notes, fixed\n", "fixup! add notes");
    let debug = write("debug.txt", "debug\n", "debug");
    let b = write("Sources/b.swift", "let b = 2\n", "change b");
    repo.ok(&["switch", "--quiet", "main"]);
    repo.commit("a main\n", "let b = 1\n", "main change");
    repo.ok(&["switch", "--quiet", "feature"]);

    let todo = repo.root.path().join("todo");
    fs::write(
        &todo,
        format!(
            "pick {} change a\nreword {} add notes\nfixup {} fixup! add notes\n\
             drop {} debug\npick {} change b\n",
            a, notes, fixup, debug, b
        ),
    )
    .unwrap();
    let editor = format!("sequence.editor=cp '{}'", todo.display());
    assert!(!repo.git(&[
        "-c",
        &editor,
        "-c",
        "core.editor=true",
        "rebase",
        "-i",
        "main"
    ]));

    let git = repo.cli();
    let mut overview = Overview::new(repo.path());
    let snapshot = overview.refresh(&git).unwrap();
    let plan = snapshot.plan.unwrap();
    assert_eq!(plan.onto_subject, "main change");
    let actions: Vec<Action> = plan.steps.iter().map(|step| step.action).collect();
    assert_eq!(
        actions,
        [
            Action::Pick,
            Action::Reword,
            Action::Fixup,
            Action::Drop,
            Action::Pick
        ]
    );
    assert_eq!(plan.current(), Some(1));
    assert_eq!(plan.steps[2].joins, Some(2));
    assert_eq!(plan.steps[4].subject, "change b");
    assert!(snapshot.applied.is_empty());

    fs::write(repo.path().join("a.txt"), "a both\n").unwrap();
    overview
        .apply(&git, OverviewCommand::StageResolved)
        .unwrap();
    overview.apply(&git, OverviewCommand::Continue).unwrap();

    let snapshot = overview.refresh(&git).unwrap();
    let plan = snapshot.plan.unwrap();
    assert_eq!(plan.current(), Some(5));
    assert!(
        plan.steps[..4]
            .iter()
            .all(|step| step.state == StepState::Done)
    );
    assert_eq!(snapshot.applied, [2, 3, 4]);
    // Read again without moving: the message stays.
    assert_eq!(overview.refresh(&git).unwrap().applied, [2, 3, 4]);
}
