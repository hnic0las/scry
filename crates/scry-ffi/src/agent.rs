//! Settings › Agent: installing the `git-scry` plugin in Claude Code, and
//! allowing its tools in Claude Code's user settings, with the user's
//! `claude` (`scry_core::agent_setup` says what to run and reads
//! its answers).

use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

use scry_core::agent_setup::{self, AgentState, Plugin};
use scry_core::session::write_atomically;

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
    /// Claude Code's settings let the plugin's read tools and `explain`
    /// run without asking, so Explain is answered without a question.
    pub tools_allowed: bool,
    /// The plugin's version, once installed.
    pub plugin_version: Option<String>,
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
                    "claude {} failed:\n{}",
                    arguments.join(" "),
                    answer(&result.error).unwrap_or_else(|| answer(&output).unwrap_or_default())
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
        agent_setup::parse_plugins(&self.plugins()?)
    }

    /// `claude plugin list --json`, as it answered.
    fn plugins(&self) -> Option<String> {
        self.claude.as_ref()?;
        self.claude(&agent_setup::list_arguments(), CLAUDE_TIMEOUT)
            .ok()
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

impl Found {
    /// Claude Code's user settings, as `change` rewrites them; nothing
    /// written when it changes nothing.
    fn change_settings(
        &self,
        change: fn(Option<&str>) -> Result<Option<String>, String>,
    ) -> Result<(), ScryError> {
        let file = agent_setup::settings_file(&self.home);
        let text = match std::fs::read_to_string(&file) {
            Ok(text) => Some(text),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => {
                return Err(ScryError::Failed {
                    message: format!("{} could not be read: {error}", file.display()),
                });
            }
        };
        let changed = change(text.as_deref()).map_err(|why| ScryError::Refused {
            message: format!(
                "Scry did not change {}: {why}. Allow Scry's tools in Claude Code instead (\"don't ask again\").",
                file.display()
            ),
        })?;
        let Some(changed) = changed else {
            return Ok(());
        };
        if let Some(folder) = file.parent() {
            std::fs::create_dir_all(folder).map_err(|error| ScryError::Failed {
                message: format!("{} could not be made: {error}", folder.display()),
            })?;
        }
        write_atomically(&file, &changed).map_err(|error| ScryError::Failed {
            message: format!("{} could not be written: {error}", file.display()),
        })
    }

    fn tools_allowed(&self) -> bool {
        let text = std::fs::read_to_string(agent_setup::settings_file(&self.home)).ok();
        agent_setup::tools_allowed(text.as_deref())
    }
}

/// Where Claude Code stands with Scry's plugin. Runs the user's shell and
/// `claude`, which can take seconds: call it off the main thread.
#[uniffi::export]
pub fn agent_setup() -> AgentSetup {
    let found = Found::new();
    let plugins = found.plugins();
    let state = match agent_setup::state(plugins.as_deref().and_then(agent_setup::parse_plugins)) {
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
        tools_allowed: found.tools_allowed(),
        plugin_version: plugins
            .as_deref()
            .and_then(agent_setup::parse_plugin_version),
    }
}

/// Installs the plugin for every project, adding Scry's marketplace first
/// when Claude Code does not have it, or turns it on when it is installed
/// and off; then removes the server an earlier Scry registered, so Claude
/// Code shows Scry once, and allows the plugin's read tools and `explain`
/// in Claude Code's user settings.
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
    found.remove_old_server()?;
    found.change_settings(agent_setup::allowing)
}

/// Allows the plugin's read tools and `explain` in Claude Code's user
/// settings, for a plugin installed without Scry.
#[uniffi::export]
pub fn allow_plugin_tools() -> Result<(), ScryError> {
    Found::new().change_settings(agent_setup::allowing)
}

/// Uninstalls the plugin and removes the rules Scry added for its tools.
/// Scry's marketplace stays, as other marketplaces do once added.
#[uniffi::export]
pub fn uninstall_plugin() -> Result<(), ScryError> {
    let found = Found::new();
    found.claude(&agent_setup::uninstall_arguments(), CLAUDE_TIMEOUT)?;
    found.change_settings(agent_setup::disallowing)
}

/// Removes the server an earlier Scry registered for every project.
#[uniffi::export]
pub fn remove_old_server() -> Result<(), ScryError> {
    Found::new().remove_old_server()
}

/// What `claude` said, as Settings shows it: its lines, the empty ones at
/// either end left out, at most 40 of them.
fn answer(text: &str) -> Option<String> {
    let lines: Vec<&str> = text.trim().lines().map(str::trim_end).take(40).collect();
    let text = lines.join("\n");
    (!text.is_empty()).then_some(text)
}

/// Who watches a repository, for Settings › Agent.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct RepositoryWatch {
    /// The repository's folder name.
    pub name: String,
    /// When each Claude Code session watching it started, in seconds
    /// since 1970, earliest first.
    pub sessions: Vec<u64>,
}

/// Who watches the repository holding `path`, a file or a folder in it;
/// `None` outside a repository.
#[uniffi::export]
pub fn repository_watch(path: String) -> Option<RepositoryWatch> {
    let path = PathBuf::from(path);
    let folder = if path.is_dir() {
        path
    } else {
        path.parent()?.to_path_buf()
    };
    let git = scry_core::git::GitCli::discover(folder);
    let root = scry_core::overview::root(&git).ok()?;
    let git_dir = scry_core::asks::git_dir(&git).ok()?;
    Some(RepositoryWatch {
        name: root
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default(),
        sessions: scry_core::asks::sessions(&git_dir, std::time::SystemTime::now())
            .into_iter()
            .map(|started| {
                started
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |since| since.as_secs())
            })
            .collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Claude's answer is shown whole, its blank ends trimmed, cut at 40
    /// lines; an empty one is none.
    #[test]
    fn claude_s_answer_is_kept_whole() {
        assert_eq!(
            answer("\nError: failed to add marketplace\nfatal: unable to access  \n\n").as_deref(),
            Some("Error: failed to add marketplace\nfatal: unable to access")
        );
        assert_eq!(answer(" \n "), None);
        let long: String = (0..50).map(|line| format!("{line}\n")).collect();
        assert_eq!(answer(&long).unwrap().lines().count(), 40);
    }

    /// A repository's watchers are read from a file or a folder in it;
    /// outside a repository there is none.
    #[test]
    fn a_repository_s_watchers_are_read() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("offline-sync");
        std::fs::create_dir(&repo).unwrap();
        assert!(repository_watch(repo.display().to_string()).is_none());
        assert!(
            std::process::Command::new("git")
                .args(["init", "--quiet"])
                .current_dir(&repo)
                .status()
                .unwrap()
                .success()
        );
        let watch = repository_watch(repo.display().to_string()).unwrap();
        assert_eq!(watch.name, "offline-sync");
        assert!(watch.sessions.is_empty());
        let _presence = scry_core::asks::Presence::start(&repo.join(".git"), "1").unwrap();
        let file = repo.join("a.txt");
        std::fs::write(&file, "a").unwrap();
        assert_eq!(
            repository_watch(file.display().to_string())
                .unwrap()
                .sessions
                .len(),
            1
        );
    }
}
