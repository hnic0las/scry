//! The MCP server as an agent talks to it: JSON-RPC messages in, answers
//! out, on real repositories stopped in a merge and a rebase, with git's
//! configuration isolated from the user's.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use scry_core::git::GitCli;
use scry_core::records::{Record, Store};
use scry_core::resolution::CandidateKind;
use scry_core::session;
use scry_mcp::Server;
use serde_json::{Value, json};
use tempfile::TempDir;

const BASE: &str = "func first() {\n    let x = 1\n}\n\nlet a = 1\nlet b = 2\nlet c = 3\nlet d = 4\n\nfunc second() {\n    let y = 1\n}\n";
const OURS: &str = "func first() {\n    let x = 2\n}\n\nlet a = 1\nlet b = 2\nlet c = 3\nlet d = 4\n\nfunc second() {\n    let y = 2\n}\n";
const THEIRS: &str = "func first() {\n    let x = 3\n}\n\nlet a = 1\nlet b = 2\nlet c = 3\nlet d = 4\n\nfunc second() {\n    let y = 3\n}\n";

/// A throwaway repository whose git ignores the user's configuration.
struct Repo {
    root: TempDir,
}

impl Repo {
    fn new() -> Repo {
        let root = tempfile::tempdir().unwrap();
        fs::write(
            root.path().join("gitconfig"),
            "[merge]\n\tconflictStyle = zdiff3\n",
        )
        .unwrap();
        fs::create_dir_all(root.path().join("repo")).unwrap();
        let repo = Repo { root };
        repo.ok(&["init", "--quiet", "--initial-branch=main"]);
        repo
    }

    /// Merging `feature` into `main` stops on `Sources/a.swift` (two
    /// conflicts), `staged.txt` (resolved and staged) and
    /// `package-lock.json` (a lockfile, settled in Scry's own view).
    fn merge() -> Repo {
        let repo = Repo::new();
        repo.commit(&[
            ("Sources/a.swift", BASE),
            ("staged.txt", "base\n"),
            ("package-lock.json", "{\"v\": 0}\n"),
        ]);
        repo.ok(&["switch", "--quiet", "-c", "feature"]);
        repo.commit(&[
            ("Sources/a.swift", THEIRS),
            ("staged.txt", "theirs\n"),
            ("package-lock.json", "{\"v\": 2}\n"),
        ]);
        repo.ok(&["switch", "--quiet", "main"]);
        repo.commit(&[
            ("Sources/a.swift", OURS),
            ("staged.txt", "ours\n"),
            ("package-lock.json", "{\"v\": 1}\n"),
        ]);
        assert!(!repo.git(&["merge", "--quiet", "feature"]));
        fs::write(repo.path().join("staged.txt"), "ours\n").unwrap();
        repo.ok(&["add", "staged.txt"]);
        repo
    }

    fn path(&self) -> PathBuf {
        self.root.path().join("repo")
    }

    fn environment(&self) -> Vec<(&'static str, String)> {
        vec![
            (
                "GIT_CONFIG_GLOBAL",
                self.root.path().join("gitconfig").display().to_string(),
            ),
            ("GIT_CONFIG_NOSYSTEM", String::from("1")),
            ("GIT_AUTHOR_NAME", String::from("Ann")),
            ("GIT_AUTHOR_EMAIL", String::from("ann@example.com")),
            ("GIT_COMMITTER_NAME", String::from("Ann")),
            ("GIT_COMMITTER_EMAIL", String::from("ann@example.com")),
        ]
    }

    /// git as the server runs it, started in `dir`.
    fn cli(&self, dir: &Path) -> GitCli {
        self.environment()
            .into_iter()
            .fold(GitCli::new(dir.to_path_buf()), |git, (key, value)| {
                git.with_env(key, &value)
            })
    }

    fn server(&self) -> Server {
        Server::new(self.cli(&self.path()))
    }

    fn git(&self, args: &[&str]) -> bool {
        Command::new("git")
            .arg("-C")
            .arg(self.path())
            .args(args)
            .envs(self.environment())
            .output()
            .unwrap()
            .status
            .success()
    }

    fn ok(&self, args: &[&str]) {
        assert!(self.git(args), "git {args:?} failed");
    }

    /// Commits `files`, the subject being the first file's text's first
    /// line.
    fn commit(&self, files: &[(&str, &str)]) {
        for (name, text) in files {
            let file = self.path().join(name);
            fs::create_dir_all(file.parent().unwrap()).unwrap();
            fs::write(file, text).unwrap();
        }
        self.ok(&["add", "."]);
        let subject = files[0].1.lines().nth(1).unwrap_or(files[0].1).trim();
        self.ok(&["commit", "--quiet", "-m", subject]);
    }
}

/// A request's answer.
fn ask(server: &mut Server, method: &str, params: Value) -> Value {
    let request = json!({ "jsonrpc": "2.0", "id": 7, "method": method, "params": params });
    let answer = server.answer(&request.to_string()).expect("an answer");
    assert_eq!(answer["jsonrpc"], "2.0");
    assert_eq!(answer["id"], 7);
    answer
}

/// A tool's text, and whether it is an error for the agent to correct.
fn call(server: &mut Server, name: &str, arguments: Value) -> (String, bool) {
    let answer = ask(
        server,
        "tools/call",
        json!({ "name": name, "arguments": arguments }),
    );
    let result = &answer["result"];
    assert_eq!(result["content"][0]["type"], "text", "{answer}");
    (
        result["content"][0]["text"].as_str().unwrap().to_string(),
        result["isError"].as_bool().unwrap(),
    )
}

fn text(server: &mut Server, name: &str, arguments: Value) -> String {
    let (text, is_error) = call(server, name, arguments);
    assert!(!is_error, "{text}");
    text
}

/// The ids `conflicts` gives, in order.
fn ids(listed: &str) -> Vec<String> {
    listed
        .lines()
        .filter(|line| line.starts_with('c') && line.contains("  "))
        .map(|line| line.split_whitespace().next().unwrap().to_string())
        .collect()
}

/// An agent connects as the protocol says: `initialize` agrees on a version (the
/// client's when the server speaks it, else the newest it speaks, for the client to
/// decide), the `initialized` notification gets no answer, and `tools/list` gives the
/// three read-only tools in a stable order with their required arguments.
#[test]
fn an_agent_connects_and_lists_the_tools() {
    let mut server = Server::new(GitCli::new(PathBuf::from(".")));
    let answer = ask(
        &mut server,
        "initialize",
        json!({ "protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": { "name": "claude-code", "version": "2.1" } }),
    );
    let result = &answer["result"];
    assert_eq!(result["protocolVersion"], "2025-06-18");
    assert_eq!(result["serverInfo"]["name"], "git-scry");
    assert!(result["capabilities"]["tools"].is_object());
    assert!(
        result["instructions"]
            .as_str()
            .unwrap()
            .contains("overview")
    );
    let answer = ask(
        &mut server,
        "initialize",
        json!({ "protocolVersion": "1999-01-01" }),
    );
    assert_eq!(answer["result"]["protocolVersion"], "2025-11-25");

    let initialized = json!({ "jsonrpc": "2.0", "method": "notifications/initialized" });
    assert_eq!(server.answer(&initialized.to_string()), None);
    assert_eq!(ask(&mut server, "ping", json!({}))["result"], json!({}));

    let tools = ask(&mut server, "tools/list", json!({}))["result"]["tools"].clone();
    let names: Vec<&str> = tools
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["overview", "conflicts", "conflict"]);
    for tool in tools.as_array().unwrap() {
        assert_eq!(tool["annotations"]["readOnlyHint"], true, "{tool}");
        assert_eq!(tool["inputSchema"]["type"], "object", "{tool}");
    }
    assert_eq!(tools[2]["inputSchema"]["required"], json!(["path", "id"]));
}

/// What is not a request the agent should get an answer to is answered by JSON-RPC's
/// errors, or not at all for an answer the client sends: the connection carries on
/// after each.
#[test]
fn malformed_messages_get_json_rpc_errors() {
    let mut server = Server::new(GitCli::new(PathBuf::from(".")));
    let code = |answer: Option<Value>| answer.unwrap()["error"]["code"].clone();
    assert_eq!(code(server.answer("{ not json")), -32700);
    assert_eq!(code(server.answer(r#"{"jsonrpc":"2.0","id":1}"#)), -32600);
    assert_eq!(
        code(server.answer(r#"{"jsonrpc":"2.0","id":1,"method":"resources/list"}"#)),
        -32601
    );
    assert_eq!(
        code(
            server.answer(
                r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"stage"}}"#
            )
        ),
        -32602
    );
    assert_eq!(
        server.answer(r#"{"jsonrpc":"2.0","id":1,"result":{}}"#),
        None
    );
    assert_eq!(ask(&mut server, "ping", json!({}))["result"], json!({}));
}

/// `overview` says what git is doing and who is on each side, counts before lists, and
/// sorts the files by what to do with them: open conflicts for these tools, the
/// lockfile for Scry's own view, the staged file apart.
#[test]
fn the_overview_names_the_operation_its_sides_and_files() {
    let repo = Repo::merge();
    let overview = text(&mut repo.server(), "overview", json!({}));
    assert!(
        overview.starts_with("Merging feature into main\n"),
        "{overview}"
    );
    assert!(
        overview.contains("\nours: main · current branch, at "),
        "{overview}"
    );
    assert!(
        overview.contains("\ntheirs: feature · being merged, at "),
        "{overview}"
    );
    assert!(
        overview.contains("1 file with 2 open conflicts, 1 file to settle in Scry, 1 file staged."),
        "{overview}"
    );
    assert!(
        overview.contains("\n  Sources/a.swift  2 open conflicts\n"),
        "{overview}"
    );
    assert!(
        overview.contains("\n  package-lock.json  npm lockfile\n"),
        "{overview}"
    );
    assert!(overview.contains("\nStaged: staged.txt\n"), "{overview}");
    assert!(overview.ends_with("Next: conflicts(path) lists a file's conflicts."));
}

/// The agent and the app address a conflict by the same id: `conflicts` gives the ids
/// the app gets opening the file, with the lines of the file as it is (git wrote the
/// base in zdiff3), what each side did, and the candidates checked by parsing.
#[test]
fn the_agent_sees_the_ids_the_app_sees() {
    let repo = Repo::merge();
    let listed = text(
        &mut repo.server(),
        "conflicts",
        json!({ "path": "Sources/a.swift" }),
    );
    let file = repo.path().join("Sources/a.swift");
    let opened = session::open_file(&repo.cli(file.parent().unwrap()), &file).unwrap();
    let app_ids: Vec<String> = (0..opened.conflict_count())
        .map(|index| opened.analysis(index).id.clone())
        .collect();
    assert_eq!(ids(&listed), app_ids);
    assert!(
        listed.starts_with("Sources/a.swift (Swift): 2 conflicts, 2 open.\n"),
        "{listed}"
    );
    let first = listed.lines().nth(1).unwrap();
    assert_eq!(
        first,
        format!(
            "{}  line 2  open. Both sides changed these lines. Candidates: ours ✓, theirs ✓, ours-then-theirs ✓, theirs-then-ours ✓, neither ✓, base ✓.",
            app_ids[0]
        )
    );
    let second = listed.lines().nth(2).unwrap();
    assert!(second.contains("  line 17  open."), "{listed}");
}

/// `conflict` gives what an agent needs to resolve one conflict without reading the
/// file: the lines around it, each side with its label and the commit that last
/// changed it, the base, and each candidate's text unless it is a side's.
#[test]
fn a_conflict_shows_its_sides_context_candidates_and_commits() {
    let repo = Repo::merge();
    let mut server = repo.server();
    let listed = text(
        &mut server,
        "conflicts",
        json!({ "path": "Sources/a.swift" }),
    );
    let id = &ids(&listed)[1];
    let shown = text(
        &mut server,
        "conflict",
        json!({ "path": "Sources/a.swift", "id": id, "context": 1 }),
    );
    assert!(
        shown.starts_with(&format!(
            "Conflict {id}, 2 of 2 in Sources/a.swift, line 17: open.\nBoth sides changed these lines.\n"
        )),
        "{shown}"
    );
    assert!(
        shown.contains("\nBefore:\n```\nfunc second() {\n```\n"),
        "{shown}"
    );
    assert!(
        shown.contains("\" (Ann, today):\n```\n    let y = 2\n```\n"),
        "{shown}"
    );
    assert!(
        shown.contains("\nOurs, main · current branch, last changed by "),
        "{shown}"
    );
    assert!(
        shown.contains("\nBase:\n```\n    let y = 1\n```\n"),
        "{shown}"
    );
    assert!(
        shown.contains("\nTheirs, feature · being merged, last changed by "),
        "{shown}"
    );
    assert!(shown.contains("\nAfter:\n```\n}\n```\n"), "{shown}");
    assert!(shown.contains("\n- ours ✓: same text as ours\n"), "{shown}");
    assert!(
        shown.contains("\n- ours-then-theirs ✓:\n```\n    let y = 2\n    let y = 3\n```\n"),
        "{shown}"
    );
    assert!(
        shown.contains("\n- neither ✓: no text, the lines removed\n"),
        "{shown}"
    );
    assert!(shown.ends_with("- base ✓: same text as base"), "{shown}");

    // No context asked: no lines around.
    let bare = text(
        &mut server,
        "conflict",
        json!({ "path": "Sources/a.swift", "id": id, "context": 0 }),
    );
    assert!(
        !bare.contains("Before:") && !bare.contains("After:"),
        "{bare}"
    );
}

/// A call to correct is answered with a sentence saying what to do instead, marked as
/// an error so the agent tries again: a path that is not a conflicted text file of this
/// step (with the ones that are), a staged file, a file for Scry's own views, a stale
/// id (with the current ones), a missing argument, a context out of range.
#[test]
fn calls_to_correct_say_what_to_do() {
    let repo = Repo::merge();
    let mut server = repo.server();
    let refused = |server: &mut Server, name: &str, arguments: Value| {
        let (text, is_error) = call(server, name, arguments);
        assert!(is_error, "{text}");
        text
    };
    assert_eq!(
        refused(&mut server, "conflicts", json!({ "path": "nope.txt" })),
        "nope.txt is not a file git left with conflicts. Paths are from the repository root; files with conflicts: Sources/a.swift."
    );
    assert_eq!(
        refused(&mut server, "conflicts", json!({ "path": "staged.txt" })),
        "staged.txt is staged: git has it. To change it, unstage it in Scry first."
    );
    assert_eq!(
        refused(
            &mut server,
            "conflicts",
            json!({ "path": "package-lock.json" })
        ),
        "package-lock.json is a conflict of another kind (npm lockfile): settle it in Scry, not with these tools."
    );
    assert_eq!(
        refused(&mut server, "conflicts", json!({})),
        "Missing path: a string is required."
    );
    assert_eq!(
        refused(&mut server, "conflicts", json!({ "path": "" })),
        "Missing path: a string is required."
    );
    let listed = text(
        &mut server,
        "conflicts",
        json!({ "path": "./Sources/a.swift" }),
    );
    let stale = refused(
        &mut server,
        "conflict",
        json!({ "path": "Sources/a.swift", "id": "c00000000" }),
    );
    assert_eq!(
        stale,
        format!(
            "No conflict c00000000 in Sources/a.swift: it changed, or never was there. Its conflicts now: {}.",
            ids(&listed).join(", ")
        )
    );
    for context in [json!(-1), json!(101), json!("3")] {
        assert_eq!(
            refused(
                &mut server,
                "conflict",
                json!({ "path": "Sources/a.swift", "id": ids(&listed)[0], "context": context }),
            ),
            "context is a number of lines, from 0 to 100."
        );
    }
}

/// A file resolved outside Scry, partly, reads as the review shows it: a conflict
/// taken from a side as that candidate, by the agent that recorded it, not reviewed
/// yet, the other one still open; the agent's reason is in `conflict`. Once the text
/// around the conflicts changes too, no conflict can be told apart, and the tools say
/// so rather than inventing one.
#[test]
fn a_file_resolved_outside_shows_how() {
    let repo = Repo::merge();
    let mut server = repo.server();
    let listed = text(
        &mut server,
        "conflicts",
        json!({ "path": "Sources/a.swift" }),
    );
    let ids = ids(&listed);
    let file = repo.path().join("Sources/a.swift");
    let marked = fs::read_to_string(&file).unwrap();
    // The first conflict resolved as theirs, the second left open.
    let start = marked.find("<<<<<<<").unwrap();
    let end = marked.find(">>>>>>> feature\n").unwrap() + ">>>>>>> feature\n".len();
    fs::write(
        &file,
        format!("{}    let x = 3\n{}", &marked[..start], &marked[end..]),
    )
    .unwrap();
    Store::open(&repo.cli(&repo.path()))
        .unwrap()
        .add_record(
            "Sources/a.swift",
            Record {
                conflict: ids[0].clone(),
                by: String::from("claude-code 2.1"),
                pick: Some(CandidateKind::Theirs),
                text: String::from("    let x = 3\n"),
                reason: String::from("Keeps feature's x, which main never used."),
                dropped: None,
                at: 1_763_300_000,
            },
        )
        .unwrap();
    let listed = text(
        &mut server,
        "conflicts",
        json!({ "path": "Sources/a.swift" }),
    );
    assert!(
        listed.starts_with("Sources/a.swift (Swift): 2 conflicts, 1 open.\n"),
        "{listed}"
    );
    assert!(
        listed.contains(&format!(
            "{}  line 2  resolved by claude-code 2.1: theirs, not reviewed yet. ",
            ids[0]
        )),
        "{listed}"
    );
    assert!(
        listed.contains(&format!("{}  line 11  open. ", ids[1])),
        "{listed}"
    );
    let shown = text(
        &mut server,
        "conflict",
        json!({ "path": "Sources/a.swift", "id": ids[0] }),
    );
    assert!(
        shown.contains(
            "\nReason given by claude-code 2.1: Keeps feature's x, which main never used.\n"
        ),
        "{shown}"
    );

    // Both resolved as theirs: nothing left for these tools, the review left in Scry.
    fs::write(&file, THEIRS).unwrap();
    let overview = text(&mut server, "overview", json!({}));
    assert!(
        overview.contains(
            "No open conflict, 1 file resolved, not staged (1 to review), 1 file to settle in Scry, 1 file staged.\n"
        ),
        "{overview}"
    );
    assert!(
        overview.contains("\n  Sources/a.swift  to review in Scry: 0 of 2 reviewed\n"),
        "{overview}"
    );
    assert!(
        overview.ends_with("\nNext: conflicts(path) lists a file's conflicts."),
        "{overview}"
    );

    fs::write(&file, OURS.replace("let a = 1", "let a = 10")).unwrap();
    let overview = text(&mut server, "overview", json!({}));
    assert!(
        overview
            .contains("\n  Sources/a.swift  to review in Scry: changed outside its conflicts\n"),
        "{overview}"
    );
    assert_eq!(
        text(
            &mut server,
            "conflicts",
            json!({ "path": "Sources/a.swift" })
        ),
        "Sources/a.swift has no markers left, and its text changed outside its conflicts too: Scry cannot tell its conflicts apart. The user reviews it in Scry."
    );
}

/// In a rebase the overview names the step and the user's commit being replayed, the
/// side people mistake for "ours"; the server answers from any folder of the
/// repository.
#[test]
fn a_rebase_names_the_step_and_the_commit_being_replayed() {
    let repo = Repo::new();
    repo.commit(&[("Sources/a.swift", BASE)]);
    repo.ok(&["switch", "--quiet", "-c", "feature"]);
    repo.commit(&[("Sources/a.swift", THEIRS)]);
    repo.ok(&["switch", "--quiet", "main"]);
    repo.commit(&[("Sources/a.swift", OURS)]);
    repo.ok(&["switch", "--quiet", "feature"]);
    assert!(!repo.git(&["rebase", "--quiet", "main"]));

    let mut server = Server::new(repo.cli(&repo.path().join("Sources")));
    let overview = text(&mut server, "overview", json!({}));
    assert!(
        overview.starts_with("Rebasing feature onto main · commit 1 of 1: "),
        "{overview}"
    );
    assert!(
        overview.contains("\nours: main · rebase target, at "),
        "{overview}"
    );
    assert!(overview.contains("\ntheirs: Your commit "), "{overview}");
    assert!(
        overview.contains("\n  Sources/a.swift  2 open conflicts\n"),
        "{overview}"
    );
}

/// Started outside a repository, or with nothing in progress, the server still
/// answers, saying so, instead of failing the connection. Conflicts left without an
/// operation (by `git stash pop`) are still listed.
#[test]
fn outside_a_repository_or_an_operation_says_so() {
    let repo = Repo::new();
    let mut server = Server::new(repo.cli(repo.root.path()));
    let (text_outside, is_error) = call(&mut server, "overview", json!({}));
    assert!(is_error);
    assert!(
        text_outside.starts_with("Not in a git repository"),
        "{text_outside}"
    );

    repo.commit(&[("a.txt", "a\n")]);
    assert_eq!(
        text(&mut repo.server(), "overview", json!({})),
        "No merge, rebase, cherry-pick or revert in progress, and no file with conflicts."
    );

    fs::write(repo.path().join("a.txt"), "stashed\n").unwrap();
    repo.ok(&["stash", "--quiet"]);
    repo.commit(&[("a.txt", "committed\n")]);
    assert!(!repo.git(&["stash", "pop", "--quiet"]));
    let overview = text(&mut repo.server(), "overview", json!({}));
    assert!(
        overview.starts_with(
            "No merge, rebase, cherry-pick or revert in progress, but files have conflicts"
        ),
        "{overview}"
    );
    assert!(
        overview.contains("\n  a.txt  1 open conflict\n"),
        "{overview}"
    );
}
