//! Loading a merge session from the files `git mergetool` passes, with the
//! real `git merge-file`.

use std::fs;
use std::path::Path;

use scry_core::command::{Command, CommandError, Event};
use scry_core::git::{Git, GitCli};
use scry_core::resolution::CandidateKind;
use scry_core::session::{self, MergetoolFiles, Origin, SaveError};

fn choose(session: &mut session::MergeSession, candidate: CandidateKind) {
    session
        .apply(Command::Choose {
            conflict: 0,
            candidate,
        })
        .unwrap();
}

fn save(session: &mut session::MergeSession, overwrite: bool) -> Result<Vec<Event>, CommandError> {
    session.apply(Command::Save { overwrite })
}

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

    choose(&mut session, CandidateKind::Theirs);
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
    choose(&mut session, CandidateKind::Ours);
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
    choose(&mut session, CandidateKind::Theirs);
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

/// The way out names the file from the repository's root, the form git takes there,
/// rather than a long absolute path through a temporary folder.
#[test]
fn malformed_markers_advice_names_the_file_from_the_root() {
    let dir = tempfile::tempdir().unwrap();
    let git = GitCli::new(dir.path().to_path_buf());
    git.run(&["init", "--quiet"]).unwrap();
    let files = files(
        dir.path(),
        "base\n",
        "ours\n",
        "theirs\n",
        "<<<<<<< HEAD\nours\n",
    );

    let error = session::load(&git, &files, "ours", "theirs").err().unwrap();
    assert!(
        error
            .to_string()
            .contains("`git checkout --merge -- file.txt` at the repository's root"),
        "{error}"
    );
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
        save(&mut session, false),
        Err(CommandError::Save(SaveError::Unresolved(1)))
    ));
    choose(&mut session, CandidateKind::Theirs);

    fs::write(&files.merged, "resolved elsewhere\n").unwrap();
    assert!(matches!(
        save(&mut session, false),
        Err(CommandError::Save(SaveError::ChangedOnDisk(_)))
    ));
    assert_eq!(
        fs::read_to_string(&files.merged).unwrap(),
        "resolved elsewhere\n"
    );

    fs::remove_file(&files.merged).unwrap();
    assert!(matches!(
        save(&mut session, false),
        Err(CommandError::Save(SaveError::ChangedOnDisk(_)))
    ));
    assert!(!files.merged.exists());

    assert_eq!(save(&mut session, true).unwrap(), vec![Event::Saved]);
    assert_eq!(fs::read_to_string(&files.merged).unwrap(), "theirs\n");
    // What Scry wrote is now the file as read: saving again is fine.
    save(&mut session, false).unwrap();
}

/// A file whose `conflict-marker-size` attribute is set (it holds lines of
/// `=======` itself) is read with that size, and its conflicts are rebuilt
/// with it; read with the default size, it would have no conflict at all.
#[test]
fn marker_size_comes_from_the_attributes() {
    let dir = tempfile::tempdir().unwrap();
    let init = std::process::Command::new("git")
        .args(["init", "--quiet"])
        .current_dir(dir.path())
        .status()
        .unwrap();
    assert!(init.success());
    fs::write(
        dir.path().join(".gitattributes"),
        "*.txt conflict-marker-size=10\n",
    )
    .unwrap();
    let merged = "<<<<<<<<<< ours\nours\n==========\ntheirs\n>>>>>>>>>> theirs\nend\n";
    let files = files(
        dir.path(),
        "base\nend\n",
        "ours\nend\n",
        "theirs\nend\n",
        merged,
    );

    let git = GitCli::new(dir.path().to_path_buf());
    let session = session::load(&git, &files, "ours", "theirs").unwrap();
    assert_eq!(session.conflict_count(), 1);
    assert_eq!(session.origin(), Origin::Rebuilt);
    assert_eq!(session.conflict(0).base.as_ref().unwrap().text, "base\n");
}

/// git adds a line break to both sides of a conflict at the end of a file, so a file
/// without a final line break would gain one on every merge, a noisy diff on that line.
/// The side taken decides, read from git's own versions of the file.
#[test]
fn a_file_without_a_final_line_break_keeps_it() {
    let dir = tempfile::tempdir().unwrap();
    // As git writes it: a line break after each side.
    let merged = "a\n<<<<<<< ours\nours\n||||||| base\nb\n=======\ntheirs\n>>>>>>> theirs\n";
    let files = files(dir.path(), "a\nb", "a\nours", "a\ntheirs\n", merged);
    let git = GitCli::new(dir.path().to_path_buf());

    let mut session = session::load(&git, &files, "ours", "theirs").unwrap();
    choose(&mut session, CandidateKind::Ours);
    assert_eq!(session.merged_text().as_deref(), Some("a\nours"));
    choose(&mut session, CandidateKind::Theirs);
    assert_eq!(session.merged_text().as_deref(), Some("a\ntheirs\n"));
    choose(&mut session, CandidateKind::TheirsThenOurs);
    assert_eq!(session.merged_text().as_deref(), Some("a\ntheirs\nours"));
}

/// git does not escape a side's `=======` line (a Markdown heading underline), so the
/// markers it writes cannot be read back. While the file is as git wrote it, the
/// conflict is rebuilt with longer markers instead of refusing to open the file.
#[test]
fn a_conflict_with_a_separator_line_is_rebuilt_with_longer_markers() {
    let dir = tempfile::tempdir().unwrap();
    let base = "# Title\n";
    let local = "# Title\n\nSummary\n=======\nours\n";
    let remote = "# Title\n\nLicense\n=======\ntheirs\n";
    let files = files(dir.path(), base, local, remote, "");
    let git = GitCli::new(dir.path().to_path_buf());
    let written = git_merge_file(dir.path());
    fs::write(&files.merged, &written).unwrap();

    let mut session = session::load(&git, &files, "ours", "theirs").unwrap();
    assert!(session.rebuilt());
    assert_eq!(session.conflict_count(), 1);
    choose(&mut session, CandidateKind::OursThenTheirs);
    assert_eq!(
        session.merged_text().as_deref(),
        Some("# Title\n\nSummary\n=======\nours\nLicense\n=======\ntheirs\n")
    );

    // Edited since git wrote it: refused as before, never rebuilt over.
    fs::write(&files.merged, format!("{written}edited\n")).unwrap();
    assert!(session::load(&git, &files, "ours", "theirs").is_err());
}

/// The merged file as `git merge-file` writes it from the `files` helper's versions.
fn git_merge_file(dir: &Path) -> String {
    let output = std::process::Command::new("git")
        .current_dir(dir)
        .args([
            "merge-file",
            "-p",
            "file_LOCAL.txt",
            "file_BASE.txt",
            "file_REMOTE.txt",
        ])
        .output()
        .unwrap();
    String::from_utf8(output.stdout).unwrap()
}

/// A rebuilt session shows each conflict at its line in the file, not in the rebuilt text,
/// which has a base section the file lacks: the line leads to it in another editor.
#[test]
fn rebuilt_conflicts_start_where_the_file_has_them() {
    let dir = tempfile::tempdir().unwrap();
    let merged = "a\n<<<<<<< HEAD\nX\n=======\nZ\n>>>>>>> other\nb\nc\nd\nf\ng\nh\n\
                  <<<<<<< HEAD\nY\n=======\nW\n>>>>>>> other\ne\n";
    let files = files(
        dir.path(),
        "a\n1\nb\nc\nd\nf\ng\nh\n2\ne\n",
        "a\nX\nb\nc\nd\nf\ng\nh\nY\ne\n",
        "a\nZ\nb\nc\nd\nf\ng\nh\nW\ne\n",
        merged,
    );
    let git = GitCli::new(dir.path().to_path_buf());
    let session = session::load(&git, &files, "ours", "theirs").unwrap();
    assert!(session.rebuilt());
    let lines: Vec<usize> = (0..2)
        .map(|index| session.analysis(index).start_line)
        .collect();
    assert_eq!(lines, [2, 13]);
}
