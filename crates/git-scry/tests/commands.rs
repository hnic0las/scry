//! The CLI's read-only commands, run as the binary.

use std::fs;
use std::process::{Command, Output, Stdio};

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

/// Piped into a reader that stops early (`git scry highlight … | head -1`), git-scry
/// stops quietly like other command-line tools, instead of printing a panic about a
/// broken pipe, and with 141 as a tool killed by `SIGPIPE`: not a success, since it did
/// not finish.
#[test]
fn a_closed_pipe_stops_quietly() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("gitconfig"), "").unwrap();
    // More than a pipe holds, so that git-scry still writes after the reader is gone.
    fs::write(
        dir.path().join("a.swift"),
        "func upload() {}\n".repeat(20_000),
    )
    .unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_git-scry"))
        .current_dir(dir.path())
        .args(["highlight", "a.swift"])
        .env("GIT_CONFIG_GLOBAL", dir.path().join("gitconfig"))
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    drop(child.stdout.take());
    let output = child.wait_with_output().unwrap();
    assert_eq!(output.status.code(), Some(141));
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
}

/// With its error output closed (`git mergetool 2>&1 | head` once `head` is gone), a
/// `resolve` that fails still exits with a failure: git trusts the merge tool's exit
/// code, and a success would stage a file that is still conflicted.
#[test]
fn a_failure_with_its_error_output_closed_is_not_a_success() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("gitconfig"), "").unwrap();
    for (name, content) in [("base", "x\n"), ("local", "a\n"), ("remote", "b\n")] {
        fs::write(dir.path().join(name), content).unwrap();
    }
    let conflicted = "<<<<<<< ours\na\n=======\nb\n>>>>>>> theirs\n";
    fs::write(dir.path().join("merged"), conflicted).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_git-scry"))
        .current_dir(dir.path())
        .args(["resolve", "--take", "1=nonsense", "--"])
        .args(["base", "local", "remote", "merged"])
        .env("GIT_CONFIG_GLOBAL", dir.path().join("gitconfig"))
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    drop(child.stderr.take());
    let status = child.wait().unwrap();
    assert!(!status.success(), "{status:?}");
    assert_eq!(
        fs::read_to_string(dir.path().join("merged")).unwrap(),
        conflicted
    );
}

/// `session` says which operation is in progress, who is on each side, which files
/// still have conflicts, and what was done to each conflict of a file resolved
/// elsewhere and not staged: what the user needs before choosing a side, and what a
/// review starts from.
#[test]
fn session_names_the_operation_its_sides_and_files() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("gitconfig"), "").unwrap();
    let git = |args: &[&str]| {
        Command::new("git")
            .current_dir(dir.path())
            .args(args)
            .env("GIT_CONFIG_GLOBAL", dir.path().join("gitconfig"))
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_AUTHOR_NAME", "Scry Test")
            .env("GIT_AUTHOR_EMAIL", "test@example.com")
            .env("GIT_COMMITTER_NAME", "Scry Test")
            .env("GIT_COMMITTER_EMAIL", "test@example.com")
            .output()
            .unwrap()
            .status
            .success()
    };
    assert!(git(&["init", "--quiet", "--initial-branch=main"]));
    let output = git_scry(dir.path(), &["session"]);
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "No merge, rebase, cherry-pick or revert in progress.\n"
    );

    fs::write(dir.path().join("a.txt"), "base\n").unwrap();
    assert!(git(&["add", "a.txt"]));
    assert!(git(&["commit", "--quiet", "-m", "base"]));
    assert!(git(&["switch", "--quiet", "-c", "feature"]));
    fs::write(dir.path().join("a.txt"), "feature\n").unwrap();
    assert!(git(&["commit", "--quiet", "-am", "feature change"]));
    assert!(git(&["switch", "--quiet", "main"]));
    fs::write(dir.path().join("a.txt"), "main\n").unwrap();
    assert!(git(&["commit", "--quiet", "-am", "main change"]));
    assert!(!git(&["merge", "feature"]));

    let output = git_scry(dir.path(), &["session"]);
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        stdout.starts_with("Merging feature into main\n\n"),
        "{stdout}"
    );
    assert!(
        stdout.contains("  ours    main · current branch\n          at "),
        "{stdout}"
    );
    assert!(stdout.contains(" main change\n"), "{stdout}");
    assert!(
        stdout.contains("  theirs  feature · being merged\n          at "),
        "{stdout}"
    );
    assert!(stdout.contains(" feature change\n"), "{stdout}");
    let (_, files) = stdout
        .split_once("\nFiles with conflicts (1):\n  a.txt\n")
        .expect(&stdout);
    let (id, rest) = files.trim_start().split_once("  ").expect(files);
    assert!(id.starts_with('c') && id.len() == 9, "{files}");
    assert_eq!(rest, "line 1  open\n");

    // Resolved by taking theirs elsewhere, not staged: what was done shows.
    fs::write(dir.path().join("a.txt"), "feature\n").unwrap();
    let stdout = String::from_utf8(git_scry(dir.path(), &["session"]).stdout).unwrap();
    assert!(
        stdout.ends_with(&format!("  a.txt\n    {id}  line 1  theirs\n")),
        "{stdout}"
    );

    // Written by hand: its own text, with its size.
    fs::write(dir.path().join("a.txt"), "feature\nmore\n").unwrap();
    let stdout = String::from_utf8(git_scry(dir.path(), &["session"]).stdout).unwrap();
    assert!(
        stdout.ends_with(&format!("  a.txt\n    {id}  line 1  own text (2 lines)\n")),
        "{stdout}"
    );
}

/// `git-scry mcp` speaks the Model Context Protocol on standard input and output, one
/// message per line, as the agent that registered it starts it: a notification gets no
/// answer, and the server stops with 0 once the agent closes its input.
#[test]
fn mcp_answers_one_message_per_line_until_input_ends() {
    use std::io::Write;
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("gitconfig"), "").unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_git-scry"))
        .current_dir(dir.path())
        .arg("mcp")
        .env("GIT_CONFIG_GLOBAL", dir.path().join("gitconfig"))
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(
            concat!(
                r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}"#,
                "\n",
                r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
                "\n\n",
                r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#,
                "\n",
            )
            .as_bytes(),
        )
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 2, "{stdout}");
    assert!(
        lines[0].starts_with(r#"{"id":1,"#) || lines[0].contains(r#""id":1"#),
        "{stdout}"
    );
    assert!(
        lines[0].contains(r#""protocolVersion":"2025-06-18""#),
        "{stdout}"
    );
    assert!(
        lines[1].contains(r#""id":2"#) && lines[1].contains(r#""name":"overview""#),
        "{stdout}"
    );
}
