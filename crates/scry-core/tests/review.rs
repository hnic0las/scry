//! Finding what was done to a conflicted file resolved outside Scry (an
//! agent, another editor, a person), on real repositories: git keeps the
//! three versions in the index until the file is staged, so the conflicts
//! can be rebuilt and each one's resolution found in the file.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use scry_core::command::Command as SessionCommand;
use scry_core::git::GitCli;
use scry_core::overview::Overview;
use scry_core::records::{Record, Store};
use scry_core::resolution::CandidateKind;
use scry_core::review::{self, Resolution};
use scry_core::session::{self, Choice, MergeSession, Origin};
use scry_core::theme::{Provenance, State};
use tempfile::TempDir;

/// A throwaway repository whose git ignores the user's configuration.
struct Repo {
    root: TempDir,
}

impl Repo {
    /// `main` and `feature` both change `file.txt` from `base`; merging
    /// `feature` into `main` stops on it, with `config` as the global
    /// configuration (a conflict style, for one).
    fn merged(base: &str, ours: &str, theirs: &str, config: &str) -> Repo {
        Repo::with_versions(Some(base), Some(ours), Some(theirs), config)
    }

    /// Like `merged`, with a version missing where `None`: a file deleted
    /// on one side, or added on both.
    fn with_versions(
        base: Option<&str>,
        ours: Option<&str>,
        theirs: Option<&str>,
        config: &str,
    ) -> Repo {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("gitconfig"), config).unwrap();
        fs::create_dir_all(root.path().join("repo")).unwrap();
        let repo = Repo { root };
        repo.ok(&["init", "--quiet", "--initial-branch=main"]);
        fs::write(repo.path().join("other.txt"), "other\n").unwrap();
        repo.commit(base, "base");
        repo.ok(&["switch", "--quiet", "-c", "feature"]);
        repo.commit(theirs, "theirs");
        repo.ok(&["switch", "--quiet", "main"]);
        repo.commit(ours, "ours");
        assert!(!repo.git(&["merge", "--quiet", "feature"]));
        repo
    }

    fn path(&self) -> PathBuf {
        self.root.path().join("repo")
    }

    fn file(&self) -> PathBuf {
        self.path().join("file.txt")
    }

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

    fn environment(root: &Path) -> Vec<(&'static str, String)> {
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

    fn commit(&self, text: Option<&str>, message: &str) {
        match text {
            Some(text) => fs::write(self.file(), text).unwrap(),
            None => {
                let _ = fs::remove_file(self.file());
            }
        }
        self.ok(&["add", "--all"]);
        self.ok(&["commit", "--quiet", "--allow-empty", "-m", message]);
    }

    /// The conflict ids Scry shows for the file as it is now.
    fn ids(&self) -> Vec<String> {
        let session = session::open_file(&GitCli::beside(&self.file()), &self.file()).unwrap();
        (0..session.conflict_count())
            .map(|index| session.analysis_so_far(index).id.clone())
            .collect()
    }

    /// The file opened for review, as the app will.
    fn open(&self) -> MergeSession {
        review::open(&self.cli(), &self.path(), "file.txt", "main", "feature")
            .unwrap()
            .expect("a review")
    }

    /// The file opened in the merge editor, as the app does.
    fn opened(&self) -> MergeSession {
        review::opened(
            &self.cli(),
            &self.cli(),
            &self.path(),
            "file.txt",
            ("main", "feature"),
        )
        .unwrap()
    }

    /// The file's row in the overview: its state and review.
    fn row(&self) -> (State, Option<review::Progress>) {
        let snapshot = Overview::new(self.path()).refresh(&self.cli()).unwrap();
        let file = snapshot
            .files
            .iter()
            .find(|file| file.path == "file.txt")
            .unwrap();
        (file.state, file.review)
    }

    fn find(&self) -> review::FileReview {
        review::find(&self.cli(), &self.path(), "file.txt")
            .unwrap()
            .expect("a text conflict")
    }
}

const BASE: &str = "start\none\nm1\nm2\nm3\nm4\nm5\ntwo\nend\n";
const OURS: &str = "start\none ours\nm1\nm2\nm3\nm4\nm5\ntwo ours\nend\n";
const THEIRS: &str = "start\none theirs\nm1\nm2\nm3\nm4\nm5\ntwo theirs\nend\n";

fn resolutions(review: &review::FileReview) -> Vec<Resolution> {
    review
        .conflicts
        .iter()
        .map(|conflict| conflict.resolution.clone())
        .collect()
}

/// A file resolved by hand elsewhere: each conflict's resolution is found
/// and named by the candidate it equals, or kept as the text written, so
/// the review can say "theirs" or "own text" rather than show a bare file.
/// The ids are the ones Scry showed while the markers were there, which the
/// agent used and the records name.
#[test]
fn each_resolution_is_found_and_named() {
    let repo = Repo::merged(BASE, OURS, THEIRS, "");
    let ids = repo.ids();
    assert_eq!(ids.len(), 2);
    fs::write(
        repo.file(),
        "start\none theirs\nm1\nm2\nm3\nm4\nm5\ntwo, mine\nend\n",
    )
    .unwrap();

    let review = repo.find();
    assert!(!review.changed_outside);
    assert_eq!(
        resolutions(&review),
        [
            Resolution::Candidate(CandidateKind::Theirs),
            Resolution::Text(String::from("two, mine\n")),
        ]
    );
    let found: Vec<&str> = review.conflicts.iter().map(|c| c.id.as_str()).collect();
    assert_eq!(found, ids);
    assert_eq!(review.conflicts[0].line, Some(2));
    assert_eq!(review.conflicts[1].line, Some(8));
}

/// Both sides kept, in either order, and a conflict removed: each is a
/// candidate the editor offers, so each is named as one. A removed conflict
/// has no line to point at.
#[test]
fn both_orders_and_neither_are_named() {
    let repo = Repo::merged(BASE, OURS, THEIRS, "");
    fs::write(
        repo.file(),
        "start\none theirs\none ours\nm1\nm2\nm3\nm4\nm5\nend\n",
    )
    .unwrap();
    let review = repo.find();
    assert_eq!(
        resolutions(&review),
        [
            Resolution::Candidate(CandidateKind::TheirsThenOurs),
            Resolution::Candidate(CandidateKind::Neither),
        ]
    );
    // A removed conflict has no line of its own.
    assert_eq!(review.conflicts[1].line, None);
}

/// A conflict left with its markers is open, with the id Scry shows for
/// it, while the one resolved before it is found: an agent that stopped
/// halfway leaves exactly that.
#[test]
fn a_conflict_left_open_is_found_open() {
    let repo = Repo::merged(BASE, OURS, THEIRS, "");
    let written = fs::read_to_string(repo.file()).unwrap();
    let rest = written.split_once("m5\n").unwrap().1;
    fs::write(
        repo.file(),
        format!("start\none ours\nm1\nm2\nm3\nm4\nm5\n{rest}"),
    )
    .unwrap();
    let open_ids = repo.ids();

    let review = repo.find();
    assert_eq!(
        resolutions(&review),
        [Resolution::Candidate(CandidateKind::Ours), Resolution::Open]
    );
    assert_eq!(open_ids, [review.conflicts[1].id.clone()]);
}

/// A change to the lines around the conflicts is what a review must catch:
/// the file says so, and the conflicts it cannot place are unknown rather
/// than guessed.
#[test]
fn a_change_outside_the_conflicts_is_flagged() {
    let repo = Repo::merged(BASE, OURS, THEIRS, "");
    fs::write(
        repo.file(),
        "start\none theirs\nm1\nm2, edited\nm3\nm4\nm5\ntwo theirs\nend\n",
    )
    .unwrap();
    let review = repo.find();
    assert!(review.changed_outside);
    assert!(
        review
            .conflicts
            .iter()
            .all(|conflict| conflict.resolution == Resolution::Unknown)
    );
}

/// With the `diff3` style the user set, the markers show the base and the
/// ids are computed with it: the rebuild follows the same style, so the ids
/// still match, and the base is a candidate.
#[test]
fn the_users_conflict_style_gives_the_same_ids() {
    let repo = Repo::merged(BASE, OURS, THEIRS, "[merge]\n\tconflictStyle = diff3\n");
    let ids = repo.ids();
    fs::write(
        repo.file(),
        "start\none\nm1\nm2\nm3\nm4\nm5\ntwo ours\nend\n",
    )
    .unwrap();

    let review = repo.find();
    assert_eq!(
        resolutions(&review),
        [
            Resolution::Candidate(CandidateKind::Base),
            Resolution::Candidate(CandidateKind::Ours),
        ]
    );
    let found: Vec<&str> = review.conflicts.iter().map(|c| c.id.as_str()).collect();
    assert_eq!(found, ids);
}

/// A file ending with a conflict whose sides have no final line break:
/// taking theirs leaves none either, as `git merge-file --theirs` does,
/// and it is still theirs.
#[test]
fn a_last_conflict_without_a_final_break_is_found() {
    let repo = Repo::merged("start\nend", "start\nend ours", "start\nend theirs", "");
    fs::write(repo.file(), "start\nend theirs").unwrap();
    assert_eq!(
        resolutions(&repo.find()),
        [Resolution::Candidate(CandidateKind::Theirs)]
    );
}

/// A resolution that repeats the lines after the conflict (closing braces)
/// is placed so those lines stay intact: cut at the first brace, the file
/// would read as changed outside the conflict. Candidates are recognised
/// in the same file.
#[test]
fn a_resolution_repeating_the_next_lines_is_placed_right() {
    let repo = Repo::merged(
        "func a() {\n}\n}\n",
        "func a() {\n  ours()\n}\n}\n",
        "func a() {\n  theirs()\n}\n}\n",
        "",
    );
    fs::write(repo.file(), "func a() {\n  x()\n}\n}\n}\n").unwrap();
    let review = repo.find();
    assert!(!review.changed_outside);
    assert_eq!(
        resolutions(&review),
        [Resolution::Text(String::from("  x()\n}\n"))]
    );

    fs::write(repo.file(), "func a() {\n  ours()\n  theirs()\n}\n}\n").unwrap();
    assert_eq!(
        resolutions(&repo.find()),
        [Resolution::Candidate(CandidateKind::OursThenTheirs)]
    );
}

/// A file missing from the folder (deleted while resolving) has nothing to
/// review here: the overview settles it as deleted.
#[test]
fn a_missing_file_has_no_review() {
    let repo = Repo::merged(BASE, OURS, THEIRS, "");
    assert!(
        review::find(&repo.cli(), &repo.path(), "missing.txt")
            .unwrap()
            .is_none()
    );
}

/// A file that could not be read is an error, not "nothing to review": the
/// overview must say why rather than show the file as if it were fine.
#[test]
fn an_unreadable_file_is_an_error() {
    let repo = Repo::merged(BASE, OURS, THEIRS, "");
    fs::create_dir(repo.path().join("folder")).unwrap();
    assert!(review::find(&repo.cli(), &repo.path(), "folder").is_err());
}

/// Deleted on one side: the index has no version for that side, so there
/// is no text conflict to rebuild; the overview's own view settles it.
#[test]
fn a_file_deleted_on_one_side_has_no_review() {
    let repo = Repo::with_versions(Some(BASE), Some(OURS), None, "");
    fs::write(repo.file(), OURS).unwrap();
    assert!(
        review::find(&repo.cli(), &repo.path(), "file.txt")
            .unwrap()
            .is_none()
    );
}

/// Added on both sides: no base, which git merges against an empty one, as
/// the review does, so both sides are still found.
#[test]
fn a_file_added_on_both_sides_is_reviewed() {
    let repo = Repo::with_versions(None, Some("ours\n"), Some("theirs\n"), "");
    fs::write(repo.file(), "theirs\n").unwrap();
    assert_eq!(
        resolutions(&repo.find()),
        [Resolution::Candidate(CandidateKind::Theirs)]
    );
}

/// In a file whose last conflict has no final line break, an earlier
/// conflict's own text is exactly what it holds, not the rest of the file.
#[test]
fn own_text_before_a_last_conflict_without_a_break_is_its_own() {
    let repo = Repo::merged(
        "one\nm1\nm2\nm3\nm4\nend",
        "one ours\nm1\nm2\nm3\nm4\nend ours",
        "one theirs\nm1\nm2\nm3\nm4\nend theirs",
        "",
    );
    fs::write(repo.file(), "mine\nm1\nm2\nm3\nm4\nend theirs").unwrap();
    assert_eq!(
        resolutions(&repo.find()),
        [
            Resolution::Text(String::from("mine\n")),
            Resolution::Candidate(CandidateKind::Theirs),
        ]
    );
}

/// The store follows the operation's stop: what was kept during the merge
/// is found again while it is in progress, kept in the git folder (nothing
/// in `git status`), and no longer seen once the merge is committed.
#[test]
fn the_store_belongs_to_the_stop() {
    let repo = Repo::merged(BASE, OURS, THEIRS, "");
    let record = Record {
        conflict: String::from("c1"),
        by: String::from("agent"),
        pick: None,
        text: String::from("mine\n"),
        reason: String::from("Both sides renamed the same value."),
        dropped: Some(String::from("nothing")),
        at: 0,
    };
    Store::open(&repo.cli())
        .unwrap()
        .add_record("file.txt", record.clone())
        .unwrap();
    assert_eq!(
        Store::open(&repo.cli()).unwrap().records("file.txt")["c1"],
        record
    );
    let status = Command::new("git")
        .arg("-C")
        .arg(repo.path())
        .args(["status", "--porcelain", "--untracked-files=all"])
        .envs(Repo::environment(repo.root.path()))
        .output()
        .unwrap();
    assert_eq!(String::from_utf8(status.stdout).unwrap(), "UU file.txt\n");

    repo.ok(&["add", "file.txt"]);
    repo.ok(&["commit", "--quiet", "--no-edit"]);
    assert!(
        Store::open(&repo.cli())
            .unwrap()
            .records("file.txt")
            .is_empty()
    );
}

fn outside(session: &MergeSession, index: usize) -> &scry_core::session::Outside {
    match session.choice(index) {
        Some(Choice::Outside(outside)) => outside,
        other => panic!("conflict {index}: {other:?}"),
    }
}

/// A file resolved elsewhere opens for review: each resolved conflict's
/// choice is what the file holds, not reviewed yet, and shown as needing
/// review; an open conflict stays open; and the merged text is the file
/// itself, so saving without a change writes it back unchanged.
#[test]
fn a_resolved_file_opens_for_review() {
    let repo = Repo::merged(BASE, OURS, THEIRS, "");
    let file = "start\none theirs\nm1\nm2\nm3\nm4\nm5\ntwo, mine\nend\n";
    fs::write(repo.file(), file).unwrap();
    let session = repo.open();
    assert_eq!(outside(&session, 0).matches, Some(CandidateKind::Theirs));
    assert_eq!(outside(&session, 1).matches, None);
    assert_eq!(outside(&session, 1).text, "two, mine\n");
    assert_eq!(session.status().unreviewed, 2);
    assert_eq!(session.merged_text().as_deref(), Some(file));
    assert_eq!(session.conflict(1).start_line, 8);

    let view = session.conflict_view(0, "main", "feature");
    assert_eq!(view.state, State::NeedsReview);
    let chosen = view.chosen.unwrap();
    assert_eq!(chosen.title, "Outside Scry · Theirs");
    assert_eq!(chosen.provenance, Provenance::Human);
    assert_eq!(view.outside.unwrap().by, None);
}

/// Marking a conflict reviewed is kept in the git folder: opening the file
/// again finds it reviewed, while the other still needs review. Once the
/// file holds something else for it, it needs review again.
#[test]
fn a_review_survives_opening_the_file_again() {
    let repo = Repo::merged(BASE, OURS, THEIRS, "");
    fs::write(
        repo.file(),
        "start\none theirs\nm1\nm2\nm3\nm4\nm5\ntwo ours\nend\n",
    )
    .unwrap();
    let mut session = repo.open();
    assert!(
        !session
            .apply(SessionCommand::MarkReviewed { conflict: 0 })
            .unwrap()
            .is_empty()
    );
    assert_eq!(session.status().unreviewed, 1);
    // Twice changes nothing.
    assert!(
        session
            .apply(SessionCommand::MarkReviewed { conflict: 0 })
            .unwrap()
            .is_empty()
    );

    let session = repo.open();
    assert!(outside(&session, 0).reviewed);
    assert!(!outside(&session, 1).reviewed);
    assert_eq!(
        session.conflict_view(0, "main", "feature").state,
        State::Resolved
    );

    fs::write(
        repo.file(),
        "start\none ours\nm1\nm2\nm3\nm4\nm5\ntwo ours\nend\n",
    )
    .unwrap();
    assert!(!outside(&repo.open(), 0).reviewed);
}

/// A conflict the user resolves in Scry during the review is theirs: once
/// saved, opening the file again gives it back as their choice, not to
/// review.
#[test]
fn a_choice_made_in_scry_counts_as_reviewed() {
    let repo = Repo::merged(BASE, OURS, THEIRS, "");
    fs::write(
        repo.file(),
        "start\none theirs\nm1\nm2\nm3\nm4\nm5\ntwo theirs\nend\n",
    )
    .unwrap();
    let mut session = repo.open();
    session
        .apply(SessionCommand::Choose {
            conflict: 1,
            candidate: CandidateKind::Ours,
        })
        .unwrap();
    session
        .apply(SessionCommand::Save { overwrite: false })
        .unwrap();
    assert_eq!(
        fs::read_to_string(repo.file()).unwrap(),
        "start\none theirs\nm1\nm2\nm3\nm4\nm5\ntwo ours\nend\n"
    );
    let session = repo.open();
    assert_eq!(
        session.choice(1),
        Some(&Choice::Candidate(CandidateKind::Ours))
    );
    assert!(
        session
            .reviewed()
            .contains_key(&session.analysis_so_far(1).id)
    );
    assert_eq!(session.status().unreviewed, 1);
}

/// What the agent recorded shows with its resolution, as long as the file
/// still holds the text it wrote; changed since, the record is not shown,
/// since it would explain text that is no longer there.
#[test]
fn an_agents_record_shows_while_the_file_holds_its_text() {
    let repo = Repo::merged(BASE, OURS, THEIRS, "");
    let ids = repo.ids();
    fs::write(
        repo.file(),
        "start\none theirs\nm1\nm2\nm3\nm4\nm5\ntwo ours\nend\n",
    )
    .unwrap();
    let record = |conflict: &str, text: &str| Record {
        conflict: conflict.to_string(),
        by: String::from("claude-code 2.1"),
        pick: Some(CandidateKind::Theirs),
        text: text.to_string(),
        reason: String::from("The feature's wording is the newer one."),
        dropped: None,
        at: 0,
    };
    let store = Store::open(&repo.cli()).unwrap();
    store
        .add_record("file.txt", record(&ids[0], "one theirs\n"))
        .unwrap();
    store
        .add_record("file.txt", record(&ids[1], "two theirs\n"))
        .unwrap();

    let session = repo.open();
    let view = session.conflict_view(0, "main", "feature");
    assert_eq!(view.chosen.as_ref().unwrap().title, "Agent · Theirs");
    assert_eq!(view.chosen.unwrap().provenance, Provenance::Agent);
    let told = view.outside.unwrap();
    assert_eq!(told.by.as_deref(), Some("claude-code 2.1"));
    assert_eq!(
        told.reason.as_deref(),
        Some("The feature's wording is the newer one.")
    );
    assert_eq!(outside(&session, 1).record, None);
}

/// Reopening a conflict takes its resolution away, as one step undo puts
/// back.
#[test]
fn reopening_a_conflict_can_be_undone() {
    let repo = Repo::merged(BASE, OURS, THEIRS, "");
    fs::write(
        repo.file(),
        "start\none theirs\nm1\nm2\nm3\nm4\nm5\ntwo ours\nend\n",
    )
    .unwrap();
    let mut session = repo.open();
    session
        .apply(SessionCommand::Reopen { conflict: 0 })
        .unwrap();
    assert_eq!(session.choice(0), None);
    assert_eq!(session.merged_text(), None);
    session.apply(SessionCommand::Undo).unwrap();
    assert_eq!(outside(&session, 0).matches, Some(CandidateKind::Theirs));
}

/// A file changed outside its conflicts has no conflict by conflict
/// review: it would hide the change.
#[test]
fn a_file_changed_outside_does_not_open_for_review() {
    let repo = Repo::merged(BASE, OURS, THEIRS, "");
    fs::write(
        repo.file(),
        "start\none theirs\nm1\nm2, edited\nm3\nm4\nm5\ntwo theirs\nend\n",
    )
    .unwrap();
    assert!(
        review::open(&repo.cli(), &repo.path(), "file.txt", "main", "feature")
            .unwrap()
            .is_none()
    );
}

/// In a file ending with a conflict whose sides have no final line break,
/// a side picked in Scry during the review keeps that: the file is written
/// as `git merge-file --ours` would, not with a line break git added.
#[test]
fn a_side_picked_in_review_keeps_its_final_break() {
    let repo = Repo::merged("start\nend", "start\nend ours", "start\nend theirs", "");
    fs::write(repo.file(), "start\nend theirs").unwrap();
    let mut session = repo.open();
    assert_eq!(session.merged_text().as_deref(), Some("start\nend theirs"));
    session
        .apply(SessionCommand::Choose {
            conflict: 0,
            candidate: CandidateKind::Ours,
        })
        .unwrap();
    assert_eq!(session.merged_text().as_deref(), Some("start\nend ours"));
}

fn progress(reviewed: usize, total: usize, changed_outside: bool) -> Option<review::Progress> {
    Some(review::Progress {
        reviewed,
        total,
        changed_outside,
    })
}

/// A file just stopped on opens from its markers in the editor, and what
/// the user resolves there counts as reviewed: once saved, the overview
/// has it resolved, not to review, and opening it again is not a review.
#[test]
fn a_file_resolved_in_scry_needs_no_review() {
    let repo = Repo::merged(BASE, OURS, THEIRS, "");
    let mut session = repo.opened();
    assert_eq!(session.origin(), Origin::File);
    for conflict in 0..2 {
        session
            .apply(SessionCommand::Choose {
                conflict,
                candidate: CandidateKind::Theirs,
            })
            .unwrap();
    }
    session
        .apply(SessionCommand::Save { overwrite: false })
        .unwrap();
    assert_eq!(repo.row(), (State::Resolved, progress(2, 2, false)));
    assert_eq!(repo.opened().unreviewed(), 0);
}

/// Opened again, a file resolved in Scry shows what the user chose there,
/// a candidate or text written by hand, not a resolution from outside; a
/// conflict changed outside since is to review, as from outside.
#[test]
fn a_file_resolved_in_scry_opens_again_with_its_choices() {
    let repo = Repo::merged(BASE, OURS, THEIRS, "");
    let mut session = repo.opened();
    let commands = [
        SessionCommand::Choose {
            conflict: 0,
            candidate: CandidateKind::Theirs,
        },
        SessionCommand::Edit {
            conflict: 1,
            text: String::from("two by hand\n"),
        },
        SessionCommand::Save { overwrite: false },
    ];
    for command in commands {
        session.apply(command).unwrap();
    }

    let again = repo.opened();
    assert_eq!(
        again.choice(0),
        Some(&Choice::Candidate(CandidateKind::Theirs))
    );
    assert!(matches!(again.choice(1), Some(Choice::Edited(edit)) if edit.text == "two by hand\n"));
    assert!((0..2).all(|index| {
        again
            .conflict_view(index, "main", "feature")
            .outside
            .is_none()
    }));
    assert!(!again.status().unwritten);

    let text = fs::read_to_string(repo.file()).unwrap();
    fs::write(repo.file(), text.replace("two by hand", "two changed")).unwrap();
    let changed = repo.opened();
    assert_eq!(
        changed.choice(0),
        Some(&Choice::Candidate(CandidateKind::Theirs))
    );
    assert!(matches!(changed.choice(1), Some(Choice::Outside(outside)) if !outside.reviewed));
}

/// A file resolved elsewhere needs review in the overview, with how many
/// conflicts are reviewed, until every one is: the overview reads the
/// review the editor keeps.
#[test]
fn the_overview_counts_reviewed_conflicts() {
    let repo = Repo::merged(BASE, OURS, THEIRS, "");
    fs::write(
        repo.file(),
        "start\none theirs\nm1\nm2\nm3\nm4\nm5\ntwo, mine\nend\n",
    )
    .unwrap();
    assert_eq!(repo.row(), (State::NeedsReview, progress(0, 2, false)));
    let mut session = repo.opened();
    assert_eq!(session.origin(), Origin::Resolved);
    session
        .apply(SessionCommand::MarkReviewed { conflict: 1 })
        .unwrap();
    assert_eq!(repo.row(), (State::NeedsReview, progress(1, 2, false)));
    session
        .apply(SessionCommand::MarkReviewed { conflict: 0 })
        .unwrap();
    assert_eq!(repo.row(), (State::Resolved, progress(2, 2, false)));
}

/// A file changed around its conflicts needs review as a whole: which
/// conflict holds what cannot be told, so none counts as reviewed.
#[test]
fn a_file_changed_outside_needs_review() {
    let repo = Repo::merged(BASE, OURS, THEIRS, "");
    fs::write(
        repo.file(),
        "start\none theirs\nm1\nm2, edited\nm3\nm4\nm5\ntwo theirs\nend\n",
    )
    .unwrap();
    assert_eq!(repo.row(), (State::NeedsReview, progress(0, 2, true)));
}

/// A file an agent resolved halfway opens as a review in the editor: the
/// resolved conflict with what it holds, the other open, so neither is
/// lost by opening from the markers left.
#[test]
fn a_file_resolved_halfway_opens_as_a_review() {
    let repo = Repo::merged(BASE, OURS, THEIRS, "");
    let open = fs::read_to_string(repo.file()).unwrap();
    let second = &open[open.find("m5\n").unwrap() + 3..];
    fs::write(
        repo.file(),
        format!("start\none theirs\nm1\nm2\nm3\nm4\nm5\n{second}"),
    )
    .unwrap();
    let session = repo.opened();
    assert_eq!(session.origin(), Origin::Resolved);
    assert_eq!(outside(&session, 0).matches, Some(CandidateKind::Theirs));
    assert!(session.choice(1).is_none());
}

/// Markers written in another style than git's now (`git checkout
/// --conflict=diff3` with no style configured) are not mistaken for text
/// of the user's: the file opens from its markers, with its base.
#[test]
fn markers_in_another_style_open_from_the_markers() {
    let repo = Repo::merged(BASE, OURS, THEIRS, "");
    repo.ok(&["checkout", "--conflict=diff3", "--", "file.txt"]);
    let session = repo.opened();
    assert_eq!(session.origin(), Origin::File);
    assert_eq!(session.conflict_count(), 2);
    assert!(session.unresolved() == 2);
}

/// Something is left to write only when the result is not what the file
/// holds: marking a resolution reviewed leaves the file as it is (the
/// review is kept already), choosing another candidate does not, until it
/// is saved; a file with a conflict open always has something left.
#[test]
fn only_a_changed_result_is_left_to_write() {
    let repo = Repo::merged(BASE, OURS, THEIRS, "");
    assert!(repo.opened().status().unwritten);
    fs::write(
        repo.file(),
        "start\none theirs\nm1\nm2\nm3\nm4\nm5\ntwo theirs\nend\n",
    )
    .unwrap();
    let mut session = repo.opened();
    assert!(!session.status().unwritten);
    session
        .apply(SessionCommand::MarkReviewed { conflict: 0 })
        .unwrap();
    assert!(!session.status().unwritten);
    session
        .apply(SessionCommand::Choose {
            conflict: 1,
            candidate: CandidateKind::Ours,
        })
        .unwrap();
    assert!(session.status().unwritten);
    session
        .apply(SessionCommand::Save { overwrite: false })
        .unwrap();
    assert!(!session.status().unwritten);
}

// Writing a conflict's resolution, as the MCP server's `resolve` does.

impl Repo {
    fn write(
        &self,
        id: &str,
        fill: &review::Fill,
        replace: bool,
    ) -> Result<String, review::WriteError> {
        review::write(&self.cli(), &self.path(), "file.txt", id, fill, replace)
    }

    fn store(&self) -> Store {
        Store::open(&self.cli()).unwrap()
    }
}

fn own(text: &str) -> review::Fill {
    review::Fill::Text(String::from(text))
}

fn record(id: &str, text: &str) -> Record {
    Record {
        conflict: id.to_string(),
        by: String::from("claude-code 2.1"),
        pick: None,
        text: text.to_string(),
        reason: String::from("A reason long enough to be kept."),
        dropped: Some(String::from("nothing")),
        at: 0,
    }
}

/// A resolution written by hand is not the agent's to replace: `write`
/// refuses it, saying it was made outside Scry, and leaves the file as it
/// is; with `replace` it is replaced.
#[test]
fn write_keeps_a_resolution_made_by_hand() {
    let repo = Repo::merged(BASE, OURS, THEIRS, "");
    let ids = repo.ids();
    let by_hand = "start\none, mine, carefully written\nm1\nm2\nm3\nm4\nm5\n<<<<<<< ours\ntwo ours\n=======\ntwo theirs\n>>>>>>> theirs\nend\n";
    fs::write(repo.file(), by_hand).unwrap();
    assert!(matches!(
        repo.write(&ids[0], &own("one, agent\n"), false),
        Err(review::WriteError::Resolved(review::Resolver::Outside))
    ));
    assert_eq!(fs::read_to_string(repo.file()).unwrap(), by_hand);
    // The open conflict is the agent's to write.
    repo.write(&ids[1], &own("two, agent\n"), false).unwrap();
    repo.write(&ids[0], &own("one, agent\n"), true).unwrap();
    assert_eq!(
        fs::read_to_string(repo.file()).unwrap(),
        "start\none, agent\nm1\nm2\nm3\nm4\nm5\ntwo, agent\nend\n"
    );
}

/// An agent may write its own resolution again while the file holds what
/// it recorded; once someone changed it, or the user reviewed it, it is
/// refused and says who.
#[test]
fn write_replaces_only_the_agents_own_untouched_resolution() {
    let repo = Repo::merged(BASE, OURS, THEIRS, "");
    let ids = repo.ids();
    let written = repo.write(&ids[0], &own("one, agent"), false).unwrap();
    assert_eq!(written, "one, agent\n");
    repo.store()
        .add_record("file.txt", record(&ids[0], &written))
        .unwrap();
    let again = repo
        .write(&ids[0], &own("one, agent again\n"), false)
        .unwrap();
    repo.store()
        .add_record("file.txt", record(&ids[0], &again))
        .unwrap();

    let changed = fs::read_to_string(repo.file())
        .unwrap()
        .replace("one, agent again", "one, edited");
    fs::write(repo.file(), &changed).unwrap();
    match repo.write(&ids[0], &own("one, third\n"), false) {
        Err(review::WriteError::Resolved(review::Resolver::ChangedSince(by))) => {
            assert_eq!(by, "claude-code 2.1")
        }
        other => panic!("{other:?}"),
    }

    let reviewed = [(ids[0].clone(), String::from("one, edited\n"))]
        .into_iter()
        .collect();
    repo.store().set_reviewed("file.txt", &reviewed).unwrap();
    repo.store()
        .add_record("file.txt", record(&ids[0], "one, edited\n"))
        .unwrap();
    assert!(matches!(
        repo.write(&ids[0], &own("one, third\n"), false),
        Err(review::WriteError::Resolved(review::Resolver::Reviewed))
    ));
    assert_eq!(fs::read_to_string(repo.file()).unwrap(), changed);
}

/// A pick on the last conflict of a file without a final line break ends
/// the file as that side does: without one, so the file is that candidate.
#[test]
fn a_pick_on_the_last_conflict_keeps_the_side_s_missing_break() {
    let repo = Repo::merged("head\nbase", "head\nours", "head\ntheirs", "");
    let ids = repo.ids();
    let theirs = review::Fill::Pick {
        kind: CandidateKind::Theirs,
        text: String::from("theirs\n"),
    };
    assert_eq!(repo.write(&ids[0], &theirs, false).unwrap(), "theirs\n");
    assert_eq!(fs::read_to_string(repo.file()).unwrap(), "head\ntheirs");
    assert_eq!(
        resolutions(&repo.find()),
        [Resolution::Candidate(CandidateKind::Theirs)]
    );
}

/// The file ends as the side the pick ends with: theirs without a final
/// break, ours with one; removing the last conflict keeps the break of the
/// line before it.
#[test]
fn a_pick_on_the_last_conflict_ends_as_its_side() {
    let theirs_unbroken = ("head\nours\n", "head\ntheirs");
    let ours_unbroken = ("head\nours", "head\ntheirs\n");
    for ((ours, theirs), kind, text, expected) in [
        (
            theirs_unbroken,
            CandidateKind::Theirs,
            "theirs\n",
            "head\ntheirs",
        ),
        (
            theirs_unbroken,
            CandidateKind::Ours,
            "ours\n",
            "head\nours\n",
        ),
        (theirs_unbroken, CandidateKind::Neither, "", "head\n"),
        (ours_unbroken, CandidateKind::Neither, "", "head\n"),
    ] {
        let repo = Repo::merged("head\nbase\n", ours, theirs, "");
        let ids = repo.ids();
        let fill = review::Fill::Pick {
            kind,
            text: String::from(text),
        };
        repo.write(&ids[0], &fill, false).unwrap();
        assert_eq!(
            fs::read_to_string(repo.file()).unwrap(),
            expected,
            "{kind:?}"
        );
    }
}

/// A pick is written byte for byte, whatever line breaks the rest of the
/// file uses, so it still reads as that candidate; own text takes the
/// file's.
#[test]
fn a_pick_keeps_its_own_line_breaks() {
    let repo = Repo::merged(
        "h1\nh2\nh3\nbase\nt1\nt2\n",
        "h1\nh2\nh3\nours\nt1\nt2\n",
        "h1\nh2\nh3\ntheirs\r\nt1\nt2\n",
        "",
    );
    let ids = repo.ids();
    let theirs = review::Fill::Pick {
        kind: CandidateKind::Theirs,
        text: String::from("theirs\r\n"),
    };
    repo.write(&ids[0], &theirs, false).unwrap();
    assert_eq!(
        fs::read_to_string(repo.file()).unwrap(),
        "h1\nh2\nh3\ntheirs\r\nt1\nt2\n"
    );
    assert_eq!(
        resolutions(&repo.find()),
        [Resolution::Candidate(CandidateKind::Theirs)]
    );
}

/// What cannot be written is refused with nothing written: a stale id (with
/// the ids now), text holding markers, a conflict resolved in a file changed
/// outside its conflicts. One still open there is written between its own
/// markers.
#[test]
fn write_refuses_what_it_cannot_place() {
    let repo = Repo::merged(BASE, OURS, THEIRS, "");
    let ids = repo.ids();
    match repo.write("c0000000", &own("x\n"), false) {
        Err(review::WriteError::NoSuchConflict(now)) => assert_eq!(now, ids),
        other => panic!("{other:?}"),
    }
    assert!(matches!(
        repo.write(
            &ids[0],
            &own("<<<<<<< a\nx\n=======\ny\n>>>>>>> b\n"),
            false
        ),
        Err(review::WriteError::Markers)
    ));
    repo.write(&ids[0], &own("one, agent\n"), false).unwrap();
    let changed = fs::read_to_string(repo.file())
        .unwrap()
        .replace("start\n", "begin\n");
    fs::write(repo.file(), &changed).unwrap();
    assert!(matches!(
        repo.write(&ids[0], &own("x\n"), true),
        Err(review::WriteError::ChangedOutside)
    ));
    assert_eq!(fs::read_to_string(repo.file()).unwrap(), changed);
    repo.write(&ids[1], &own("two, agent\n"), false).unwrap();
    assert_eq!(
        fs::read_to_string(repo.file()).unwrap(),
        "begin\none, agent\nm1\nm2\nm3\nm4\nm5\ntwo, agent\nend\n"
    );
}

/// Own text repeating the lines between two conflicts is placed where its
/// record says it ends, not cut at the shortest region: both resolutions
/// read with their records, and writing the first again replaces all of
/// it, leaving no stray lines (review P1#2).
#[test]
fn own_text_holding_the_lines_between_conflicts_is_placed_by_its_record() {
    let repo = Repo::merged(
        "A\nx1\ns\ns\ns\ns\ny1\nB\n",
        "A\nx2\ns\ns\ns\ns\ny2\nB\n",
        "A\nx3\ns\ns\ns\ns\ny3\nB\n",
        "",
    );
    let ids = repo.ids();
    assert_eq!(ids.len(), 2);
    let first = repo
        .write(&ids[0], &own("x4\ns\ns\ns\ns\nx5\n"), false)
        .unwrap();
    repo.store()
        .add_record("file.txt", record(&ids[0], &first))
        .unwrap();
    let theirs = review::Fill::Pick {
        kind: CandidateKind::Theirs,
        text: String::from("y3\n"),
    };
    let second = repo.write(&ids[1], &theirs, false).unwrap();
    repo.store()
        .add_record("file.txt", record(&ids[1], &second))
        .unwrap();
    assert_eq!(
        resolutions(&repo.find()),
        [
            Resolution::Text(String::from("x4\ns\ns\ns\ns\nx5\n")),
            Resolution::Candidate(CandidateKind::Theirs),
        ]
    );

    repo.write(&ids[0], &own("x6\n"), false).unwrap();
    assert_eq!(
        fs::read_to_string(repo.file()).unwrap(),
        "A\nx6\ns\ns\ns\ns\ny3\nB\n"
    );
}

/// Markers in another conflict style than git's now (`git checkout
/// --conflict=diff3` with the default style): the ids are the ones the
/// file's markers give, as the editor and the MCP server show them, and
/// writing one of them replaces those markers; an id from neither is
/// refused with the file's ids (review P2#2).
#[test]
fn write_resolves_markers_in_another_style() {
    let repo = Repo::merged("A\nb1\nb2\nB\n", "A\nk\no2\nB\n", "A\nk\nt2\nB\n", "");
    repo.ok(&["checkout", "--conflict=diff3", "--", "file.txt"]);
    assert!(fs::read_to_string(repo.file()).unwrap().contains("|||||||"));
    let ids = repo.ids();
    assert_eq!(ids.len(), 1);
    match repo.write("c0000000", &own("x\n"), false) {
        Err(review::WriteError::NoSuchConflict(now)) => assert_eq!(now, ids),
        other => panic!("{other:?}"),
    }
    repo.write(&ids[0], &own("k\nboth\n"), false).unwrap();
    assert_eq!(fs::read_to_string(repo.file()).unwrap(), "A\nk\nboth\nB\n");
}

/// A stale id in a file with no markers left is refused with git's ids: the
/// file's own markers are a fallback only where it still has some.
#[test]
fn a_stale_id_in_a_resolved_file_names_git_s_ids() {
    let repo = Repo::merged(BASE, OURS, THEIRS, "");
    let ids = repo.ids();
    fs::write(repo.file(), THEIRS).unwrap();
    match repo.write("c0000000", &own("x\n"), false) {
        Err(review::WriteError::NoSuchConflict(now)) => assert_eq!(now, ids),
        other => panic!("{other:?}"),
    }
}
