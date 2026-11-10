//! Loading a merge session from the files `git mergetool` passes, with the
//! real `git merge-file`.

use std::fs;
use std::path::Path;

use scry_core::git::GitCli;
use scry_core::resolution::CandidateKind;
use scry_core::session::{self, MergetoolFiles, Origin, SaveError};

fn files(dir: &Path, base: &str, local: &str, remote: &str, merged: &str) -> MergetoolFiles {
    let files = MergetoolFiles {
        base: dir.join("file_BASE.txt"),
        local: dir.join("file_LOCAL.txt"),
        remote: dir.join("file_REMOTE.txt"),
        merged: dir.join("file.txt"),
    };
    fs::write(&files.base, base).unwrap();
    fs::write(&files.local, local).unwrap();
    fs::write(&files.remote, remote).unwrap();
    fs::write(&files.merged, merged).unwrap();
    files
}

/// A file in the `merge` style lacks the base; it is rebuilt from git's three versions
/// with the human labels, so the conflict can be classified.
#[test]
fn markers_without_base_are_rebuilt_with_it() {
    let dir = tempfile::tempdir().unwrap();
    let merged = "<<<<<<< HEAD\nours\n=======\ntheirs\n>>>>>>> other\nend\n";
    let files = files(
        dir.path(),
        "base\nend\n",
        "ours\nend\n",
        "theirs\nend\n",
        merged,
    );

    let git = GitCli::new(dir.path().to_path_buf());
    let mut session = session::load(&git, &files, "main · rebase target", "Your commit").unwrap();
    assert!(session.rebuilt());
    assert_eq!(session.conflict_count(), 1);
    let conflict = session.conflict(0);
    assert_eq!(conflict.ours.label, "main · rebase target");
    assert_eq!(conflict.theirs.label, "Your commit");
    assert_eq!(conflict.base.as_ref().unwrap().text, "base\n");

    session.choose(0, CandidateKind::Theirs).unwrap();
    assert_eq!(session.merged_text().as_deref(), Some("theirs\nend\n"));
}

/// A file whose markers already show the base is kept as it is, so edits the user made
/// before launching the tool survive.
#[test]
fn markers_with_base_are_kept_as_they_are() {
    let dir = tempfile::tempdir().unwrap();
    // Edited by the user before launching: "end, edited".
    let merged =
        "<<<<<<< HEAD\nours\n||||||| base\nbase\n=======\ntheirs\n>>>>>>> other\nend, edited\n";
    let files = files(
        dir.path(),
        "base\nend\n",
        "ours\nend\n",
        "theirs\nend\n",
        merged,
    );

    let git = GitCli::new(dir.path().to_path_buf());
    let mut session = session::load(&git, &files, "ours", "theirs").unwrap();
    assert!(!session.rebuilt());
    session.choose(0, CandidateKind::Ours).unwrap();
    assert_eq!(
        session.merged_text().as_deref(),
        Some("ours\nend, edited\n")
    );
}

/// A file that is not UTF-8 is refused rather than corrupted by a lossy conversion.
#[test]
fn non_utf8_files_are_refused() {
    let dir = tempfile::tempdir().unwrap();
    let files = files(dir.path(), "a\n", "b\n", "c\n", "x\n");
    fs::write(&files.merged, [0xff, 0xfe, b'\n']).unwrap();
    let git = GitCli::new(dir.path().to_path_buf());
    let error = session::load(&git, &files, "ours", "theirs").err().unwrap();
    assert!(error.to_string().contains("not UTF-8"));
}

/// A `merge`-style file edited before launching the tool (one line fixed
/// outside the conflict) is used as it is, without the base: rebuilding
/// its conflicts from git's three versions would silently drop the edit.
#[test]
fn edited_markers_without_base_are_kept() {
    let dir = tempfile::tempdir().unwrap();
    let merged = "<<<<<<< HEAD\nours\n=======\ntheirs\n>>>>>>> other\nend, edited\n";
    let files = files(
        dir.path(),
        "base\nend\n",
        "ours\nend\n",
        "theirs\nend\n",
        merged,
    );

    let git = GitCli::new(dir.path().to_path_buf());
    let mut session = session::load(&git, &files, "ours", "theirs").unwrap();
    assert_eq!(session.origin(), Origin::EditedFile);
    assert!(session.conflict(0).base.is_none());
    session.choose(0, CandidateKind::Theirs).unwrap();
    assert_eq!(
        session.merged_text().as_deref(),
        Some("theirs\nend, edited\n")
    );
}

/// A file the user already resolved by hand (or rerere did) has no
/// conflict left; it is kept as it is instead of being rebuilt with the
/// conflicts back.
#[test]
fn file_without_conflicts_is_kept() {
    let dir = tempfile::tempdir().unwrap();
    let merged = "ours and theirs, by hand\nend\n";
    let files = files(
        dir.path(),
        "base\nend\n",
        "ours\nend\n",
        "theirs\nend\n",
        merged,
    );

    let git = GitCli::new(dir.path().to_path_buf());
    let session = session::load(&git, &files, "ours", "theirs").unwrap();
    assert_eq!(session.origin(), Origin::File);
    assert_eq!(session.conflict_count(), 0);
    assert_eq!(session.merged_text().as_deref(), Some(merged));
}

/// Markers git does not write, usually left by a partial edit, are
/// refused with a way out, rather than rebuilt over the user's work.
#[test]
fn malformed_markers_are_refused() {
    let dir = tempfile::tempdir().unwrap();
    let merged = "<<<<<<< HEAD\nours\nend\n";
    let files = files(dir.path(), "base\n", "ours\n", "theirs\n", merged);

    let git = GitCli::new(dir.path().to_path_buf());
    let error = session::load(&git, &files, "ours", "theirs").err().unwrap();
    assert!(error.to_string().contains("git checkout --merge"));
    assert_eq!(fs::read_to_string(&files.merged).unwrap(), merged);
}

/// Saving writes the result only over the file as it was read: a file
/// changed meanwhile (resolved in another editor, an aborted rebase) is
/// left alone unless the user says to overwrite it, and a deleted file is
/// not recreated behind their back.
#[test]
fn save_refuses_a_file_changed_on_disk() {
    let dir = tempfile::tempdir().unwrap();
    let merged = "<<<<<<< HEAD\nours\n||||||| base\nbase\n=======\ntheirs\n>>>>>>> other\n";
    let files = files(dir.path(), "base\n", "ours\n", "theirs\n", merged);
    let git = GitCli::new(dir.path().to_path_buf());
    let mut session = session::load(&git, &files, "ours", "theirs").unwrap();

    assert!(matches!(
        session.save(&files.merged, false),
        Err(SaveError::Unresolved(1))
    ));
    session.choose(0, CandidateKind::Theirs).unwrap();

    fs::write(&files.merged, "resolved elsewhere\n").unwrap();
    assert!(matches!(
        session.save(&files.merged, false),
        Err(SaveError::ChangedOnDisk(_))
    ));
    assert_eq!(
        fs::read_to_string(&files.merged).unwrap(),
        "resolved elsewhere\n"
    );

    fs::remove_file(&files.merged).unwrap();
    assert!(matches!(
        session.save(&files.merged, false),
        Err(SaveError::ChangedOnDisk(_))
    ));
    assert!(!files.merged.exists());

    session.save(&files.merged, true).unwrap();
    assert_eq!(fs::read_to_string(&files.merged).unwrap(), "theirs\n");
    // What Scry wrote is now the file as read: saving again is fine.
    session.save(&files.merged, false).unwrap();
}
