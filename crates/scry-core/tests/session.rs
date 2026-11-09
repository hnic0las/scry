//! Loading a merge session from the files `git mergetool` passes, with the
//! real `git merge-file`.

use std::fs;
use std::path::Path;

use scry_core::git::GitCli;
use scry_core::resolution::CandidateKind;
use scry_core::session::{self, MergetoolFiles};

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
