//! Finding what was done to a conflicted file resolved outside Scry (an
//! agent, another editor, a person), on real repositories: git keeps the
//! three versions in the index until the file is staged, so the conflicts
//! can be rebuilt and each one's resolution found in the file.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use scry_core::git::GitCli;
use scry_core::resolution::CandidateKind;
use scry_core::review::{self, Resolution};
use scry_core::session;
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
