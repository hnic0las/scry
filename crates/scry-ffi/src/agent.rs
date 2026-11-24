//! Settings › Agent: installing the `git-scry` plugin in Claude Code, with
//! the user's `claude` (`scry_core::agent_setup` says what to run and reads
//! its answers).

use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

use scry_core::agent_setup::{self, AgentState, Plugin};

use crate::ScryError;
use crate::cli_command::{fallback_path, home, run, shell_path};

/// Where Claude Code stands with Scry's plugin.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum AgentSetupState {
    /// No `claude` on the user's `PATH`.
    NoClaude,
    /// Claude Code does not have the plugin.
    NotInstalled,
    /// The plugin is installed and turned off.
    Disabled,
    /// Claude Code has the plugin, for every project.
    Installed,
}

/// The settings' Agent section.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct AgentSetup {
    pub state: AgentSetupState,
    /// The commands that install the plugin, to copy into a terminal.
    pub command: String,
    /// An earlier Scry registered its server for every project, which
    /// Claude Code shows next to the plugin's.
    pub old_server: bool,
    /// The `git-scry` the plugin starts; `None` when it would find no Scry.
    pub plugin_scry: Option<String>,
}

/// Listing, enabling or removing: `claude mcp get` starts the server to
/// check it, allow for a slow start.
const CLAUDE_TIMEOUT: Duration = Duration::from_secs(20);
/// Adding the marketplace clones Scry's repository, and installing copies
/// the plugin.
const INSTALL_TIMEOUT: Duration = Duration::from_secs(120);

/// The user's `PATH` and their `claude`.
struct Found {
    path: Vec<PathBuf>,
    home: PathBuf,
    claude: Option<PathBuf>,
}

impl Found {
    /// Runs the user's shell to read its `PATH`, which can take a second.
    fn new() -> Found {
        let home = home();
        let path = shell_path().unwrap_or_else(fallback_path);
        let claude = agent_setup::find_claude(&path, &home);
        Found { path, home, claude }
    }

    /// Runs `claude` with `arguments`, from the home folder, with the
    /// user's `PATH` (Claude Code installed with npm needs `node` on it).
    fn claude(&self, arguments: &[String], timeout: Duration) -> Result<String, ScryError> {
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
        match run(command, timeout) {
            Some((output, result)) if result.succeeded => Ok(output),
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

    /// Whether Claude Code has the plugin; `None` without `claude`, or when
    /// its answer cannot be read.
    fn plugin(&self) -> Option<Plugin> {
        self.claude.as_ref()?;
        let output = self
            .claude(&agent_setup::list_arguments(), CLAUDE_TIMEOUT)
            .ok()?;
        agent_setup::parse_plugins(&output)
    }

    /// Whether an earlier Scry's server is registered for every project.
    fn old_server(&self) -> bool {
        self.claude.is_some()
            && self
                .claude(&agent_setup::old_server_arguments(), CLAUDE_TIMEOUT)
                .is_ok_and(|output| agent_setup::parse_old_server(&output, true))
    }

    /// Removes an earlier Scry's server, when there is one.
    fn remove_old_server(&self) -> Result<(), ScryError> {
        if self.old_server() {
            self.claude(&agent_setup::remove_old_server_arguments(), CLAUDE_TIMEOUT)?;
        }
        Ok(())
    }
}

/// Where Claude Code stands with Scry's plugin. Runs the user's shell and
/// `claude`, which can take seconds: call it off the main thread.
#[uniffi::export]
pub fn agent_setup() -> AgentSetup {
    let found = Found::new();
    let state = match agent_setup::state(found.plugin()) {
        AgentState::NoClaude => AgentSetupState::NoClaude,
        AgentState::NotInstalled => AgentSetupState::NotInstalled,
        AgentState::Disabled => AgentSetupState::Disabled,
        AgentState::Installed => AgentSetupState::Installed,
    };
    AgentSetup {
        state,
        command: agent_setup::command(),
        old_server: found.old_server(),
        plugin_scry: agent_setup::plugin_scry(&found.path, &found.home)
            .map(|path| path.display().to_string()),
    }
}

/// Installs the plugin for every project, adding Scry's marketplace first
/// when Claude Code does not have it, or turns it on when it is installed
/// and off; then removes the server an earlier Scry registered, so Claude
/// Code shows Scry once.
#[uniffi::export]
pub fn install_plugin() -> Result<(), ScryError> {
    let found = Found::new();
    match found.plugin() {
        Some(Plugin::Enabled) => {}
        Some(Plugin::Disabled) => {
            found.claude(&agent_setup::enable_arguments(), CLAUDE_TIMEOUT)?;
        }
        _ => {
            let marketplaces =
                found.claude(&agent_setup::marketplaces_arguments(), CLAUDE_TIMEOUT)?;
            if !agent_setup::parse_marketplaces(&marketplaces) {
                found.claude(&agent_setup::add_marketplace_arguments(), INSTALL_TIMEOUT)?;
            }
            found.claude(&agent_setup::install_arguments(), INSTALL_TIMEOUT)?;
        }
    }
    found.remove_old_server()
}

/// Uninstalls the plugin. Scry's marketplace stays, as other marketplaces
/// do once added.
#[uniffi::export]
pub fn uninstall_plugin() -> Result<(), ScryError> {
    Found::new().claude(&agent_setup::uninstall_arguments(), CLAUDE_TIMEOUT)?;
    Ok(())
}

/// Removes the server an earlier Scry registered for every project.
#[uniffi::export]
pub fn remove_old_server() -> Result<(), ScryError> {
    Found::new().remove_old_server()
}

fn first_line(text: &str) -> Option<String> {
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(String::from)
}
