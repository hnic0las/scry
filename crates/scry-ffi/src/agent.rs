//! Settings › Agent: registering Scry's MCP server in Claude Code for every
//! project, with the user's `claude` (`scry_core::agent_setup` says what to
//! run and reads its answers).

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use scry_core::agent_setup::{self, AgentState, Registration};
use scry_core::cli_link::{self, CommandState};
use scry_core::mergetool;

use crate::ScryError;
use crate::cli_command::{fallback_path, home, run, shell_path};

/// Where Claude Code stands with Scry's server.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum AgentSetupState {
    /// No `claude` on the user's `PATH`.
    NoClaude,
    /// Claude Code does not know Scry's server, or for one folder only.
    NotRegistered,
    /// Claude Code starts this app's `git-scry` in every project.
    Registered,
    /// Claude Code starts `command` instead.
    Other { command: String },
}

/// The settings' Agent section.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct AgentSetup {
    pub state: AgentSetupState,
    /// The command that registers the server, to copy into a terminal.
    pub command: String,
}

/// `claude mcp get` starts the server to check it: allow for a slow start.
const CLAUDE_TIMEOUT: Duration = Duration::from_secs(20);

/// The user's `PATH`, their `claude`, and the `git-scry` to register.
struct Found {
    path: Vec<PathBuf>,
    home: PathBuf,
    claude: Option<PathBuf>,
    /// The `git scry` link when the terminal runs the app's, since it
    /// follows the app's updates and moves; else the app's own.
    git_scry: PathBuf,
}

impl Found {
    /// Runs the user's shell to read its `PATH`, which can take a second.
    fn new(cli: &str) -> Found {
        let home = home();
        let path = shell_path().unwrap_or_else(fallback_path);
        let git_scry = match cli_link::state(Path::new(cli), &path, &home) {
            CommandState::Installed { path } => path,
            _ => PathBuf::from(cli),
        };
        let claude = agent_setup::find_claude(&path, &home);
        Found {
            path,
            home,
            claude,
            git_scry,
        }
    }

    /// Runs `claude` with `arguments`, from the home folder, with the
    /// user's `PATH` (Claude Code installed with npm needs `node` on it).
    fn claude(&self, arguments: &[String]) -> Result<(String, String), ScryError> {
        let Some(claude) = &self.claude else {
            return Err(ScryError::Refused {
                message: String::from(
                    "Claude Code is not on your terminal's PATH. Install it, then try again.",
                ),
            });
        };
        let mut command = Command::new(claude);
        command.args(arguments).current_dir(&self.home);
        if let Ok(path) = std::env::join_paths(&self.path) {
            command.env("PATH", path);
        }
        match run(command, CLAUDE_TIMEOUT) {
            Some((output, result)) if result.succeeded => Ok((output, result.error)),
            Some((output, result)) => Err(ScryError::Failed {
                message: format!(
                    "claude {} failed: {}",
                    arguments.join(" "),
                    first_line(&result.error)
                        .unwrap_or_else(|| first_line(&output).unwrap_or_default())
                ),
            }),
            None => Err(ScryError::Failed {
                message: format!("claude {} did not answer.", arguments.join(" ")),
            }),
        }
    }

    /// How Claude Code knows the server; `None` without `claude`.
    fn registration(&self) -> Option<Registration> {
        self.claude.as_ref()?;
        Some(match self.claude(&agent_setup::get_arguments()) {
            Ok((output, _)) => agent_setup::parse_get(&output, true),
            Err(_) => Registration::None,
        })
    }
}

/// Where Claude Code stands with Scry's server, for the app's `cli`. Runs
/// the user's shell and `claude`, which can take seconds: call it off the
/// main thread.
#[uniffi::export]
pub fn agent_setup(cli: String) -> AgentSetup {
    let found = Found::new(&cli);
    let state = match agent_setup::state(found.registration().as_ref(), &found.git_scry) {
        AgentState::NoClaude => AgentSetupState::NoClaude,
        AgentState::NotRegistered => AgentSetupState::NotRegistered,
        AgentState::Registered => AgentSetupState::Registered,
        AgentState::Other { command } => AgentSetupState::Other { command },
    };
    AgentSetup {
        state,
        command: agent_setup::command(&found.git_scry),
    }
}

/// Registers the app's `git-scry` (or the `git scry` link to it) in Claude
/// Code for every project, replacing another registration for every
/// project.
#[uniffi::export]
pub fn register_agent(cli: String) -> Result<(), ScryError> {
    let found = Found::new(&cli);
    if found.git_scry == Path::new(&cli) && mergetool::translocated(Path::new(&cli)) {
        return Err(ScryError::Refused {
            message: String::from(
                "macOS runs Scry from a temporary copy, which Claude Code would lose once Scry quits. Move Scry to the Applications folder, open it from there, then try again.",
            ),
        });
    }
    if let Some(Registration::User { .. }) = found.registration() {
        found.claude(&agent_setup::remove_arguments())?;
    }
    found.claude(&agent_setup::add_arguments(&found.git_scry))?;
    Ok(())
}

/// Removes Scry's server from Claude Code's projects.
#[uniffi::export]
pub fn unregister_agent(cli: String) -> Result<(), ScryError> {
    Found::new(&cli).claude(&agent_setup::remove_arguments())?;
    Ok(())
}

fn first_line(text: &str) -> Option<String> {
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(String::from)
}
