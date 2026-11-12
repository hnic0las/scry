//! The operation overview on real repositories: a rebase stopped on two
//! conflicted files, resolved, staged and continued through its commands.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

use scry_core::git::GitCli;
use scry_core::overview::{
    Overview, OverviewCommand, OverviewError, OverviewEvent, Reword, Special, Version,
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
    assert_eq!(
        snapshot.reword,
        Some(Reword {
            message: String::from("add notes"),
            committed: true
        })
    );
    // Skipping would keep the commit made, so it is not offered.
    assert!(!snapshot.can_skip());
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
    // The plan shows the commit git made of the reword, not the original
    // line of the todo list.
    assert_eq!(plan.steps[1].subject, "Add the notes");
    assert_ne!(plan.steps[1].short_id.as_deref(), Some(&notes[..7]));
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
    assert_eq!(
        snapshot.reword,
        Some(Reword {
            message: String::from("feature change"),
            committed: false
        })
    );
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
    assert_eq!(
        deletion.changes,
        "@@ -1 +1 @@\n-old\n+old, changed on main\n"
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
