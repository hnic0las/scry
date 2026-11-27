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

/// A repository in `root/repo` with one commit, git's configuration kept
/// beside it so that it is not one of the repository's files.
fn repository(root: &std::path::Path, files: &[(&str, &str)]) -> std::path::PathBuf {
    let repo = root.join("repo");
    fs::create_dir(&repo).unwrap();
    fs::write(root.join("gitconfig"), "").unwrap();
    for (path, content) in files {
        let path = repo.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }
    for args in [
        &["init", "--quiet", "--initial-branch=main"][..],
        &["add", "."],
        &["commit", "--quiet", "-m", "base"],
    ] {
        let status = Command::new("git")
            .current_dir(&repo)
            .args(args)
            .env("GIT_CONFIG_GLOBAL", root.join("gitconfig"))
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_AUTHOR_NAME", "Scry Test")
            .env("GIT_AUTHOR_EMAIL", "test@example.com")
            .env("GIT_COMMITTER_NAME", "Scry Test")
            .env("GIT_COMMITTER_EMAIL", "test@example.com")
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?}");
    }
    repo
}

/// `git-scry` in `dir`, a folder of a `repository`.
fn git_scry_in(root: &std::path::Path, dir: &std::path::Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_git-scry"))
        .current_dir(dir)
        .args(args)
        .env("GIT_CONFIG_GLOBAL", root.join("gitconfig"))
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .unwrap()
}

/// `diff --print` lists everything not committed in its groups, with a
/// title saying what is compared and the totals; unlike `git diff`, a new
/// file git does not know yet is part of it.
#[test]
fn diff_print_lists_what_is_not_committed() {
    let root = tempfile::tempdir().unwrap();
    let repo = repository(root.path(), &[("src/a.txt", "one\ntwo\n")]);
    fs::write(repo.join("src/a.txt"), "one\n2\nthree\n").unwrap();
    fs::write(repo.join("new.txt"), "x\n").unwrap();

    let output = git_scry_in(root.path(), &repo, &["diff", "--print"]);
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "Not committed, against HEAD · 2 files · +3 −1\n\
         Not staged\n  M  src/a.txt  +2 −1\n\
         Untracked\n  U  new.txt  +1 −0\n"
    );

    fs::remove_file(repo.join("new.txt")).unwrap();
    fs::write(repo.join("src/a.txt"), "one\ntwo\n").unwrap();
    let output = git_scry_in(root.path(), &repo, &["diff", "--print"]);
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "Not committed, against HEAD\nNothing to show: no file differs.\n"
    );
}

/// `--print=unified` and `--print=split` show each file whole after the
/// list, with both line numbers and the changed words marked, and a path
/// after `--` is read from the current folder, not from the root.
#[test]
fn diff_print_shows_each_file_whole_in_either_layout() {
    let root = tempfile::tempdir().unwrap();
    let repo = repository(
        root.path(),
        &[("src/a.txt", "one\ntimeout = 30\n"), ("b.txt", "b\n")],
    );
    fs::write(repo.join("src/a.txt"), "one\ntimeout = 60\nlast\n").unwrap();
    fs::write(repo.join("b.txt"), "B\n").unwrap();

    let src = repo.join("src");
    let output = git_scry_in(
        root.path(),
        &src,
        &["diff", "--print=unified", "--", "a.txt"],
    );
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "Not committed, against HEAD · 1 file · +2 −1\n\
         Not staged\n  M  src/a.txt  +2 −1\n\
         \n== src/a.txt\n\
         \x20   1     1   one\n\
         \x20   2       - timeout = [-30-]\n\
         \x20         2 + timeout = {+60+}\n\
         \x20         3 + last\n"
    );

    let output = git_scry_in(root.path(), &src, &["diff", "--print=split", "--", "a.txt"]);
    let stdout = String::from_utf8(output.stdout).unwrap();
    let rows: Vec<&str> = stdout.lines().skip(5).collect();
    assert_eq!(rows.len(), 3, "{stdout}");
    assert!(rows[1].starts_with("    2 timeout = [-30-]"), "{stdout}");
    assert!(rows[1].ends_with("│     2 timeout = {+60+}"), "{stdout}");
    assert!(rows[2].trim_start().starts_with("│     3 last"), "{stdout}");
}

/// A revision compares the working copy with it, `a..b` two commits and
/// `--staged` the index, each said in the title; `--staged` with a range
/// is refused, since the index has one commit to be compared with.
#[test]
fn diff_print_reads_revisions_ranges_and_staged() {
    let root = tempfile::tempdir().unwrap();
    let repo = repository(root.path(), &[("a.txt", "one\n")]);
    fs::write(repo.join("a.txt"), "two\n").unwrap();
    let print = |args: &[&str]| {
        let mut all = vec!["diff", "--print"];
        all.extend_from_slice(args);
        let output = git_scry_in(root.path(), &repo, &all);
        (
            output.status.code(),
            String::from_utf8(output.stdout).unwrap(),
        )
    };

    let (code, stdout) = print(&["main"]);
    assert_eq!(code, Some(0));
    assert!(
        stdout.starts_with("Working copy, against main · 1 file · +1 −1\n"),
        "{stdout}"
    );
    let (_, stdout) = print(&["main..HEAD"]);
    assert_eq!(
        stdout,
        "HEAD, against main\nNothing to show: no file differs.\n"
    );
    let (_, stdout) = print(&["main..."]);
    assert!(
        stdout.starts_with("HEAD, against its merge base with main ("),
        "{stdout}"
    );
    let (_, stdout) = print(&["--staged"]);
    assert!(stdout.starts_with("Staged, against HEAD\n"), "{stdout}");

    let (code, stdout) = print(&["--staged", "main..HEAD"]);
    assert_eq!((code, stdout.as_str()), (Some(2), ""));
}

/// An executable standing for the app: it writes its arguments, one per
/// line, to the file returned.
#[cfg(unix)]
fn fake_app(dir: &std::path::Path) -> (std::path::PathBuf, std::path::PathBuf) {
    use std::os::unix::fs::PermissionsExt;
    let app = dir.join("fake-scry");
    let arguments = dir.join("arguments");
    fs::write(
        &app,
        format!(
            "#!/bin/sh\nprintf '%s\\n' \"$@\" > '{}'\n",
            arguments.display()
        ),
    )
    .unwrap();
    fs::set_permissions(&app, fs::Permissions::from_mode(0o755)).unwrap();
    (app, arguments)
}

/// Without `--print`, `diff` opens the app's Changes window: the app gets
/// the repository's root after `--diff`, then what was asked, each path
/// from the root after `--path`, whatever folder the command ran in.
#[cfg(unix)]
#[test]
fn diff_opens_the_app_on_what_was_asked() {
    let root = tempfile::tempdir().unwrap();
    let repo = repository(root.path(), &[("src/a.txt", "one\n")]);
    let (app, arguments) = fake_app(root.path());
    let app = format!("--app={}", app.display());
    let canonical = fs::canonicalize(&repo).unwrap().display().to_string();

    let output = git_scry_in(root.path(), &repo.join("src"), &["diff", &app]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(
        fs::read_to_string(&arguments).unwrap(),
        format!("--diff\n{canonical}\n")
    );
    assert!(
        String::from_utf8(output.stdout)
            .unwrap()
            .starts_with("Opening ")
    );

    let asked = ["diff", &app, "--staged", "main", "--", "a.txt", "b c.txt"];
    let output = git_scry_in(root.path(), &repo.join("src"), &asked);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(
        fs::read_to_string(&arguments).unwrap(),
        format!(
            "--diff\n{canonical}\n--staged\n--revisions\nmain\n\
             --path\nsrc/a.txt\n--path\nsrc/b c.txt\n"
        )
    );
}

/// A revision git does not know, a range with `--staged` and a folder
/// outside any repository are reported in the terminal with exit code 2,
/// and the app is not opened on something it could not show.
#[cfg(unix)]
#[test]
fn diff_reports_what_it_cannot_read_without_opening_the_app() {
    let root = tempfile::tempdir().unwrap();
    let repo = repository(root.path(), &[("a.txt", "one\n")]);
    let (app, arguments) = fake_app(root.path());
    let app = format!("--app={}", app.display());

    for asked in [&["no-such-branch"][..], &["--staged", "main..HEAD"]] {
        let mut all = vec!["diff", app.as_str()];
        all.extend_from_slice(asked);
        let output = git_scry_in(root.path(), &repo, &all);
        assert_eq!(output.status.code(), Some(2), "{asked:?}");
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert!(stderr.starts_with("git-scry: "), "{stderr}");
        assert!(!arguments.exists(), "{asked:?}");
    }

    let outside = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_git-scry"))
        .current_dir(outside.path())
        .args(["diff", &app])
        .env("GIT_CEILING_DIRECTORIES", outside.path().parent().unwrap())
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .starts_with("git-scry: ")
    );
    assert!(!arguments.exists());
}

/// `git scry` alone, with no merge, rebase, cherry-pick or revert in
/// progress, opens the Changes window on what is not committed instead of
/// an overview with nothing to show.
#[cfg(unix)]
#[test]
fn git_scry_alone_opens_the_changes_outside_an_operation() {
    let root = tempfile::tempdir().unwrap();
    let repo = repository(root.path(), &[("a.txt", "one\n")]);
    let (app, arguments) = fake_app(root.path());
    let output = Command::new(env!("CARGO_BIN_EXE_git-scry"))
        .current_dir(&repo)
        .env("SCRY_APP", &app)
        .env("GIT_CONFIG_GLOBAL", root.path().join("gitconfig"))
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let canonical = fs::canonicalize(&repo).unwrap().display().to_string();
    assert_eq!(
        fs::read_to_string(&arguments).unwrap(),
        format!("--diff\n{canonical}\n")
    );
}

/// Files without lines to count say what they are, in the list and in
/// their place among the files shown whole: a binary file with its
/// sizes, a folder with a repository of its own, a file left with
/// conflicts by a merge; and a last line that lost its line ending is
/// noted under the file.
#[test]
fn diff_print_says_what_has_no_lines_to_show() {
    let root = tempfile::tempdir().unwrap();
    let repo = repository(root.path(), &[("a.txt", "one\n"), ("c.txt", "base\n")]);
    let git = |dir: &std::path::Path, args: &[&str]| {
        Command::new("git")
            .current_dir(dir)
            .args(args)
            .env("GIT_CONFIG_GLOBAL", root.path().join("gitconfig"))
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
    // A merge that stops on c.txt.
    assert!(git(&repo, &["switch", "--quiet", "-c", "feature"]));
    fs::write(repo.join("c.txt"), "feature\n").unwrap();
    assert!(git(&repo, &["commit", "--quiet", "-am", "feature"]));
    assert!(git(&repo, &["switch", "--quiet", "main"]));
    fs::write(repo.join("c.txt"), "main\n").unwrap();
    assert!(git(&repo, &["commit", "--quiet", "-am", "main"]));
    assert!(!git(&repo, &["merge", "--quiet", "feature"]));

    fs::write(repo.join("a.txt"), "one").unwrap();
    fs::write(repo.join("icon.png"), [0u8, 1, 2]).unwrap();
    fs::create_dir(repo.join("vendor")).unwrap();
    assert!(git(&repo.join("vendor"), &["init", "--quiet"]));
    fs::write(repo.join("vendor/inner.txt"), "x\n").unwrap();
    // Too large to be read for the list.
    fs::write(repo.join("big.log"), "0123456789\n".repeat(200_000)).unwrap();

    let output = git_scry_in(root.path(), &repo, &["diff", "--print"]);
    let list = String::from_utf8(output.stdout).unwrap();
    assert!(list.contains("\n  U  big.log  not counted\n"), "{list}");
    fs::remove_file(repo.join("big.log")).unwrap();

    let output = git_scry_in(root.path(), &repo, &["diff", "--print=unified"]);
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "Not committed, against HEAD · 4 files · +1 −1\n\
         Not staged\n  M  a.txt  +1 −1\n  !  c.txt  conflicted\n\
         Untracked\n  B  icon.png  binary\n  U  vendor  a repository of its own\n\
         \n== a.txt\n\
         \x20   1       - one\n\
         \x20         1 + one\n\
         (no line ending after the last line of the new side)\n\
         \n== c.txt\nThis file has conflicts: resolve them first (git scry).\n\
         \n== icon.png\nBinary file, nothing → 3 bytes\n\
         \n== vendor\nA repository of its own.\n"
    );
}

/// Run by `git difftool`, once per file, `difftool` opens the app on the
/// two files git hands over, by absolute paths, with the file's name in
/// the repository; with `--dir-diff`, once on the two folders, with no
/// name. A file one side does not have stays `/dev/null`.
#[cfg(unix)]
#[test]
fn git_difftool_opens_the_app_on_each_file_or_on_the_folders() {
    let root = tempfile::tempdir().unwrap();
    let repo = repository(root.path(), &[("src/a.txt", "one\n")]);
    fs::write(repo.join("src/a.txt"), "two\n").unwrap();
    fs::write(repo.join("new.txt"), "new\n").unwrap();
    let (app, arguments) = fake_app(root.path());
    // The fake app keeps every call, one line per argument.
    fs::write(
        &app,
        format!(
            "#!/bin/sh\nprintf '%s\\n' \"$@\" >> '{0}'\necho == >> '{0}'\n",
            arguments.display()
        ),
    )
    .unwrap();
    let command = format!(
        "'{}' difftool --app='{}' -- \"$LOCAL\" \"$REMOTE\" \"$MERGED\"",
        env!("CARGO_BIN_EXE_git-scry"),
        app.display()
    );
    let git = |args: &[&str]| {
        let output = Command::new("git")
            .current_dir(&repo)
            .args(["-c", &format!("difftool.scry.cmd={command}")])
            .args(["-c", "difftool.prompt=false", "-c", "diff.tool=scry"])
            .args(args)
            .env("GIT_CONFIG_GLOBAL", root.path().join("gitconfig"))
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
    };

    git(&["add", "new.txt"]);
    git(&["difftool", "HEAD"]);
    let calls = fs::read_to_string(&arguments).unwrap();
    let calls: Vec<Vec<&str>> = calls
        .trim_end()
        .split("\n==")
        .map(|call| call.trim().lines().collect())
        .filter(|call: &Vec<&str>| !call.is_empty())
        .collect();
    assert_eq!(calls.len(), 2, "{calls:?}");
    let added = &calls[0];
    assert_eq!(added[0], "--difftool");
    assert_eq!((added[1], added[2]), ("--old", "/dev/null"));
    assert_eq!(added[3], "--new");
    assert!(
        added[4].starts_with('/') && added[4].ends_with("/new.txt"),
        "{added:?}"
    );
    assert_eq!((added[5], added[6]), ("--name", "new.txt"));
    let modified = &calls[1];
    assert!(
        modified[2].starts_with('/') && modified[2].ends_with("/a.txt"),
        "{modified:?}"
    );
    assert!(modified[4].ends_with("/src/a.txt"), "{modified:?}");
    assert_eq!((modified[5], modified[6]), ("--name", "src/a.txt"));

    // Each call knows its place among the files git opens in turn.
    assert_eq!(added[7..], ["--place", "1", "--of", "2"]);
    assert_eq!(modified[7..], ["--place", "2", "--of", "2"]);

    fs::remove_file(&arguments).unwrap();
    git(&["difftool", "--dir-diff", "HEAD"]);
    let call = fs::read_to_string(&arguments).unwrap();
    let call: Vec<&str> = call.lines().collect();
    assert_eq!(call.len(), 6, "{call:?}");
    assert!(
        call[2].ends_with("/left/") || call[2].ends_with("/left"),
        "{call:?}"
    );
    assert!(
        call[4].ends_with("/right/") || call[4].ends_with("/right"),
        "{call:?}"
    );
    assert_eq!(call[5], "==");

    // The app answers 1 for Stop: git opens no other file.
    fs::write(
        &app,
        format!(
            "#!/bin/sh\necho call >> '{}'\nexit 1\n",
            arguments.display()
        ),
    )
    .unwrap();
    fs::remove_file(&arguments).unwrap();
    let stopped = Command::new("git")
        .current_dir(&repo)
        .args(["-c", &format!("difftool.scry.cmd={command}")])
        .args(["-c", "difftool.prompt=false", "-c", "diff.tool=scry"])
        .args(["difftool", "HEAD"])
        .env("GIT_CONFIG_GLOBAL", root.path().join("gitconfig"))
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .unwrap();
    assert!(!stopped.status.success(), "{stopped:?}");
    assert_eq!(fs::read_to_string(&arguments).unwrap(), "call\n");
}
