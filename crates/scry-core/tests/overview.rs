//! The operation overview on real repositories: a rebase stopped on two
//! conflicted files, resolved, staged and continued through its commands.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

use scry_core::commit_changes::{self, LineKind};
use scry_core::git::GitCli;
use scry_core::overview::{
    Overview, OverviewCommand, OverviewError, OverviewEvent, Special, Version,
};
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

    /// The full message of `commit`, without its last newline.
    fn message(&self, commit: &str) -> String {
        let output = Command::new("git")
            .envs(Repo::environment(self.root.path()))
            .arg("-C")
            .arg(self.path())
            .args(["show", "--no-patch", "--format=%B", commit])
            .output()
            .unwrap();
        String::from_utf8(output.stdout)
            .unwrap()
            .trim_end()
            .to_string()
    }

    /// The full id of `commit`.
    fn rev(&self, commit: &str) -> String {
        let output = Command::new("git")
            .envs(Repo::environment(self.root.path()))
            .arg("-C")
            .arg(self.path())
            .args(["rev-parse", commit])
            .output()
            .unwrap();
        String::from_utf8(output.stdout).unwrap().trim().to_string()
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
    // What each row offers: an unresolved file opens, nothing else.
    let actions = |file: &scry_core::overview::FileView| {
        (file.can_open(), file.can_stage(), file.can_unstage())
    };
    assert_eq!(actions(&snapshot.files[1]), (true, false, false));
    assert!(!snapshot.can_continue());
    assert!(snapshot.can_skip());

    fs::write(repo.path().join("a.txt"), "a both\n").unwrap();
    assert_eq!(state(&mut overview, &git, "a.txt"), State::Resolved);
    assert_eq!(
        actions(&file(&mut overview, &git, "a.txt")),
        (true, true, false)
    );
    assert_eq!(
        overview
            .apply(&git, OverviewCommand::StageResolved)
            .unwrap(),
        [OverviewEvent::Staged {
            paths: vec![String::from("a.txt")]
        }]
    );
    assert_eq!(state(&mut overview, &git, "a.txt"), State::Staged);
    // A staged file is unstaged before it opens again.
    assert_eq!(
        actions(&file(&mut overview, &git, "a.txt")),
        (false, false, true)
    );
    assert!(!overview.refresh(&git).unwrap().can_continue());

    assert!(matches!(
        overview.apply(&git, OverviewCommand::Continue { message: None }),
        Err(OverviewError::NotStaged(1))
    ));

    fs::write(repo.path().join("Sources/b.swift"), "let b = 3\n").unwrap();
    overview
        .apply(&git, OverviewCommand::StageResolved)
        .unwrap();
    assert!(overview.refresh(&git).unwrap().can_continue());
    assert_eq!(
        overview
            .apply(&git, OverviewCommand::Continue { message: None })
            .unwrap(),
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
/// fixup joins, and where git stands. Continue stops on the reword line,
/// so the user writes its message, even with an editor in the inherited
/// environment; then git applies the lines without conflicts on its own,
/// and the overview says which, so the jump from step 2 to step 5 is
/// explained.
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
            .envs(Repo::environment(repo.root.path()))
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

    // An editor that would fail every commit: Scry's own must replace it.
    let git = repo.cli().with_env("GIT_EDITOR", "false");
    let mut overview = Overview::new(repo.path());
    let snapshot = overview.refresh(&git).unwrap();
    assert!(snapshot.reword.is_none());
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
    overview
        .apply(&git, OverviewCommand::Continue { message: None })
        .unwrap();

    let snapshot = overview.refresh(&git).unwrap();
    assert_eq!(snapshot.plan.as_ref().unwrap().current(), Some(2));
    assert!(snapshot.applied.is_empty());
    let reword = snapshot.reword.clone().unwrap();
    assert_eq!(reword.message, "add notes");
    assert!(reword.committed);
    // The commit git made, to read while writing its message.
    let commit = reword.commit.unwrap();
    assert_eq!(commit.id, repo.rev("HEAD"));
    let paths: Vec<&str> = commit.files.iter().map(|file| file.path.as_str()).collect();
    assert_eq!(paths, ["notes.txt"]);
    assert_eq!(commit.files[0].added, Some(1));
    let lines = commit_changes::diff(&git, &commit.id, "notes.txt").unwrap();
    assert_eq!(lines.last().unwrap().text, "notes");
    // Skipping would keep the commit made, so it is not offered.
    assert!(!snapshot.can_skip());
    // "You" is git's `user.email`: the commit is the user's only when it
    // names the same address, whatever its case.
    assert!(!commit.by_you);
    fs::write(
        repo.root.path().join("gitconfig"),
        "[user]\n\temail = someone@example.com\n",
    )
    .unwrap();
    let other = overview.refresh(&git).unwrap();
    assert!(!other.reword.unwrap().commit.unwrap().by_you);
    fs::write(
        repo.root.path().join("gitconfig"),
        "[user]\n\temail = Test@Example.com\n",
    )
    .unwrap();
    let mine = overview.refresh(&git).unwrap();
    assert!(mine.reword.unwrap().commit.unwrap().by_you);
    fs::write(repo.root.path().join("gitconfig"), "").unwrap();
    // Step 1 kept its message through the commit after its conflict.
    assert_eq!(repo.message("HEAD~1"), "change a");

    overview
        .apply(
            &git,
            OverviewCommand::Continue {
                message: Some(String::from("Add the notes\n\nWith a body.")),
            },
        )
        .unwrap();

    let snapshot = overview.refresh(&git).unwrap();
    assert!(snapshot.reword.is_none());
    // The fixup joined the reworded commit and kept its new message.
    assert_eq!(repo.message("HEAD"), "Add the notes\n\nWith a body.");
    let plan = snapshot.plan.unwrap();
    assert_eq!(plan.current(), Some(5));
    // The plan keeps the todo list's id, the one the user wrote, and shows
    // the reword's new subject with the old one.
    assert_eq!(plan.steps[1].short_id.as_deref(), Some(&notes[..7]));
    assert_eq!(plan.steps[1].subject, "Add the notes");
    assert_eq!(plan.steps[1].was.as_deref(), Some("add notes"));
    let replayed = plan.steps[1].replayed_id.clone().unwrap();
    assert_ne!(replayed, notes[..7]);
    assert!(repo.rev("HEAD").starts_with(&replayed));
    // A pick keeps its subject: nothing to say it was another.
    assert_eq!(plan.steps[0].was, None);
    assert!(plan.steps[0].replayed_id.is_some());
    assert_eq!(plan.steps[2].subject, "fixup! add notes");
    assert!(
        plan.steps[..4]
            .iter()
            .all(|step| step.state == StepState::Done)
    );
    assert_eq!(snapshot.applied, [3, 4]);
    // Read again without moving: the message stays.
    assert_eq!(overview.refresh(&git).unwrap().applied, [3, 4]);
}

/// A reword line that stops on conflicts offers its message too: git
/// makes the commit once the files are staged, with the message the user
/// wrote, and does not open an editor nobody would see.
#[test]
fn a_conflicted_reword_commits_with_the_message_written() {
    let repo = Repo::stopped_rebase();
    repo.ok(&["rebase", "--abort"]);
    // Every `pick` of the todo list becomes a `reword`.
    assert!(!repo.git(&[
        "-c",
        "sequence.editor=sed -i.bak -e s/^pick/reword/",
        "rebase",
        "-i",
        "main"
    ]));

    let git = repo.cli();
    let mut overview = Overview::new(repo.path());
    let snapshot = overview.refresh(&git).unwrap();
    let reword = snapshot.reword.clone().unwrap();
    assert_eq!(reword.message, "feature change");
    assert!(!reword.committed);
    // No commit yet: the file table shows what is in it.
    assert!(reword.commit.is_none());
    fs::write(repo.path().join("a.txt"), "a both\n").unwrap();
    fs::write(repo.path().join("Sources/b.swift"), "let b = 3\n").unwrap();
    overview
        .apply(&git, OverviewCommand::StageResolved)
        .unwrap();
    overview
        .apply(
            &git,
            OverviewCommand::Continue {
                message: Some(String::from("Join both changes")),
            },
        )
        .unwrap();

    assert!(overview.refresh(&git).unwrap().operation.is_none());
    assert_eq!(repo.message("HEAD"), "Join both changes");
}

/// One file is staged on its own once resolved, and unstaged again to
/// change it: git has its conflict back, and the file keeps the content
/// the user resolved, so nothing is lost.
#[test]
fn one_file_is_staged_then_unstaged() {
    let repo = Repo::stopped_rebase();
    let git = repo.cli();
    let mut overview = Overview::new(repo.path());
    let stage = |path: &str| OverviewCommand::Stage {
        paths: vec![path.to_string()],
    };
    let unstage = |path: &str| OverviewCommand::Unstage {
        paths: vec![path.to_string()],
    };

    // Still conflicted: nothing to stage.
    assert!(matches!(
        overview.apply(&git, stage("a.txt")),
        Err(OverviewError::NotThatKind(_))
    ));
    fs::write(repo.path().join("a.txt"), "a both\n").unwrap();
    assert_eq!(
        overview.apply(&git, stage("a.txt")).unwrap(),
        [OverviewEvent::Staged {
            paths: vec![String::from("a.txt")]
        }]
    );
    assert_eq!(state(&mut overview, &git, "a.txt"), State::Staged);
    assert_eq!(
        state(&mut overview, &git, "Sources/b.swift"),
        State::Unresolved
    );

    assert_eq!(
        overview.apply(&git, unstage("a.txt")).unwrap(),
        [OverviewEvent::Unstaged {
            paths: vec![String::from("a.txt")]
        }]
    );
    assert_eq!(state(&mut overview, &git, "a.txt"), State::Resolved);
    assert_eq!(
        fs::read_to_string(repo.path().join("a.txt")).unwrap(),
        "a both\n"
    );
    // Not staged any more: nothing to unstage.
    assert!(matches!(
        overview.apply(&git, unstage("a.txt")),
        Err(OverviewError::NotThatKind(_))
    ));
}

/// A message is refused where git would not use it: outside a reword
/// line, or empty. Nothing runs, so the operation stays where it was.
#[test]
fn a_message_outside_a_reword_is_refused() {
    let repo = Repo::stopped_rebase();
    let git = repo.cli();
    let mut overview = Overview::new(repo.path());
    fs::write(repo.path().join("a.txt"), "a both\n").unwrap();
    fs::write(repo.path().join("Sources/b.swift"), "let b = 3\n").unwrap();
    overview
        .apply(&git, OverviewCommand::StageResolved)
        .unwrap();
    assert!(matches!(
        overview.apply(
            &git,
            OverviewCommand::Continue {
                message: Some(String::from("Another message")),
            },
        ),
        Err(OverviewError::Invalid(_))
    ));
    assert!(overview.refresh(&git).unwrap().operation.is_some());
}

/// A merge shows both branches since their merge base, newest first, so
/// the user sees what each side brings before resolving.
#[test]
fn a_merge_shows_both_branches_since_their_base() {
    let repo = Repo::stopped_rebase();
    repo.ok(&["rebase", "--abort"]);
    repo.ok(&["switch", "--quiet", "main"]);
    repo.commit("a main 2\n", "let b = 1\n", "second main change");
    repo.ok(&["switch", "--quiet", "feature"]);
    assert!(!repo.git(&["merge", "main"]));

    let snapshot = Overview::new(repo.path()).refresh(&repo.cli()).unwrap();
    assert!(snapshot.plan.is_none());
    let graph = snapshot.merge.unwrap();
    assert_eq!(graph.ours.name, "feature");
    assert_eq!(graph.theirs.name, "main");
    let subjects = |lane: &scry_core::merge_graph::Lane| -> Vec<String> {
        lane.commits
            .iter()
            .map(|commit| commit.subject.clone())
            .collect()
    };
    assert_eq!(subjects(&graph.ours), ["feature change"]);
    assert_eq!(
        subjects(&graph.theirs),
        ["second main change", "main change"]
    );
    assert_eq!((graph.ours.more, graph.theirs.more), (0, 0));
    assert_eq!(graph.base.subject, "base");
}

/// The files the merge editor cannot settle, in one real merge: a file
/// deleted on one side is kept or deleted, a binary file takes one
/// version, a project file keeps both sides' additions, and a string
/// catalog merges key by key with one choice left.
#[test]
fn special_files_are_settled_from_the_overview() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("gitconfig"), "").unwrap();
    fs::create_dir_all(root.path().join("repo/Sources")).unwrap();
    let repo = Repo { root };
    repo.ok(&["init", "--quiet", "--initial-branch=main"]);
    let path = repo.path();
    let catalog = |done: &str, pending: &str| {
        format!(
            "{{\n  \"sourceLanguage\" : \"en\",\n  \"strings\" : {{\n    \"done\" : {{\n      \"comment\" : \"{done}\"\n    }},\n    \"pending\" : {{\n      \"comment\" : \"{pending}\"\n    }}\n  }},\n  \"version\" : \"1.0\"\n}}\n"
        )
    };
    let project = |entry: &str| {
        format!(
            "\t\tA2000001 /* One.swift */ = {{isa = PBXFileReference; }};\n{entry}\t\tchildren = (\n\t\t\t\tA2000001 /* One.swift */,\n\t\t\t);\n"
        )
    };
    let write = |files: &[(&str, &[u8])], message: &str| {
        for (name, content) in files {
            fs::write(path.join(name), content).unwrap();
        }
        repo.ok(&["add", "-A"]);
        repo.ok(&["commit", "--quiet", "-m", message]);
    };
    write(
        &[
            ("old.txt", b"old\n"),
            ("gone.txt", b"gone\n"),
            ("icon.png", b"\x89PNG\x00base"),
            ("project.pbxproj", project("").as_bytes()),
            (
                "Localizable.xcstrings",
                catalog("Done", "Waiting").as_bytes(),
            ),
        ],
        "base",
    );
    repo.ok(&["switch", "--quiet", "-c", "feature"]);
    fs::remove_file(path.join("old.txt")).unwrap();
    write(
        &[
            ("gone.txt", b"gone, changed on feature\n"),
            ("icon.png", b"\x89PNG\x00feature"),
            (
                "project.pbxproj",
                project("\t\tA2000002 /* Two.swift */ = {isa = PBXFileReference; };\n").as_bytes(),
            ),
            (
                "Localizable.xcstrings",
                catalog("Finished", "Waiting").as_bytes(),
            ),
        ],
        "feature",
    );
    repo.ok(&["switch", "--quiet", "main"]);
    fs::remove_file(path.join("gone.txt")).unwrap();
    write(
        &[
            ("old.txt", b"old, changed on main\n"),
            ("icon.png", b"\x89PNG\x00main"),
            (
                "project.pbxproj",
                project("\t\tA2000003 /* Three.swift */ = {isa = PBXFileReference; };\n")
                    .as_bytes(),
            ),
            (
                "Localizable.xcstrings",
                catalog("All done", "Queued").as_bytes(),
            ),
        ],
        "main",
    );
    repo.ok(&["switch", "--quiet", "feature"]);
    assert!(!repo.git(&["merge", "main"]));

    let git = repo.cli();
    let mut overview = Overview::new(repo.path());
    let snapshot = overview.refresh(&git).unwrap();
    let special = |name: &str| {
        snapshot
            .files
            .iter()
            .find(|file| file.path == name)
            .unwrap()
            .special
    };
    assert_eq!(special("old.txt"), Some(Special::DeletedOnOneSide));
    assert_eq!(special("gone.txt"), Some(Special::DeletedOnOneSide));
    assert_eq!(special("icon.png"), Some(Special::Binary));
    assert_eq!(special("project.pbxproj"), Some(Special::ProjectFile));
    assert_eq!(
        special("Localizable.xcstrings"),
        Some(Special::StringCatalog)
    );

    // Deleted on one side: feature (ours) deleted old.txt, main changed it.
    let deletion = overview.deletion(&git, "old.txt").unwrap();
    assert_eq!(deletion.deleted_by, Version::Ours);
    assert!(deletion.kept_label.starts_with("main"));
    assert_eq!(deletion.kept_lines, Some(1));
    let changes: Vec<(LineKind, &str)> = deletion
        .changes
        .iter()
        .map(|line| (line.kind, line.text.as_str()))
        .collect();
    assert_eq!(
        changes,
        [
            (LineKind::Hunk, "-1 +1"),
            (LineKind::Removed, "old"),
            (LineKind::Added, "old, changed on main"),
        ]
    );
    overview
        .apply(
            &git,
            OverviewCommand::KeepFile {
                path: String::from("old.txt"),
            },
        )
        .unwrap();
    assert_eq!(
        fs::read_to_string(path.join("old.txt")).unwrap(),
        "old, changed on main\n"
    );
    overview
        .apply(
            &git,
            OverviewCommand::DeleteFile {
                path: String::from("gone.txt"),
            },
        )
        .unwrap();
    assert!(!path.join("gone.txt").exists());
    // Unstaged, the deletion is a choice to make again.
    overview
        .apply(
            &git,
            OverviewCommand::Unstage {
                paths: vec![String::from("gone.txt")],
            },
        )
        .unwrap();
    let gone = overview
        .refresh(&git)
        .unwrap()
        .files
        .into_iter()
        .find(|file| file.path == "gone.txt")
        .unwrap();
    assert_eq!(gone.state, State::Unresolved);
    assert_eq!(gone.special, Some(Special::DeletedOnOneSide));
    overview
        .apply(
            &git,
            OverviewCommand::DeleteFile {
                path: String::from("gone.txt"),
            },
        )
        .unwrap();

    // Binary: both versions previewed, theirs taken.
    let (ours, theirs) = overview.binary(&git, "icon.png").unwrap();
    assert_eq!(ours.bytes.as_deref(), Some(&b"\x89PNG\x00feature"[..]));
    assert_eq!(theirs.size, Some(9));
    overview
        .apply(
            &git,
            OverviewCommand::TakeVersion {
                path: String::from("icon.png"),
                version: Version::Theirs,
            },
        )
        .unwrap();
    assert_eq!(fs::read(path.join("icon.png")).unwrap(), b"\x89PNG\x00main");

    // Project file: each side added a file; both kept.
    let review = overview.project(&git, "project.pbxproj").unwrap();
    assert!(review.can_keep_both());
    assert_eq!(review.conflicts[0].ours_adds, ["Two.swift"]);
    assert_eq!(review.conflicts[0].theirs_adds, ["Three.swift"]);
    overview
        .apply(
            &git,
            OverviewCommand::KeepBothAdditions {
                path: String::from("project.pbxproj"),
                remove_missing: true,
            },
        )
        .unwrap();
    let project_file = fs::read_to_string(path.join("project.pbxproj")).unwrap();
    assert!(project_file.contains("Two.swift") && project_file.contains("Three.swift"));
    assert!(!project_file.contains("<<<<<<<"));

    // String catalog: "pending" changed on main only merges by itself;
    // "done" changed on both sides is the one choice.
    let merge = overview.catalog(&git, "Localizable.xcstrings").unwrap();
    assert_eq!(merge.automatic, 1);
    assert_eq!(merge.conflicts.len(), 1);
    assert_eq!(merge.conflicts[0].key.as_deref(), Some("done"));
    overview
        .apply(
            &git,
            OverviewCommand::ResolveCatalog {
                path: String::from("Localizable.xcstrings"),
                picks: vec![Version::Theirs],
            },
        )
        .unwrap();
    assert_eq!(
        fs::read_to_string(path.join("Localizable.xcstrings")).unwrap(),
        catalog("All done", "Queued")
    );

    // The two written files are resolved; staging them ends the merge.
    let snapshot = overview.refresh(&git).unwrap();
    assert_eq!(snapshot.count(State::Resolved), 2);
    overview
        .apply(&git, OverviewCommand::StageResolved)
        .unwrap();
    assert!(overview.refresh(&git).unwrap().can_continue());
}

/// Generated files in one real merge: a `Package.resolved` merges package
/// by package with one choice left, a lockfile lists each side's upgrades
/// and is taken unstaged so the package manager can write it again, and a
/// `go.sum` keeps both sides' checksums.
#[test]
fn lockfiles_and_package_pins_are_settled_from_the_overview() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("gitconfig"), "").unwrap();
    fs::create_dir_all(root.path().join("repo/ios")).unwrap();
    let repo = Repo { root };
    repo.ok(&["init", "--quiet", "--initial-branch=main"]);
    let path = repo.path();
    let pin = |identity: &str, version: &str| {
        format!(
            "    {{\n      \"identity\" : \"{identity}\",\n      \"kind\" : \"remoteSourceControl\",\n      \"location\" : \"https://github.com/example/{identity}\",\n      \"state\" : {{\n        \"revision\" : \"r{version}\",\n        \"version\" : \"{version}\"\n      }}\n    }}"
        )
    };
    let resolved = |alamofire: &str, kingfisher: &str| {
        format!(
            "{{\n  \"originHash\" : \"h\",\n  \"pins\" : [\n{},\n{}\n  ],\n  \"version\" : 3\n}}\n",
            pin("alamofire", alamofire),
            pin("kingfisher", kingfisher)
        )
    };
    let pods = |alamofire: &str, kingfisher: &str| {
        format!(
            "PODS:\n  - Alamofire ({alamofire})\n  - Kingfisher ({kingfisher})\n\nCOCOAPODS: 1.15.2\n"
        )
    };
    let write = |files: &[(&str, String)], message: &str| {
        for (name, content) in files {
            fs::write(path.join(name), content).unwrap();
        }
        repo.ok(&["add", "-A"]);
        repo.ok(&["commit", "--quiet", "-m", message]);
    };
    write(
        &[
            ("ios/Package.resolved", resolved("5.8.0", "7.9.0")),
            ("ios/Podfile.lock", pods("5.8.0", "7.9.0")),
            ("go.sum", String::from("a v1 h1:a\nb v1 h1:b\n")),
        ],
        "base",
    );
    repo.ok(&["switch", "--quiet", "-c", "feature"]);
    write(
        &[
            ("ios/Package.resolved", resolved("5.8.1", "7.9.0")),
            ("ios/Podfile.lock", pods("5.8.1", "7.9.0")),
            ("go.sum", String::from("a v1 h1:a\nb v1 h1:b\nc v1 h1:c\n")),
        ],
        "feature",
    );
    repo.ok(&["switch", "--quiet", "main"]);
    write(
        &[
            ("ios/Package.resolved", resolved("5.9.0", "7.10.0")),
            ("ios/Podfile.lock", pods("5.9.0", "7.10.0")),
            ("go.sum", String::from("a v1 h1:a\nb v2 h1:b2\n")),
        ],
        "main",
    );
    repo.ok(&["switch", "--quiet", "feature"]);
    assert!(!repo.git(&["merge", "main"]));

    let git = repo.cli();
    let mut overview = Overview::new(repo.path());
    let snapshot = overview.refresh(&git).unwrap();
    let file = |name: &str| {
        snapshot
            .files
            .iter()
            .find(|file| file.path == name)
            .unwrap()
    };
    assert_eq!(
        file("ios/Package.resolved").special,
        Some(Special::PackageResolved)
    );
    assert_eq!(file("ios/Package.resolved").kind, "Swift packages");
    assert_eq!(file("ios/Podfile.lock").special, Some(Special::Lockfile));
    assert_eq!(file("ios/Podfile.lock").kind, "CocoaPods lockfile");
    assert_eq!(file("go.sum").special, Some(Special::Lockfile));

    // Package.resolved: kingfisher from main, alamofire chosen.
    let merge = overview.packages(&git, "ios/Package.resolved").unwrap();
    assert_eq!(merge.automatic.len(), 1);
    assert_eq!(merge.conflicts.len(), 1);
    assert_eq!(merge.conflicts[0].identity, "alamofire");
    overview
        .apply(
            &git,
            OverviewCommand::ResolvePackages {
                path: String::from("ios/Package.resolved"),
                picks: vec![Version::Theirs],
            },
        )
        .unwrap();
    assert_eq!(
        fs::read_to_string(path.join("ios/Package.resolved")).unwrap(),
        resolved("5.9.0", "7.10.0")
    );

    // Podfile.lock: both upgrades listed, ours taken but not staged.
    let review = overview.lockfile(&git, "ios/Podfile.lock").unwrap();
    assert_eq!(review.tool, "CocoaPods");
    assert_eq!(review.manifest, "ios/Podfile");
    assert!(!review.manifest_conflicted);
    assert_eq!(review.command, "cd ios && pod install");
    assert_eq!(review.changes.len(), 2);
    assert!(review.changes[0].both_changed());
    overview
        .apply(
            &git,
            OverviewCommand::TakeLockfile {
                path: String::from("ios/Podfile.lock"),
                version: Version::Ours,
            },
        )
        .unwrap();
    assert_eq!(
        fs::read_to_string(path.join("ios/Podfile.lock")).unwrap(),
        pods("5.8.1", "7.9.0")
    );

    // go.sum: c added on feature, b moved to v2 on main.
    assert!(overview.lockfile(&git, "go.sum").unwrap().merges_by_lines);
    overview
        .apply(
            &git,
            OverviewCommand::MergeLockfileLines {
                path: String::from("go.sum"),
            },
        )
        .unwrap();
    assert_eq!(
        fs::read_to_string(path.join("go.sum")).unwrap(),
        "a v1 h1:a\nb v2 h1:b2\nc v1 h1:c\n"
    );

    // Written, not staged: the overview shows them resolved.
    let snapshot = overview.refresh(&git).unwrap();
    for name in ["ios/Package.resolved", "ios/Podfile.lock", "go.sum"] {
        let file = snapshot
            .files
            .iter()
            .find(|file| file.path == name)
            .unwrap();
        assert_eq!(file.state, State::Resolved, "{name}");
    }
}

/// A file name with glob characters is a name, not a pattern: staging the
/// conflicted `file[1].txt` once also staged the unrelated `file1.txt`
/// it matched as a pattern.
#[test]
fn a_name_with_glob_characters_stages_only_itself() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("gitconfig"), "").unwrap();
    fs::create_dir_all(root.path().join("repo")).unwrap();
    let repo = Repo { root };
    let path = repo.path();
    repo.ok(&["init", "--quiet", "--initial-branch=main"]);
    let write = |name: &str, text: &str| fs::write(path.join(name), text).unwrap();
    write("file[1].txt", "base\n");
    write("file1.txt", "untouched\n");
    repo.ok(&["add", "."]);
    repo.ok(&["commit", "--quiet", "-m", "base"]);
    repo.ok(&["switch", "--quiet", "-c", "feature"]);
    write("file[1].txt", "feature\n");
    repo.ok(&["commit", "--quiet", "-am", "feature"]);
    repo.ok(&["switch", "--quiet", "main"]);
    write("file[1].txt", "main\n");
    repo.ok(&["commit", "--quiet", "-am", "main"]);
    assert!(!repo.git(&["merge", "feature"]));
    write("file[1].txt", "both\n");
    write("file1.txt", "changed, not to stage\n");

    let git = repo.cli();
    let mut overview = Overview::new(repo.path());
    overview
        .apply(
            &git,
            OverviewCommand::Stage {
                paths: vec![String::from("file[1].txt")],
            },
        )
        .unwrap();
    let staged = Command::new("git")
        .envs(Repo::environment(repo.root.path()))
        .args([
            "-C",
            path.to_str().unwrap(),
            "diff",
            "--cached",
            "--name-only",
        ])
        .envs(Repo::environment(repo.root.path()))
        .output()
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&staged.stdout), "file[1].txt\n");
}

/// The plan reads the todo list as git writes it (`pick <id> # <subject>`):
/// subjects come without the `#`, and a commit replayed as it was is not
/// shown as reworded. The other plan tests write their list by hand.
#[test]
fn the_plan_reads_the_todo_list_git_writes() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("gitconfig"), "").unwrap();
    fs::create_dir_all(root.path().join("repo/Sources")).unwrap();
    let repo = Repo { root };
    repo.ok(&["init", "--quiet", "--initial-branch=main"]);
    repo.commit("a\n", "let b = 0\n", "base");
    repo.ok(&["switch", "--quiet", "-c", "feature"]);
    fs::write(repo.path().join("notes.txt"), "notes\n").unwrap();
    repo.ok(&["add", "."]);
    repo.ok(&["commit", "--quiet", "-m", "add notes"]);
    repo.commit("a feature\n", "let b = 2\n", "change a");
    fs::write(repo.path().join("more.txt"), "more\n").unwrap();
    repo.ok(&["add", "."]);
    repo.ok(&["commit", "--quiet", "-m", "add more"]);
    repo.ok(&["switch", "--quiet", "main"]);
    repo.commit("a main\n", "let b = 1\n", "main change");
    repo.ok(&["switch", "--quiet", "feature"]);
    assert!(!repo.git(&["rebase", "main"]));

    let git = repo.cli();
    let mut overview = Overview::new(repo.path());
    let plan = overview.refresh(&git).unwrap().plan.unwrap();
    let subjects: Vec<&str> = plan
        .steps
        .iter()
        .map(|step| step.subject.as_str())
        .collect();
    assert_eq!(subjects, ["add notes", "change a", "add more"]);
    assert_eq!(plan.steps[0].state, StepState::Done);
    assert_eq!(plan.steps[0].was, None);
    assert_eq!(plan.steps[1].state, StepState::Current);
}

/// A repository whose `feature` has the given commits (file, content,
/// message), all with the same author date, on a `main` that changed
/// `a.txt`; it gives the commits' ids.
fn same_second_commits(repo: &Repo, commits: &[(&str, &str, &str)]) -> Vec<String> {
    repo.ok(&["init", "--quiet", "--initial-branch=main"]);
    fs::write(repo.path().join("a.txt"), "a\n").unwrap();
    repo.ok(&["add", "."]);
    repo.ok(&["commit", "--quiet", "-m", "base"]);
    repo.ok(&["switch", "--quiet", "-c", "feature"]);
    let mut ids = Vec::new();
    for (file, content, message) in commits {
        fs::write(repo.path().join(file), content).unwrap();
        repo.ok(&["add", "."]);
        let committed = Command::new("git")
            .arg("-C")
            .arg(repo.path())
            .args(["commit", "--quiet", "-m", message])
            .envs(Repo::environment(repo.root.path()))
            .env("GIT_AUTHOR_DATE", "1700000000 +0100")
            .output()
            .unwrap();
        assert!(committed.status.success());
        let id = Command::new("git")
            .envs(Repo::environment(repo.root.path()))
            .arg("-C")
            .arg(repo.path())
            .args(["rev-parse", "HEAD"])
            .output()
            .unwrap();
        ids.push(String::from_utf8(id.stdout).unwrap().trim().to_string());
    }
    repo.ok(&["switch", "--quiet", "main"]);
    fs::write(repo.path().join("a.txt"), "a main\n").unwrap();
    repo.ok(&["commit", "--quiet", "-am", "main change"]);
    repo.ok(&["switch", "--quiet", "feature"]);
    ids
}

/// Starts `rebase -i main` with this todo list.
fn rebase_with(repo: &Repo, todo: &str) {
    let file = repo.root.path().join("todo");
    fs::write(&file, todo).unwrap();
    let editor = format!("sequence.editor=cp '{}'", file.display());
    assert!(!repo.git(&[
        "-c",
        &editor,
        "-c",
        "core.editor=true",
        "rebase",
        "-i",
        "main"
    ]));
}

fn subjects(repo: &Repo) -> Vec<String> {
    let log = Command::new("git")
        .envs(Repo::environment(repo.root.path()))
        .arg("-C")
        .arg(repo.path())
        .args(["log", "--format=%s", "main..HEAD"])
        .output()
        .unwrap();
    String::from_utf8(log.stdout)
        .unwrap()
        .lines()
        .map(str::to_string)
        .collect()
}

/// A reword stopped on conflicts whose previous commit has the same author
/// and date (scripts, `commit -C`) was taken for "already committed", and
/// Continue amended the previous commit, folding the two together. The
/// staged resolution tells them apart: the reword gets its message, and
/// both commits stay.
#[test]
fn a_conflicted_reword_never_amends_the_commit_before() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("gitconfig"), "").unwrap();
    fs::create_dir_all(root.path().join("repo")).unwrap();
    let repo = Repo { root };
    let ids = same_second_commits(
        &repo,
        &[
            ("notes.txt", "notes\n", "add notes"),
            ("a.txt", "a feature\n", "change a"),
        ],
    );
    rebase_with(
        &repo,
        &format!("pick {} add notes\nreword {} change a\n", ids[0], ids[1]),
    );
    fs::write(repo.path().join("a.txt"), "a both\n").unwrap();
    repo.ok(&["add", "a.txt"]);

    let git = repo.cli();
    let mut overview = Overview::new(repo.path());
    let reword = overview.refresh(&git).unwrap().reword.unwrap();
    assert!(!reword.committed);
    overview
        .apply(
            &git,
            OverviewCommand::Continue {
                message: Some(String::from("change a, both sides")),
            },
        )
        .unwrap();
    assert_eq!(subjects(&repo), ["change a, both sides", "add notes"]);
}

/// A reword whose resolution leaves nothing to commit is dropped by git,
/// which goes on to the next line: the message written for it used to land
/// on that next commit. It now waits for its own commit, and the next
/// reword stops for its message as usual.
#[test]
fn a_reword_message_never_lands_on_another_commit() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("gitconfig"), "").unwrap();
    fs::create_dir_all(root.path().join("repo")).unwrap();
    let repo = Repo { root };
    let ids = same_second_commits(
        &repo,
        &[
            ("a.txt", "a feature\n", "change a"),
            ("b.txt", "b\n", "add b"),
        ],
    );
    rebase_with(
        &repo,
        &format!("reword {} change a\nreword {} add b\n", ids[0], ids[1]),
    );
    // Resolved as main has it: the reword has nothing left to commit.
    fs::write(repo.path().join("a.txt"), "a main\n").unwrap();
    repo.ok(&["add", "a.txt"]);

    let git = repo.cli();
    let mut overview = Overview::new(repo.path());
    overview.refresh(&git).unwrap();
    overview
        .apply(
            &git,
            OverviewCommand::Continue {
                message: Some(String::from("message for change a")),
            },
        )
        .unwrap();
    let snapshot = overview.refresh(&git).unwrap();
    let reword = snapshot.reword.expect("stopped on the next reword");
    assert!(reword.committed);
    assert_eq!(reword.message, "add b");
    assert_eq!(subjects(&repo), ["add b"]);
}

/// A message whose subject starts with `#` (`#42 …`, an issue number) was
/// emptied by git, which strips lines starting with its comment character:
/// it is now committed as written.
#[test]
fn a_reword_message_starting_with_a_hash_is_kept() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("gitconfig"), "").unwrap();
    fs::create_dir_all(root.path().join("repo")).unwrap();
    let repo = Repo { root };
    let ids = same_second_commits(&repo, &[("a.txt", "a feature\n", "change a")]);
    rebase_with(&repo, &format!("reword {} change a\n", ids[0]));
    fs::write(repo.path().join("a.txt"), "a both\n").unwrap();
    repo.ok(&["add", "a.txt"]);

    let git = repo.cli();
    let mut overview = Overview::new(repo.path());
    overview.refresh(&git).unwrap();
    overview
        .apply(
            &git,
            OverviewCommand::Continue {
                message: Some(String::from(
                    "#42 Keep both sides of a\n\n# not a comment either",
                )),
            },
        )
        .unwrap();
    let body = Command::new("git")
        .envs(Repo::environment(repo.root.path()))
        .arg("-C")
        .arg(repo.path())
        .args(["log", "-1", "--format=%B"])
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8(body.stdout).unwrap().trim_end(),
        "#42 Keep both sides of a\n\n# not a comment either"
    );
}

/// Skip used to report success whenever conflicts were left, even when git
/// failed and stayed on the same commit (here a leftover `index.lock`): the
/// failure is now reported.
#[test]
fn a_failed_skip_is_reported() {
    let repo = Repo::stopped_rebase();
    let git = repo.cli();
    let mut overview = Overview::new(repo.path());
    overview.refresh(&git).unwrap();
    let lock = repo.path().join(".git/index.lock");
    fs::write(&lock, "").unwrap();
    assert!(overview.apply(&git, OverviewCommand::Skip).is_err());
    fs::remove_file(&lock).unwrap();
    assert!(overview.apply(&git, OverviewCommand::Skip).is_ok());
    assert!(overview.refresh(&git).unwrap().operation.is_none());
}

/// A repository whose `main` and `feature` were made by `setup`, which
/// stages its own changes; `feature` is merged into `main` with conflicts.
fn stopped_merge(setup: impl Fn(&Repo, &str)) -> Repo {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("gitconfig"), "").unwrap();
    fs::create_dir_all(root.path().join("repo")).unwrap();
    let repo = Repo { root };
    repo.ok(&["init", "--quiet", "--initial-branch=main"]);
    setup(&repo, "base");
    repo.ok(&["commit", "--quiet", "-m", "base"]);
    repo.ok(&["switch", "--quiet", "-c", "feature"]);
    setup(&repo, "feature");
    repo.ok(&["commit", "--quiet", "-m", "feature"]);
    repo.ok(&["switch", "--quiet", "main"]);
    setup(&repo, "main");
    repo.ok(&["commit", "--quiet", "-m", "main"]);
    assert!(!repo.git(&["merge", "feature"]));
    repo
}

/// The file at `path` in the overview.
fn file(overview: &mut Overview, git: &GitCli, path: &str) -> scry_core::overview::FileView {
    let snapshot = overview.refresh(git).unwrap();
    snapshot
        .files
        .into_iter()
        .find(|file| file.path == path)
        .unwrap()
}

/// With `-merge` in `.gitattributes`, git leaves ours in the folder
/// without markers. That is not a resolution: the file waits for a version
/// to be taken, and Stage resolved leaves it alone, so the other side's
/// change is never dropped without a decision.
#[test]
fn a_file_git_did_not_merge_waits_for_a_version() {
    let repo = stopped_merge(|repo, side| {
        fs::write(repo.path().join(".gitattributes"), "*.json -merge\n").unwrap();
        fs::write(
            repo.path().join("data.json"),
            format!("{{\"{side}\": 1}}\n"),
        )
        .unwrap();
        repo.ok(&["add", "--all"]);
    });
    let git = repo.cli();
    let mut overview = Overview::new(repo.path());

    let data = file(&mut overview, &git, "data.json");
    assert_eq!(data.state, State::Unresolved);
    assert_eq!(data.special, Some(Special::NotMerged));
    assert!(!data.openable);

    overview
        .apply(&git, OverviewCommand::StageResolved)
        .unwrap();
    assert_eq!(state(&mut overview, &git, "data.json"), State::Unresolved);

    overview
        .apply(
            &git,
            OverviewCommand::TakeVersion {
                path: String::from("data.json"),
                version: Version::Theirs,
            },
        )
        .unwrap();
    assert_eq!(state(&mut overview, &git, "data.json"), State::Staged);
    assert_eq!(
        fs::read_to_string(repo.path().join("data.json")).unwrap(),
        "{\"feature\": 1}\n"
    );

    // Edited by hand, the same file counts as resolved.
    let repo = stopped_merge(|repo, side| {
        fs::write(repo.path().join(".gitattributes"), "*.json -merge\n").unwrap();
        fs::write(
            repo.path().join("data.json"),
            format!("{{\"{side}\": 1}}\n"),
        )
        .unwrap();
        repo.ok(&["add", "--all"]);
    });
    fs::write(repo.path().join("data.json"), "{\"both\": 1}\n").unwrap();
    let mut overview = Overview::new(repo.path());
    assert_eq!(
        state(&mut overview, &repo.cli(), "data.json"),
        State::Resolved
    );
}

/// Both sides rename the same file: git leaves the old name with only its
/// base and each new name with one side. None of them is "deleted on one
/// side"; each is marked resolved as it is in the folder, and the merge
/// can then continue.
#[test]
fn a_rename_on_both_sides_is_marked_resolved_as_in_the_folder() {
    let repo = stopped_merge(|repo, side| {
        let path = repo.path();
        match side {
            "base" => {
                fs::write(path.join("old.txt"), "text\n").unwrap();
                repo.ok(&["add", "old.txt"]);
            }
            "feature" => repo.ok(&["mv", "old.txt", "theirs.txt"]),
            _ => repo.ok(&["mv", "old.txt", "ours.txt"]),
        }
    });
    let git = repo.cli();
    let mut overview = Overview::new(repo.path());
    for (path, kind) in [
        ("old.txt", "Renamed on both sides"),
        ("ours.txt", "Name on one side only"),
        ("theirs.txt", "Name on one side only"),
    ] {
        let view = file(&mut overview, &git, path);
        assert_eq!(view.special, Some(Special::Unusual), "{path}");
        assert_eq!(view.kind, kind, "{path}");
    }
    let old = overview.unusual(&git, "old.txt").unwrap();
    assert!(!old.in_folder);
    assert!(overview.unusual(&git, "ours.txt").unwrap().in_folder);

    for path in ["old.txt", "ours.txt", "theirs.txt"] {
        overview
            .apply(
                &git,
                OverviewCommand::MarkResolved {
                    path: String::from(path),
                },
            )
            .unwrap();
    }
    let snapshot = overview.refresh(&git).unwrap();
    assert!(snapshot.can_continue());
    overview
        .apply(&git, OverviewCommand::Continue { message: None })
        .unwrap();
    assert!(repo.path().join("ours.txt").exists());
    assert!(repo.path().join("theirs.txt").exists());
    assert!(!repo.path().join("old.txt").exists());
}

/// Mark resolved is only for the unusual shapes: an ordinary text
/// conflict, still full of markers, is refused.
#[test]
fn mark_resolved_refuses_an_ordinary_conflict() {
    let repo = Repo::stopped_rebase();
    let git = repo.cli();
    let mut overview = Overview::new(repo.path());
    let error = overview
        .apply(
            &git,
            OverviewCommand::MarkResolved {
                path: String::from("a.txt"),
            },
        )
        .unwrap_err();
    assert!(matches!(error, OverviewError::NotThatKind(_)));
    assert_eq!(state(&mut overview, &git, "a.txt"), State::Unresolved);
}

/// A submodule both sides moved is not a text file to open: it is marked
/// resolved once a commit is checked out in it, and git refuses while
/// none is, so nothing half-done is staged.
#[test]
fn a_submodule_conflict_is_marked_resolved_once_a_commit_is_checked_out() {
    let link = |repo: &Repo, commit: &str| {
        repo.ok(&[
            "update-index",
            "--add",
            "--cacheinfo",
            &format!("160000,{},sub", commit.repeat(40)),
        ]);
    };
    let repo = stopped_merge(|repo, side| {
        fs::write(repo.path().join("f.txt"), "f\n").unwrap();
        repo.ok(&["add", "f.txt"]);
        link(
            repo,
            match side {
                "base" => "1",
                "feature" => "2",
                _ => "3",
            },
        );
    });
    let git = repo.cli();
    let mut overview = Overview::new(repo.path());
    let sub = file(&mut overview, &git, "sub");
    assert_eq!(sub.special, Some(Special::Unusual));
    assert_eq!(sub.kind, "Submodule");

    let mark = |overview: &mut Overview| {
        overview.apply(
            &git,
            OverviewCommand::MarkResolved {
                path: String::from("sub"),
            },
        )
    };
    assert!(mark(&mut overview).is_err());

    let sub_path = repo.path().join("sub");
    for args in [
        vec!["init", "--quiet", "--initial-branch=main"],
        vec!["commit", "--quiet", "--allow-empty", "-m", "chosen"],
    ] {
        assert!(
            Command::new("git")
                .arg("-C")
                .arg(&sub_path)
                .args(&args)
                .envs(Repo::environment(repo.root.path()))
                .status()
                .unwrap()
                .success()
        );
    }
    mark(&mut overview).unwrap();
    assert_eq!(state(&mut overview, &git, "sub"), State::Staged);
}

/// Files staged at this step before the overview opened (by an earlier
/// Scry window, or `git add` in a terminal) are still listed as staged,
/// so Unstage stays within reach after a restart.
#[test]
fn files_staged_before_opening_are_listed_as_staged() {
    let repo = Repo::stopped_rebase();
    fs::write(repo.path().join("a.txt"), "a both\n").unwrap();
    repo.ok(&["add", "a.txt"]);

    let git = repo.cli();
    let mut overview = Overview::new(repo.path());
    let snapshot = overview.refresh(&git).unwrap();
    let paths: Vec<&str> = snapshot
        .files
        .iter()
        .map(|file| file.path.as_str())
        .collect();
    assert_eq!(paths, ["Sources/b.swift", "a.txt"]);
    assert_eq!(state(&mut overview, &git, "a.txt"), State::Staged);

    overview
        .apply(
            &git,
            OverviewCommand::Unstage {
                paths: vec![String::from("a.txt")],
            },
        )
        .unwrap();
    assert_eq!(state(&mut overview, &git, "a.txt"), State::Resolved);
}

/// Several rows are staged, or unstaged, in one command and one git run.
/// With a file not resolved among them, nothing is staged: a selection is
/// taken whole or not at all.
#[test]
fn several_files_are_staged_and_unstaged_in_one_command() {
    let repo = Repo::stopped_rebase();
    let git = repo.cli();
    let mut overview = Overview::new(repo.path());
    let both = vec![String::from("a.txt"), String::from("Sources/b.swift")];
    fs::write(repo.path().join("a.txt"), "a both\n").unwrap();

    let error = overview
        .apply(
            &git,
            OverviewCommand::Stage {
                paths: both.clone(),
            },
        )
        .unwrap_err();
    assert!(matches!(error, OverviewError::NotThatKind(path) if path == "Sources/b.swift"));
    assert_eq!(state(&mut overview, &git, "a.txt"), State::Resolved);

    fs::write(repo.path().join("Sources/b.swift"), "let b = 3\n").unwrap();
    overview
        .apply(
            &git,
            OverviewCommand::Stage {
                paths: both.clone(),
            },
        )
        .unwrap();
    assert!(overview.refresh(&git).unwrap().can_continue());

    overview
        .apply(&git, OverviewCommand::Unstage { paths: both })
        .unwrap();
    assert_eq!(state(&mut overview, &git, "a.txt"), State::Resolved);
    assert_eq!(
        state(&mut overview, &git, "Sources/b.swift"),
        State::Resolved
    );
}

/// Commands that write a file act only on a file in conflict at this
/// step: on a clean `go.sum`, or a path outside the repository, they are
/// refused and the file is left as it is.
#[test]
fn file_commands_refuse_a_path_not_in_conflict() {
    let repo = Repo::stopped_rebase();
    fs::write(repo.path().join("go.sum"), "kept v1.0.0 h1:abc=\n").unwrap();
    let outside = repo.root.path().join("outside.txt");
    fs::write(&outside, "kept\n").unwrap();
    let git = repo.cli();
    let mut overview = Overview::new(repo.path());

    for path in [String::from("go.sum"), outside.display().to_string()] {
        let error = overview
            .apply(
                &git,
                OverviewCommand::MergeLockfileLines { path: path.clone() },
            )
            .unwrap_err();
        assert!(matches!(error, OverviewError::NotThatKind(_)), "{path}");
        let error = overview
            .apply(
                &git,
                OverviewCommand::TakeVersion {
                    path,
                    version: Version::Theirs,
                },
            )
            .unwrap_err();
        assert!(matches!(error, OverviewError::NotThatKind(_)));
    }
    assert_eq!(
        fs::read_to_string(repo.path().join("go.sum")).unwrap(),
        "kept v1.0.0 h1:abc=\n"
    );
    assert_eq!(fs::read_to_string(&outside).unwrap(), "kept\n");
}

/// What the core shows reads as JSON (portability rule 2): another front
/// end, or an MCP server, can take the overview and a file's rows,
/// conflicts and header as they are, without a mapping of its own.
#[test]
fn the_overview_and_a_file_read_as_json() {
    let repo = Repo::stopped_rebase();
    let git = repo.cli();
    let mut overview = Overview::new(repo.path());
    let snapshot = overview.refresh(&git).unwrap();
    let json = serde_json::to_value(&snapshot).unwrap();
    assert_eq!(json["operation"]["kind"], "rebase");
    assert_eq!(json["files"][0]["path"], "Sources/b.swift");
    assert_eq!(json["files"][0]["state"], "unresolved");

    let path = repo.path().join("a.txt");
    let session = scry_core::session::open_file(&git, &path).unwrap();
    let operation = snapshot.operation.as_ref();
    let header = scry_core::presentation::header(&path, &session, operation);
    let conflict = session.conflict_view(0, "ours", "theirs");
    let rows = session.rows(false);
    for value in [
        serde_json::to_value(&header).unwrap(),
        serde_json::to_value(&conflict).unwrap(),
        serde_json::to_value(&rows).unwrap(),
    ] {
        assert!(!value.is_null());
    }
    assert_eq!(
        serde_json::to_value(&conflict).unwrap()["state"],
        "unresolved"
    );
}

/// A file opened from the overview reads the sides' final line break from the index:
/// the markers git writes have one after each side, so without it a file that had
/// none would gain one when saved.
#[test]
fn a_file_opened_from_the_overview_keeps_no_final_line_break() {
    let repo = stopped_merge(|repo, side| {
        fs::write(repo.path().join("a.txt"), format!("a\n{side}")).unwrap();
        repo.ok(&["add", "--all"]);
    });
    let path = repo.path().join("a.txt");
    let git = repo.cli();
    let mut session = scry_core::session::open_file(&git, &path).unwrap();
    session
        .apply(scry_core::command::Command::Choose {
            conflict: 0,
            candidate: scry_core::resolution::CandidateKind::Theirs,
        })
        .unwrap();
    assert_eq!(session.merged_text().as_deref(), Some("a\nfeature"));
}

/// A Markdown file whose sides both add an underlined heading with text under it opens
/// from the overview: the `=======` lines inside the conflict make git's markers
/// unreadable, so its versions are read from the index and the conflict rebuilt with
/// longer markers. (Sides ending with the underline would share it outside the conflict,
/// and the file would open without being rebuilt.)
#[test]
fn a_file_with_a_separator_line_opens_from_the_overview() {
    let repo = stopped_merge(|repo, side| {
        let text = match side {
            "base" => String::from("# Title\n"),
            side => format!("# Title\n\n{side}\n=======\n{side} text\n"),
        };
        fs::write(repo.path().join("README.md"), text).unwrap();
        repo.ok(&["add", "--all"]);
    });
    let path = repo.path().join("README.md");
    let written = fs::read_to_string(&path).unwrap();
    assert!(scry_core::conflict::parse(&written).is_err(), "{written}");
    let session = scry_core::session::open_file(&repo.cli(), &path).unwrap();
    assert!(session.rebuilt());
    assert_eq!(session.conflict_count(), 1);
    assert_eq!(session.conflict(0).ours.text, "main\n=======\nmain text\n");
    assert_eq!(
        session.conflict(0).theirs.text,
        "feature\n=======\nfeature text\n"
    );
}

/// Counts the git runs of a `GitCli`.
struct Counting {
    git: GitCli,
    runs: std::cell::Cell<usize>,
}

impl scry_core::git::Git for Counting {
    fn output(&self, args: &[&str]) -> Result<scry_core::git::GitOutput, scry_core::git::GitError> {
        self.runs.set(self.runs.get() + 1);
        self.git.output(args)
    }

    fn output_with_input(
        &self,
        args: &[&str],
        input: &[u8],
    ) -> Result<scry_core::git::GitOutput, scry_core::git::GitError> {
        self.runs.set(self.runs.get() + 1);
        self.git.output_with_input(args, input)
    }

    fn output_with_env(
        &self,
        args: &[&str],
        env: &[(&str, &str)],
    ) -> Result<scry_core::git::GitOutput, scry_core::git::GitError> {
        self.runs.set(self.runs.get() + 1);
        self.git.output_with_env(args, env)
    }
}

/// A refresh runs as many gits for twenty files in conflict as for one: their stages
/// and attributes are read in one run each, since the app refreshes on every activation.
#[test]
fn a_refresh_does_not_run_git_per_file() {
    let runs = |files: usize| {
        let repo = stopped_merge(|repo, side| {
            for file in 0..files {
                fs::write(
                    repo.path().join(format!("f{file}.txt")),
                    format!("{side}\n"),
                )
                .unwrap();
            }
            repo.ok(&["add", "--all"]);
        });
        let git = Counting {
            git: repo.cli(),
            runs: std::cell::Cell::new(0),
        };
        let mut overview = Overview::new(repo.path());
        let snapshot = overview.refresh(&git).unwrap();
        assert_eq!(snapshot.files.len(), files);
        git.runs.get()
    };
    assert_eq!(runs(20), runs(1));
}

/// `git stash pop` leaves conflicts without an operation: the files are
/// still listed and can be staged, with nothing to continue or skip.
#[test]
fn a_stash_pop_conflict_lists_its_files() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("gitconfig"), "").unwrap();
    fs::create_dir_all(root.path().join("repo/Sources")).unwrap();
    let repo = Repo { root };
    repo.ok(&["init", "--quiet", "--initial-branch=main"]);
    repo.commit("a\n", "let b = 0\n", "base");
    fs::write(repo.path().join("a.txt"), "a stashed\n").unwrap();
    repo.ok(&["stash", "--quiet"]);
    repo.commit("a main\n", "let b = 0\n", "main change");
    assert!(!repo.git(&["stash", "pop"]));
    let git = repo.cli();
    let mut overview = Overview::new(repo.path());

    let snapshot = overview.refresh(&git).unwrap();
    assert!(snapshot.operation.is_none());
    let paths: Vec<&str> = snapshot
        .files
        .iter()
        .map(|file| file.path.as_str())
        .collect();
    assert_eq!(paths, ["a.txt"]);
    assert_eq!(snapshot.files[0].state, State::Unresolved);
    assert!(!snapshot.can_continue());
    assert!(!snapshot.can_skip());

    fs::write(repo.path().join("a.txt"), "a main and stashed\n").unwrap();
    overview
        .apply(
            &git,
            OverviewCommand::Stage {
                paths: vec![String::from("a.txt")],
            },
        )
        .unwrap();
    assert_eq!(state(&mut overview, &git, "a.txt"), State::Staged);
}

/// A binary version is read for its preview when it is a few megabytes,
/// as images are; a version too large to preview is never read, only
/// measured, so a large asset does not pass through memory.
#[test]
fn large_binary_versions_are_measured_not_read() {
    const MB: usize = 1024 * 1024;
    let repo = stopped_merge(|repo, side| {
        let fill = side.as_bytes()[0];
        let mut image = vec![0u8; 2 * MB];
        image[0] = fill;
        fs::write(repo.path().join("photo.png"), &image).unwrap();
        let mut asset = vec![0u8; 9 * MB];
        asset[0] = fill;
        fs::write(repo.path().join("asset.bin"), &asset).unwrap();
        repo.ok(&["add", "."]);
    });
    let git = repo.cli();
    let mut overview = Overview::new(repo.path());
    overview.refresh(&git).unwrap();

    let (ours, theirs) = overview.binary(&git, "photo.png").unwrap();
    assert_eq!(ours.size, Some(2 * MB as u64));
    assert_eq!(ours.bytes.as_ref().map(|bytes| bytes[0]), Some(b'm'));
    assert_eq!(theirs.bytes.as_ref().map(|bytes| bytes[0]), Some(b'f'));

    let (ours, theirs) = overview.binary(&git, "asset.bin").unwrap();
    assert_eq!(ours.size, Some(9 * MB as u64));
    assert_eq!(theirs.size, Some(9 * MB as u64));
    assert!(ours.bytes.is_none() && theirs.bytes.is_none());
}

/// Git's default style writes a project's conflicts without the base; the
/// review rebuilds them with it, so a file one side removed from a group
/// is reported as removed, not missed.
#[test]
fn a_project_in_the_default_style_is_reviewed_with_its_base() {
    let repo = stopped_merge(|repo, side| {
        let children = match side {
            "base" => "\t\t\t\tA1 /* One.swift */,\n\t\t\t\tA2 /* Two.swift */,\n",
            "main" => "\t\t\t\tA1 /* One.swift */,\n\t\t\t\tA3 /* Three.swift */,\n",
            _ => {
                "\t\t\t\tA1 /* One.swift */,\n\t\t\t\tA2 /* Two.swift */,\n\t\t\t\tA4 /* Four.swift */,\n"
            }
        };
        let project = format!(
            "\t\tA1 /* One.swift */ = {{isa = PBXFileReference; }};\n\
             \t\tA2 /* Two.swift */ = {{isa = PBXFileReference; }};\n\
             \t\tA3 /* Three.swift */ = {{isa = PBXFileReference; }};\n\
             \t\tA4 /* Four.swift */ = {{isa = PBXFileReference; }};\n\
             \t\tchildren = (\n{children}\t\t\t);\n"
        );
        fs::write(repo.path().join("project.pbxproj"), project).unwrap();
        repo.ok(&["add", "."]);
    });
    let written = fs::read_to_string(repo.path().join("project.pbxproj")).unwrap();
    assert!(!written.contains("|||||||"), "{written}");
    let git = repo.cli();
    let mut overview = Overview::new(repo.path());
    overview.refresh(&git).unwrap();

    let review = overview.project(&git, "project.pbxproj").unwrap();
    assert_eq!(review.conflicts.len(), 1);
    let conflict = &review.conflicts[0];
    assert_eq!(conflict.ours_adds, ["Three.swift"]);
    assert_eq!(conflict.ours_removes, ["Two.swift"]);
    assert_eq!(conflict.theirs_adds, ["Four.swift"]);
    assert!(conflict.theirs_removes.is_empty());
    assert!(review.can_keep_both());
}

/// A file whose `conflict-marker-size` attribute makes git write longer
/// markers (it holds lines of `=======` itself) is read with that size:
/// listed with its conflict, unresolved, not taken for a file without
/// markers.
#[test]
fn the_marker_size_attribute_is_read_for_the_list() {
    let repo = stopped_merge(|repo, side| {
        fs::write(
            repo.path().join(".gitattributes"),
            "*.md conflict-marker-size=10\n",
        )
        .unwrap();
        fs::write(
            repo.path().join("notes.md"),
            format!("Title\n=======\n\n{side}\n"),
        )
        .unwrap();
        repo.ok(&["add", "."]);
    });
    let written = fs::read_to_string(repo.path().join("notes.md")).unwrap();
    assert!(written.contains("<<<<<<<<<< "), "{written}");
    let git = repo.cli();
    let mut overview = Overview::new(repo.path());
    let notes = file(&mut overview, &git, "notes.md");
    assert_eq!(notes.conflicts, Some(1));
    assert_eq!(notes.state, State::Unresolved);
}
