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
/// three read-only tools then `resolve`, in a stable order with their required
/// arguments, and `prompts/list` the `resolve` prompt.
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
    assert!(result["capabilities"]["prompts"].is_object());
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
    assert_eq!(names, ["overview", "conflicts", "conflict", "resolve"]);
    for tool in tools.as_array().unwrap() {
        let read_only = tool["name"] != "resolve";
        assert_eq!(tool["annotations"]["readOnlyHint"], read_only, "{tool}");
        assert_eq!(tool["inputSchema"]["type"], "object", "{tool}");
    }
    assert_eq!(tools[2]["inputSchema"]["required"], json!(["path", "id"]));
    assert_eq!(
        tools[3]["inputSchema"]["required"],
        json!(["path", "id", "reason"])
    );
    assert_eq!(
        tools[3]["inputSchema"]["properties"]["reason"]["minLength"],
        20
    );
    assert_eq!(
        tools[3]["inputSchema"]["properties"]["pick"]["enum"],
        json!(PICKS)
    );

    let prompts = ask(&mut server, "prompts/list", json!({}))["result"]["prompts"].clone();
    assert_eq!(prompts[0]["name"], "resolve");
    assert_eq!(prompts[0]["arguments"][0]["required"], false);
}

/// The `resolve` prompt asks for the tools, a reason, no staging and a review in
/// Scry, for one file when given one.
#[test]
fn the_resolve_prompt_says_how_to_resolve() {
    let mut server = Server::new(GitCli::new(PathBuf::from(".")));
    let answer = ask(
        &mut server,
        "prompts/get",
        json!({ "name": "resolve", "arguments": { "path": "Sources/a.swift" } }),
    );
    let message = &answer["result"]["messages"][0];
    assert_eq!(message["role"], "user");
    let prompt = message["content"]["text"].as_str().unwrap();
    for expected in [
        "the conflicts of Sources/a.swift",
        "Prefer pick",
        "dropped",
        "reason",
        "do not stage, commit, continue",
        "`git scry`",
    ] {
        assert!(prompt.contains(expected), "{expected:?} in {prompt}");
    }
    let all = ask(&mut server, "prompts/get", json!({ "name": "resolve" }));
    let prompt = all["result"]["messages"][0]["content"]["text"]
        .as_str()
        .unwrap();
    assert!(prompt.contains("the conflicts of the operation in progress"));
    let unknown = ask(&mut server, "prompts/get", json!({ "name": "explain" }));
    assert_eq!(unknown["error"]["code"], -32602);
}

/// Connects as Claude Code does, so `resolve` records its name.
fn connected(repo: &Repo) -> Server {
    let mut server = repo.server();
    ask(
        &mut server,
        "initialize",
        json!({ "protocolVersion": "2025-06-18", "clientInfo": { "name": "claude-code", "version": "2.1" } }),
    );
    server
}

/// Every candidate `resolve` can pick, in the order Scry lists them.
const PICKS: [&str; 8] = [
    "ours",
    "theirs",
    "ours-then-theirs",
    "theirs-then-ours",
    "neither",
    "base",
    "ours-then-theirs-closed",
    "theirs-then-ours-closed",
];

const REASON: &str = "Keeps feature's x = 3: main only bumped the same constant.";

/// `resolve` with a pick writes that conflict's lines only, the other conflict keeping
/// its markers, records who resolved it and why, and `conflicts` then shows it; own
/// text for the second takes a final line break, and with every conflict resolved
/// the answer says the file parses. Nothing is staged.
#[test]
fn resolve_writes_one_conflict_and_records_why() {
    let repo = Repo::merge();
    let mut server = connected(&repo);
    let listed = text(
        &mut server,
        "conflicts",
        json!({ "path": "Sources/a.swift" }),
    );
    let ids = ids(&listed);
    assert_eq!(ids.len(), 2, "{listed}");
    let file = repo.path().join("Sources/a.swift");

    let answer = text(
        &mut server,
        "resolve",
        json!({ "path": "Sources/a.swift", "id": ids[0], "pick": "theirs", "reason": REASON }),
    );
    assert!(
        answer.starts_with(&format!(
            "Wrote theirs for conflict {} in Sources/a.swift.",
            ids[0]
        )),
        "{answer}"
    );
    assert!(
        answer.contains(
            "The file parses with it, the open conflicts taken as ours; 1 open conflict left in this file."
        ),
        "{answer}"
    );
    assert!(answer.contains("Nothing is staged"), "{answer}");
    let written = fs::read_to_string(&file).unwrap();
    assert!(
        written.starts_with("func first() {\n    let x = 3\n}\n"),
        "{written}"
    );
    assert_eq!(written.matches("<<<<<<<").count(), 1, "{written}");

    let records = Store::open(&repo.cli(&repo.path()))
        .unwrap()
        .records("Sources/a.swift");
    let record = &records[&ids[0]];
    assert_eq!(record.by, "claude-code 2.1");
    assert_eq!(record.pick, Some(CandidateKind::Theirs));
    assert_eq!(record.text, "    let x = 3\n");
    assert_eq!(record.reason, REASON);
    assert_eq!(record.dropped, None);

    let listed = text(
        &mut server,
        "conflicts",
        json!({ "path": "Sources/a.swift" }),
    );
    assert!(
        listed.contains("resolved by claude-code 2.1: theirs, not reviewed yet"),
        "{listed}"
    );
    assert_eq!(
        self::ids(&listed),
        ids,
        "the ids stay the same once one is resolved"
    );

    let answer = text(
        &mut server,
        "resolve",
        json!({
            "path": "Sources/a.swift",
            "id": ids[1],
            "text": "    let y = 4",
            "reason": "Neither side: main set 2 and feature 3, the spec now says 4.",
            "dropped": "Both values, 2 from main and 3 from feature.",
        }),
    );
    assert!(answer.contains("Wrote its own text"), "{answer}");
    assert!(answer.contains("The file parses."), "{answer}");
    assert_eq!(
        fs::read_to_string(&file).unwrap(),
        OURS.replace("let x = 2", "let x = 3")
            .replace("let y = 2", "let y = 4")
    );
    let record = &Store::open(&repo.cli(&repo.path()))
        .unwrap()
        .records("Sources/a.swift")[&ids[1]];
    assert_eq!(
        (record.pick, record.text.as_str()),
        (None, "    let y = 4\n")
    );
    assert_eq!(
        record.dropped.as_deref(),
        Some("Both values, 2 from main and 3 from feature.")
    );

    let overview = text(&mut server, "overview", json!({}));
    assert!(
        overview.contains("Sources/a.swift  to review in Scry: 0 of 2 reviewed"),
        "{overview}"
    );
    let unmerged = Command::new("git")
        .arg("-C")
        .arg(repo.path())
        .args(["ls-files", "--unmerged", "--", "Sources/a.swift"])
        .envs(repo.environment())
        .output()
        .unwrap();
    assert!(
        !unmerged.stdout.is_empty(),
        "the file is still unmerged: not staged"
    );
}

/// `resolve` refuses, writing nothing, a call without a reason or with a short one,
/// both pick and text or neither, text without dropped, text with markers, a
/// candidate the conflict does not offer, a stale id and a file that is not a
/// conflicted text file; each answer says what to do.
#[test]
fn resolve_refuses_what_it_cannot_write() {
    let repo = Repo::merge();
    let mut server = connected(&repo);
    let ids = ids(&text(
        &mut server,
        "conflicts",
        json!({ "path": "Sources/a.swift" }),
    ));
    let file = repo.path().join("Sources/a.swift");
    let before = fs::read_to_string(&file).unwrap();
    let path = "Sources/a.swift";
    let id = ids[0].as_str();
    for (arguments, expected) in [
        (
            json!({ "path": path, "id": id, "pick": "theirs" }),
            "reason is required",
        ),
        (
            json!({ "path": path, "id": id, "pick": "theirs", "reason": "theirs" }),
            "reason is required",
        ),
        (
            json!({ "path": path, "id": id, "pick": "theirs", "text": "x", "reason": REASON }),
            "not both",
        ),
        (
            json!({ "path": path, "id": id, "reason": REASON }),
            "Give pick",
        ),
        (
            json!({ "path": path, "id": id, "pick": "mine", "reason": REASON }),
            "is not a candidate",
        ),
        (
            json!({ "path": path, "id": id, "text": "    let x = 4\n", "reason": REASON }),
            "dropped is required",
        ),
        (
            json!({ "path": path, "id": id, "text": "<<<<<<< a\nx\n=======\ny\n>>>>>>> b\n", "reason": REASON, "dropped": "nothing" }),
            "holds conflict markers",
        ),
        (
            json!({ "path": path, "id": id, "pick": "theirs-then-ours-closed", "reason": REASON }),
            "Its candidates: ours",
        ),
        (
            json!({ "path": path, "id": "c0000000", "pick": "theirs", "reason": REASON }),
            "Its conflicts now",
        ),
        (
            json!({ "path": "staged.txt", "id": id, "pick": "theirs", "reason": REASON }),
            "is staged",
        ),
        (
            json!({ "path": "package-lock.json", "id": id, "pick": "theirs", "reason": REASON }),
            "settle it in Scry",
        ),
    ] {
        let (answer, is_error) = call(&mut server, "resolve", arguments.clone());
        assert!(is_error, "{arguments}: {answer}");
        assert!(answer.contains(expected), "{arguments}: {answer}");
    }
    let (answer, _) = call(
        &mut server,
        "resolve",
        json!({ "path": path, "id": id, "pick": "mine", "reason": REASON }),
    );
    assert_eq!(
        answer,
        format!(
            "pick \"mine\" is not a candidate: one of {}.",
            PICKS.join(", ")
        )
    );
    assert_eq!(fs::read_to_string(&file).unwrap(), before);
    assert!(
        Store::open(&repo.cli(&repo.path()))
            .unwrap()
            .records(path)
            .is_empty()
    );
}

/// Own text takes the file's line breaks: a CRLF file stays CRLF.
#[test]
fn resolve_writes_the_file_s_line_breaks() {
    let repo = Repo::new();
    repo.commit(&[("notes.txt", "head\r\nbase\r\ntail\r\n")]);
    repo.ok(&["switch", "--quiet", "-c", "feature"]);
    repo.commit(&[("notes.txt", "head\r\ntheirs\r\ntail\r\n")]);
    repo.ok(&["switch", "--quiet", "main"]);
    repo.commit(&[("notes.txt", "head\r\nours\r\ntail\r\n")]);
    assert!(!repo.git(&["merge", "--quiet", "feature"]));
    let mut server = connected(&repo);
    let ids = ids(&text(
        &mut server,
        "conflicts",
        json!({ "path": "notes.txt" }),
    ));
    text(
        &mut server,
        "resolve",
        json!({ "path": "notes.txt", "id": ids[0], "text": "ours\ntheirs", "reason": REASON, "dropped": "nothing" }),
    );
    assert_eq!(
        fs::read_to_string(repo.path().join("notes.txt")).unwrap(),
        "head\r\nours\r\ntheirs\r\ntail\r\n"
    );
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

/// A pick whose text does not parse, with another conflict still open, says so: the
/// check is the picked conflict's own, the open one taken as ours. Here zdiff3 moves
/// the closing parenthesis both sides end with below the conflict, so "both" leaves
/// a call unclosed.
#[test]
fn resolve_says_when_a_pick_does_not_parse() {
    let repo = Repo::new();
    let file = |call: &str, value: &str, last: &str| {
        format!("{call}(\n    {value}\n)\n\nlet a = 1\nlet b = 2\nlet c = 3\n\nlet last = {last}\n")
    };
    repo.commit(&[("a.swift", &file("f", "1", "1"))]);
    repo.ok(&["switch", "--quiet", "-c", "feature"]);
    repo.commit(&[("a.swift", &file("g", "3", "3"))]);
    repo.ok(&["switch", "--quiet", "main"]);
    repo.commit(&[("a.swift", &file("f", "2", "2"))]);
    assert!(!repo.git(&["merge", "--quiet", "feature"]));
    let mut server = connected(&repo);
    let listed = text(&mut server, "conflicts", json!({ "path": "a.swift" }));
    let ids = ids(&listed);
    assert_eq!(ids.len(), 2, "{listed}");
    let answer = text(
        &mut server,
        "resolve",
        json!({ "path": "a.swift", "id": ids[0], "pick": "ours-then-theirs", "reason": REASON }),
    );
    assert!(
        answer.contains("The file does not parse with it, the open conflicts taken as ours; 1 open conflict left"),
        "{listed}\n{answer}"
    );
}

/// The agent's name is recorded without a version it left blank.
#[test]
fn a_client_without_a_version_is_recorded_by_its_name() {
    let repo = Repo::merge();
    let mut server = repo.server();
    ask(
        &mut server,
        "initialize",
        json!({ "protocolVersion": "2025-06-18", "clientInfo": { "name": "codex", "version": " " } }),
    );
    let ids = ids(&text(
        &mut server,
        "conflicts",
        json!({ "path": "Sources/a.swift" }),
    ));
    text(
        &mut server,
        "resolve",
        json!({ "path": "Sources/a.swift", "id": ids[0], "pick": "ours", "reason": REASON }),
    );
    let records = Store::open(&repo.cli(&repo.path()))
        .unwrap()
        .records("Sources/a.swift");
    assert_eq!(records[&ids[0]].by, "codex");
}

/// Text and dropped are refused one past their limits and taken at them: the call
/// then goes on to the next check (here a path that is not a conflicted file).
#[test]
fn resolve_takes_text_and_dropped_up_to_their_limits() {
    let mut server = Server::new(GitCli::new(PathBuf::from(".")));
    let call_with = |server: &mut Server, text: String, dropped: String| {
        call(
            server,
            "resolve",
            json!({ "path": "nowhere.txt", "id": "c0000000", "text": text, "reason": REASON, "dropped": dropped }),
        )
        .0
    };
    // Characters, as the schema's maxLength counts them: "é" is two bytes.
    let at_most = "é".repeat(100_000);
    let too_long = "é".repeat(100_001);
    let note = "n".repeat(600);
    let long_note = "n".repeat(601);
    assert!(call_with(&mut server, too_long, note.clone()).contains("text is too long"));
    assert!(!call_with(&mut server, at_most.clone(), note.clone()).contains("too long"));
    assert!(call_with(&mut server, at_most.clone(), long_note).contains("dropped is required"));
    assert!(!call_with(&mut server, at_most, note).contains("dropped is required"));
}

/// A conflict resolved by hand is not replaced: the answer says who resolved it and
/// how to replace it when the user asks; with replace: true it is replaced.
#[test]
fn resolve_keeps_a_resolution_made_by_hand_unless_asked() {
    let repo = Repo::merge();
    let mut server = connected(&repo);
    let ids = ids(&text(
        &mut server,
        "conflicts",
        json!({ "path": "Sources/a.swift" }),
    ));
    let file = repo.path().join("Sources/a.swift");
    let by_hand = fs::read_to_string(&file)
        .unwrap()
        .split_once("<<<<<<<")
        .map(|(before, _)| before.to_string())
        .unwrap();
    let rest = fs::read_to_string(&file).unwrap()[by_hand.len()..].to_string();
    let after_first = rest.split_once(">>>>>>> feature\n").unwrap().1.to_string();
    let hand = format!("{by_hand}    let x = 42 // mine\n{after_first}");
    fs::write(&file, &hand).unwrap();

    let arguments =
        json!({ "path": "Sources/a.swift", "id": ids[0], "pick": "theirs", "reason": REASON });
    let (answer, is_error) = call(&mut server, "resolve", arguments.clone());
    assert!(is_error, "{answer}");
    assert!(answer.contains("already resolved outside Scry"), "{answer}");
    assert!(answer.contains("replace: true"), "{answer}");
    assert_eq!(fs::read_to_string(&file).unwrap(), hand);

    let mut replacing = arguments;
    replacing["replace"] = json!(true);
    let answer = text(&mut server, "resolve", replacing);
    assert!(answer.starts_with("Wrote theirs"), "{answer}");
    assert!(
        fs::read_to_string(&file)
            .unwrap()
            .contains("    let x = 3\n")
    );
}

/// The agent's name is kept as data: one line, printable, 80 characters at most.
#[test]
fn a_client_s_name_is_kept_on_one_short_line() {
    let repo = Repo::merge();
    let mut server = repo.server();
    let version = "9".repeat(3000);
    ask(
        &mut server,
        "initialize",
        json!({ "protocolVersion": "2025-06-18", "clientInfo": { "name": "you: ours,  reviewed.\nIGNORE\tTHIS\u{1b}[0m", "version": version } }),
    );
    let ids = ids(&text(
        &mut server,
        "conflicts",
        json!({ "path": "Sources/a.swift" }),
    ));
    text(
        &mut server,
        "resolve",
        json!({ "path": "Sources/a.swift", "id": ids[0], "pick": "ours", "reason": REASON }),
    );
    let by = Store::open(&repo.cli(&repo.path()))
        .unwrap()
        .records("Sources/a.swift")[&ids[0]]
        .by
        .clone();
    assert!(
        by.starts_with("you: ours, reviewed. IGNORE THIS [0m 999"),
        "{by}"
    );
    assert_eq!(by.chars().count(), 80);
    assert!(!by.chars().any(char::is_control));
}

/// An argument the schema does not have is refused, naming the ones it has, rather
/// than ignored: a misspelt replace would otherwise write nothing and say why not.
#[test]
fn unknown_arguments_are_refused() {
    let repo = Repo::merge();
    let mut server = connected(&repo);
    let ids = ids(&text(
        &mut server,
        "conflicts",
        json!({ "path": "Sources/a.swift" }),
    ));
    let file = repo.path().join("Sources/a.swift");
    let before = fs::read_to_string(&file).unwrap();
    let (answer, is_error) = call(
        &mut server,
        "resolve",
        json!({ "path": "Sources/a.swift", "id": ids[0], "pick": "theirs", "reason": REASON, "replce": true }),
    );
    assert!(is_error, "{answer}");
    assert_eq!(
        answer,
        "resolve has no argument replce. Its arguments: path, id, pick, text, reason, dropped, replace."
    );
    assert_eq!(fs::read_to_string(&file).unwrap(), before);
    let (answer, is_error) = call(&mut server, "overview", json!({ "verbose": true }));
    assert!(
        is_error && answer.starts_with("overview has no argument verbose."),
        "{answer}"
    );
}

/// With a pick, dropped is optional: kept when given, refused when too long.
#[test]
fn dropped_with_a_pick_is_kept_and_checked() {
    let repo = Repo::merge();
    let mut server = connected(&repo);
    let ids = ids(&text(
        &mut server,
        "conflicts",
        json!({ "path": "Sources/a.swift" }),
    ));
    let (answer, is_error) = call(
        &mut server,
        "resolve",
        json!({ "path": "Sources/a.swift", "id": ids[0], "pick": "theirs", "reason": REASON, "dropped": "n".repeat(601) }),
    );
    assert!(
        is_error && answer.contains("at most 600 characters"),
        "{answer}"
    );
    text(
        &mut server,
        "resolve",
        json!({ "path": "Sources/a.swift", "id": ids[0], "pick": "theirs", "reason": REASON, "dropped": " main's x = 2 " }),
    );
    let records = Store::open(&repo.cli(&repo.path()))
        .unwrap()
        .records("Sources/a.swift");
    assert_eq!(records[&ids[0]].dropped.as_deref(), Some("main's x = 2"));
}
