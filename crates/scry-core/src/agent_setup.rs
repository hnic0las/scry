//! Registering Scry's MCP server in Claude Code, for every project of the
//! user (`claude mcp add --scope user`), from the app's settings.
//!
//! The one exception to "the app never runs the `claude` CLI" (vault
//! decision 0002, amended on 2026-11-17): `claude mcp get`, `add` and
//! `remove` for the server named `scry`, on the user's click, to change
//! Claude Code's configuration. Nothing here starts a session, sends a
//! prompt or reads credentials.
//!
//! What to run and how to read its answer is decided here; running it is
//! the front end's, which knows the user's `PATH`.

use std::path::{Path, PathBuf};

use crate::cli_link;

/// The name the server is registered under, and its tools shown with.
pub const SERVER: &str = "scry";

/// How Claude Code knows Scry's server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Registration {
    /// It does not.
    None,
    /// For every project of the user (`--scope user`).
    User { command: String, args: Vec<String> },
    /// For one folder only (local or project scope): the one `claude` ran
    /// in.
    Folder,
}

/// Where the settings stand, and what the button does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentState {
    /// No `claude` on the user's `PATH`: the command is shown to run once
    /// Claude Code is installed.
    NoClaude,
    /// Claude Code does not know Scry's server, or only in one folder.
    NotRegistered,
    /// Claude Code starts this `git-scry` in every project.
    Registered,
    /// Claude Code starts another command (another copy of Scry, a build
    /// moved since): the button registers this one instead.
    Other { command: String },
}

/// What the settings show for `registration`, wanting Claude Code to run
/// `git_scry`.
pub fn state(registration: Option<&Registration>, git_scry: &Path) -> AgentState {
    match registration {
        None => AgentState::NoClaude,
        Some(Registration::None | Registration::Folder) => AgentState::NotRegistered,
        Some(Registration::User { command, args }) => {
            if Path::new(command) == git_scry && args == &["mcp"] {
                AgentState::Registered
            } else {
                AgentState::Other {
                    command: std::iter::once(command.as_str())
                        .chain(args.iter().map(String::as_str))
                        .collect::<Vec<_>>()
                        .join(" "),
                }
            }
        }
    }
}

/// Where `claude` is: first on the user's `PATH` (`path`, its folders in
/// order), else where Claude Code's installers put it, since a `PATH` read
/// from the shell can miss what a shell's startup file adds late.
pub fn find_claude(path: &[PathBuf], home: &Path) -> Option<PathBuf> {
    let installed = [
        home.join(".local/bin"),
        home.join(".claude/local"),
        PathBuf::from("/opt/homebrew/bin"),
        PathBuf::from("/usr/local/bin"),
    ];
    path.iter()
        .chain(installed.iter())
        .map(|folder| folder.join("claude"))
        .find(|file| cli_link::is_executable(file))
}

/// The arguments of `claude` that register `git_scry` for every project.
pub fn add_arguments(git_scry: &Path) -> Vec<String> {
    vec![
        String::from("mcp"),
        String::from("add"),
        String::from("--scope"),
        String::from("user"),
        String::from(SERVER),
        String::from("--"),
        git_scry.display().to_string(),
        String::from("mcp"),
    ]
}

/// The arguments of `claude` that remove the server registered for every
/// project.
pub fn remove_arguments() -> Vec<String> {
    vec![
        String::from("mcp"),
        String::from("remove"),
        String::from("--scope"),
        String::from("user"),
        String::from(SERVER),
    ]
}

/// The arguments of `claude` that describe the server.
pub fn get_arguments() -> Vec<String> {
    vec![
        String::from("mcp"),
        String::from("get"),
        String::from(SERVER),
    ]
}

/// The command to copy into a terminal, the path quoted for the shell when
/// it needs to be.
pub fn command(git_scry: &Path) -> String {
    format!(
        "claude mcp add --scope user {SERVER} -- {} mcp",
        shell_quoted(&git_scry.display().to_string())
    )
}

/// What `claude mcp get scry` printed, with its exit status:
///
/// ```text
/// scry:
///   Scope: User config (available in all your projects)
///   Status: ✔ Connected
///   Type: stdio
///   Command: /opt/homebrew/bin/git-scry
///   Args: mcp
/// ```
///
/// A failure is no server; a server whose scope is not the user's is one
/// folder's.
pub fn parse_get(output: &str, succeeded: bool) -> Registration {
    if !succeeded {
        return Registration::None;
    }
    let field = |name: &str| {
        output.lines().find_map(|line| {
            line.trim()
                .strip_prefix(name)
                .and_then(|rest| rest.strip_prefix(':'))
                .map(|value| value.trim().to_string())
        })
    };
    let user = field("Scope").is_some_and(|scope| scope.starts_with("User"));
    match (user, field("Command")) {
        (true, Some(command)) => Registration::User {
            command,
            args: field("Args")
                .map(|args| args.split_whitespace().map(String::from).collect())
                .unwrap_or_default(),
        },
        _ => Registration::Folder,
    }
}

/// `text` as one shell word: as it is when it holds nothing the shell
/// reads, else between single quotes.
fn shell_quoted(text: &str) -> String {
    let plain = !text.is_empty()
        && text
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "/._-+".contains(character));
    match plain {
        true => text.to_string(),
        false => format!("'{}'", text.replace('\'', "'\\''")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const USER: &str = "scry:\n  Scope: User config (available in all your projects)\n  Status: ✔ Connected\n  Type: stdio\n  Command: /opt/homebrew/bin/git-scry\n  Args: mcp\n  Environment:\n\nTo remove this server, run: claude mcp remove scry -s user\n";
    const LOCAL: &str = "scry:\n  Scope: Local config (private to you in this project)\n  Status: ✔ Connected\n  Type: stdio\n  Command: /tmp/git-scry\n  Args: mcp\n";

    /// Claude Code's description of the server is read for its scope and
    /// command: registered for every project, for one folder, or not at
    /// all when `claude mcp get` fails.
    #[test]
    fn claude_s_description_is_read() {
        assert_eq!(
            parse_get(USER, true),
            Registration::User {
                command: String::from("/opt/homebrew/bin/git-scry"),
                args: vec![String::from("mcp")],
            }
        );
        assert_eq!(parse_get(LOCAL, true), Registration::Folder);
        assert_eq!(
            parse_get("No MCP server named \"scry\".", false),
            Registration::None
        );
        // A description Scry cannot read is not taken for every project.
        assert_eq!(parse_get("scry:\n", true), Registration::Folder);
    }

    /// `claude` is found on the `PATH` first, in its order, and else in the
    /// folders its installers use; a folder holding a `claude` that cannot
    /// run is passed over.
    #[test]
    fn claude_is_found_on_the_path_then_where_installers_put_it() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let executable = |file: &Path| {
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(file, "#!/bin/sh\n").unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(file, std::fs::Permissions::from_mode(0o755)).unwrap();
            }
        };
        let first = dir.path().join("first");
        let second = dir.path().join("second");
        executable(&second.join("claude"));
        std::fs::create_dir_all(&first).unwrap();
        std::fs::create_dir_all(first.join("claude")).unwrap();
        let path = [first.clone(), second.clone()];
        assert_eq!(find_claude(&path, &home), Some(second.join("claude")));
        executable(&home.join(".claude/local/claude"));
        assert_eq!(
            find_claude(&[first], &home),
            Some(home.join(".claude/local/claude"))
        );
    }

    /// The settings offer to register when Claude Code does not run this
    /// app's `git-scry` in every project, and say so when it does; without
    /// `claude`, only the command to run later.
    #[test]
    fn the_state_follows_the_registration() {
        let ours = Path::new("/opt/homebrew/bin/git-scry");
        let registered = parse_get(USER, true);
        assert_eq!(state(None, ours), AgentState::NoClaude);
        assert_eq!(
            state(Some(&Registration::None), ours),
            AgentState::NotRegistered
        );
        assert_eq!(
            state(Some(&Registration::Folder), ours),
            AgentState::NotRegistered
        );
        assert_eq!(state(Some(&registered), ours), AgentState::Registered);
        assert_eq!(
            state(
                Some(&registered),
                Path::new("/Applications/Scry.app/Contents/Helpers/git-scry")
            ),
            AgentState::Other {
                command: String::from("/opt/homebrew/bin/git-scry mcp")
            }
        );
        let other_args = Registration::User {
            command: String::from("/opt/homebrew/bin/git-scry"),
            args: vec![String::from("serve")],
        };
        assert_eq!(
            state(Some(&other_args), ours),
            AgentState::Other {
                command: String::from("/opt/homebrew/bin/git-scry serve")
            }
        );
    }

    /// The server is registered for every project, under one name, with
    /// the path passed as one argument whatever it holds; the command to
    /// copy quotes it for the shell only when needed.
    #[test]
    fn the_commands_register_for_every_project() {
        let path = Path::new("/Users/me/Apps/My Scry.app/Contents/Helpers/git-scry");
        assert_eq!(
            add_arguments(path),
            [
                "mcp",
                "add",
                "--scope",
                "user",
                "scry",
                "--",
                "/Users/me/Apps/My Scry.app/Contents/Helpers/git-scry",
                "mcp"
            ]
        );
        assert_eq!(
            remove_arguments(),
            ["mcp", "remove", "--scope", "user", "scry"]
        );
        assert_eq!(get_arguments(), ["mcp", "get", "scry"]);
        assert_eq!(
            command(Path::new("/opt/homebrew/bin/git-scry")),
            "claude mcp add --scope user scry -- /opt/homebrew/bin/git-scry mcp"
        );
        assert_eq!(
            command(path),
            "claude mcp add --scope user scry -- '/Users/me/Apps/My Scry.app/Contents/Helpers/git-scry' mcp"
        );
        assert_eq!(shell_quoted("it's"), "'it'\\''s'");
    }
}
