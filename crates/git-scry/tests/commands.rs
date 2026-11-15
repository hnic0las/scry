//! The CLI's read-only commands, run as the binary.

use std::fs;
use std::process::{Command, Output};

/// `git-scry` with git's configuration isolated from the user's.
fn git_scry(dir: &std::path::Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_git-scry"))
        .current_dir(dir)
        .args(args)
        .env("GIT_CONFIG_GLOBAL", dir.join("gitconfig"))
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .unwrap()
}

/// `conflicts` sums each conflict up with the size of each side, so the user sees what
/// each candidate weighs before choosing, and exits with 1 since conflicts are left.
#[test]
fn conflicts_lists_each_side_with_its_size() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("gitconfig"), "").unwrap();
    fs::write(
        dir.path().join("a.txt"),
        "<<<<<<< main\na\nb\n||||||| base\nx\n=======\nc\n>>>>>>> feature\n",
    )
    .unwrap();
    let output = git_scry(dir.path(), &["conflicts", "a.txt"]);
    assert_eq!(output.status.code(), Some(1));
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("a.txt: 1 conflict(s)"), "{stdout}");
    assert!(
        stdout.contains("ours main (2 lines) · theirs feature (1 line) · base (1 line)"),
        "{stdout}"
    );
}

/// `highlight` shows the core's colours in the terminal, or with `--spans` the role of
/// each span, to check highlighting without the app; a file without a grammar is refused.
#[test]
fn highlight_shows_colours_or_spans() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("gitconfig"), "").unwrap();
    fs::write(dir.path().join("a.swift"), "func upload() {}\n").unwrap();
    fs::write(dir.path().join("a.unknown"), "text\n").unwrap();

    let output = git_scry(dir.path(), &["highlight", "a.swift"]);
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.starts_with("\x1b[35mfunc\x1b[0m "), "{stdout:?}");

    let output = git_scry(dir.path(), &["highlight", "--spans", "a.swift"]);
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.starts_with("Keyword \"func\" · "), "{stdout:?}");

    let output = git_scry(dir.path(), &["highlight", "a.unknown"]);
    assert_eq!(output.status.code(), Some(2));
}
