//! Installing the `git-scry` Claude Code plugin from the app's settings:
//! the plugin is the one way Scry's MCP server and its `/git-scry:resolve`
//! skill reach Claude Code.
//!
//! The one exception to "the app never runs the `claude` CLI" (vault
//! decision 0002, amended on 2026-11-17 and 2026-11-23): `claude plugin`
//! to list, add the marketplace, install, enable and uninstall Scry's
//! plugin, and `claude mcp get` and `remove --scope user` for the server
//! named `scry` an earlier Scry registered, on the user's click; and adding
//! to Claude Code's user settings the rules that let the plugin's read
//! tools, `explain` and `suggest` run without asking, so the background agent that
//! answers Scry's asks is never stopped by a question (amended again on
//! 2026-11-24). These only change Claude Code's configuration: nothing
//! here starts a session, sends a prompt or reads credentials.
//!
//! What to run and how to read its answer is decided here; running it is
//! the front end's, which knows the user's `PATH`.

use std::path::{Path, PathBuf};

use crate::cli_link;

/// The plugin, as `<plugin>@<marketplace>`.
pub const PLUGIN: &str = "git-scry@scry";
/// The marketplace the plugin is published in, as Claude Code names it.
pub const MARKETPLACE: &str = "scry";
/// Where the marketplace is: Scry's GitHub repository.
pub const MARKETPLACE_SOURCE: &str = "hnic0las/scry";
/// The server an earlier Scry registered for every project (`claude mcp
/// add --scope user scry`), which Claude Code would show next to the
/// plugin's.
pub const OLD_SERVER: &str = "scry";

/// The plugin's tools allowed without asking: the three that read, and
/// `explain` and `suggest`, which write only Scry's answers under the git
/// folder. `resolve`, which writes the user's files, still asks.
pub const ALLOWED_TOOLS: [&str; 5] = [
    "mcp__plugin_git-scry_scry__overview",
    "mcp__plugin_git-scry_scry__conflicts",
    "mcp__plugin_git-scry_scry__conflict",
    "mcp__plugin_git-scry_scry__explain",
    "mcp__plugin_git-scry_scry__suggest",
];

/// Rules that allow every tool of the plugin's server, which the user may
/// have written instead.
const SERVER_RULES: [&str; 2] = ["mcp__plugin_git-scry_scry", "mcp__plugin_git-scry_scry__*"];

/// Claude Code's user settings.
pub fn settings_file(home: &Path) -> PathBuf {
    home.join(".claude").join("settings.json")
}

/// The rules of `permissions.allow` in Claude Code's settings.
fn allow_rules(settings: &serde_json::Value) -> Vec<&str> {
    settings["permissions"]["allow"]
        .as_array()
        .map(|rules| rules.iter().filter_map(serde_json::Value::as_str).collect())
        .unwrap_or_default()
}

/// Whether Claude Code's user settings (`settings`, the file's text, `None`
/// when there is none) let the tools of [`ALLOWED_TOOLS`] run without
/// asking: each named, or the whole server.
pub fn tools_allowed(settings: Option<&str>) -> bool {
    let Some(settings) = settings.and_then(|text| serde_json::from_str(text).ok()) else {
        return false;
    };
    let rules = allow_rules(&settings);
    rules.iter().any(|rule| SERVER_RULES.contains(rule))
        || ALLOWED_TOOLS.iter().all(|tool| rules.contains(tool))
}

/// Claude Code's user settings with the rules of [`ALLOWED_TOOLS`] added
/// to `permissions.allow`, everything else kept in its order; `None` when
/// they are all there. An error when the file is not a JSON object, which
/// Scry does not rewrite.
pub fn allowing(settings: Option<&str>) -> Result<Option<String>, String> {
    let mut value = parsed(settings)?;
    let rules: Vec<String> = allow_rules(&value)
        .iter()
        .map(|rule| rule.to_string())
        .collect();
    let missing: Vec<&str> = ALLOWED_TOOLS
        .into_iter()
        .filter(|tool| !rules.iter().any(|rule| rule == tool))
        .collect();
    if missing.is_empty() {
        return Ok(None);
    }
    let permissions = value
        .as_object_mut()
        .expect("parsed checks it is an object")
        .entry("permissions")
        .or_insert_with(|| serde_json::json!({}));
    let Some(permissions) = permissions.as_object_mut() else {
        return Err(String::from("its permissions are not an object"));
    };
    let allow = permissions
        .entry("allow")
        .or_insert_with(|| serde_json::json!([]));
    let Some(allow) = allow.as_array_mut() else {
        return Err(String::from("its permissions.allow is not a list"));
    };
    allow.extend(missing.into_iter().map(serde_json::Value::from));
    pretty(&value).map(Some)
}

/// Claude Code's user settings without the rules of [`ALLOWED_TOOLS`];
/// `None` when there are none to remove. Rules the user wrote another way
/// are theirs, and kept.
pub fn disallowing(settings: Option<&str>) -> Result<Option<String>, String> {
    if settings.is_none() {
        return Ok(None);
    }
    let mut value = parsed(settings)?;
    let Some(allow) = value
        .get_mut("permissions")
        .and_then(|permissions| permissions.get_mut("allow"))
        .and_then(serde_json::Value::as_array_mut)
    else {
        return Ok(None);
    };
    let before = allow.len();
    allow.retain(|rule| {
        !rule
            .as_str()
            .is_some_and(|rule| ALLOWED_TOOLS.contains(&rule))
    });
    if allow.len() == before {
        return Ok(None);
    }
    pretty(&value).map(Some)
}

/// The settings as a JSON object: empty when there is no file.
fn parsed(settings: Option<&str>) -> Result<serde_json::Value, String> {
    let value: serde_json::Value = match settings {
        None => serde_json::json!({}),
        Some(text) if text.trim().is_empty() => serde_json::json!({}),
        Some(text) => {
            serde_json::from_str(text).map_err(|error| format!("it is not JSON ({error})"))?
        }
    };
    match value.is_object() {
        true => Ok(value),
        false => Err(String::from("it is not a JSON object")),
    }
}

/// The settings written as Claude Code writes them: two spaces, a final
/// line break.
fn pretty(value: &serde_json::Value) -> Result<String, String> {
    serde_json::to_string_pretty(value)
        .map(|text| text + "\n")
        .map_err(|error| error.to_string())
}

/// Whether Claude Code has the plugin, for every project of the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Plugin {
    Missing,
    Disabled,
    Enabled,
}

/// What `claude plugin list --json` printed: the user-scope entry for
/// [`PLUGIN`]. `None` when it cannot be read.
pub fn parse_plugins(json: &str) -> Option<Plugin> {
    let plugins: Vec<serde_json::Value> = serde_json::from_str(json).ok()?;
    let ours = plugins.iter().find(|plugin| {
        plugin["id"].as_str() == Some(PLUGIN) && plugin["scope"].as_str() == Some("user")
    });
    Some(match ours {
        None => Plugin::Missing,
        Some(plugin) if plugin["enabled"].as_bool() == Some(false) => Plugin::Disabled,
        Some(_) => Plugin::Enabled,
    })
}

/// The version of Scry's plugin `claude plugin list --json` lists for the
/// user; `None` when it lists none, or no version.
pub fn parse_plugin_version(json: &str) -> Option<String> {
    let plugins: Vec<serde_json::Value> = serde_json::from_str(json).ok()?;
    plugins.iter().find(|plugin| {
        plugin["id"].as_str() == Some(PLUGIN) && plugin["scope"].as_str() == Some("user")
    })?["version"]
        .as_str()
        .map(String::from)
}

/// Whether `claude plugin marketplace list --json` lists [`MARKETPLACE`],
/// from GitHub or, while developing, from a folder.
pub fn parse_marketplaces(json: &str) -> bool {
    serde_json::from_str::<Vec<serde_json::Value>>(json).is_ok_and(|marketplaces| {
        marketplaces
            .iter()
            .any(|marketplace| marketplace["name"].as_str() == Some(MARKETPLACE))
    })
}

/// Whether `claude mcp get scry` described a server registered for every
/// project (`Scope: User config …`): one an earlier Scry added. A server
/// of one folder is the user's own, and left alone.
pub fn parse_old_server(output: &str, succeeded: bool) -> bool {
    succeeded
        && output.lines().any(|line| {
            line.trim()
                .strip_prefix("Scope:")
                .is_some_and(|scope| scope.trim().starts_with("User"))
        })
}

/// Where the settings stand, and what the button does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentState {
    /// No `claude` on the user's `PATH`: the commands are shown to run once
    /// Claude Code is installed.
    NoClaude,
    /// Claude Code does not have the plugin: the button installs it.
    NotInstalled,
    /// The plugin is installed and turned off: the button turns it on.
    Disabled,
    /// Claude Code has the plugin.
    Installed,
}

/// What the settings show for `plugin`, `None` without `claude` or when
/// its answer cannot be read.
pub fn state(plugin: Option<Plugin>) -> AgentState {
    match plugin {
        None => AgentState::NoClaude,
        Some(Plugin::Missing) => AgentState::NotInstalled,
        Some(Plugin::Disabled) => AgentState::Disabled,
        Some(Plugin::Enabled) => AgentState::Installed,
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

/// The `git-scry` the plugin's launcher (`plugins/git-scry/bin/git-scry-mcp`)
/// starts, looking where it looks, in its order: on the `PATH`, then in
/// Scry in `/Applications` and `~/Applications`. `None` when it finds no
/// Scry, and Claude Code would show the plugin's server failing.
pub fn plugin_scry(path: &[PathBuf], home: &Path) -> Option<PathBuf> {
    let helper = Path::new("Scry.app/Contents/Helpers/git-scry");
    path.iter()
        .map(|folder| folder.join("git-scry"))
        .chain([
            Path::new("/Applications").join(helper),
            home.join("Applications").join(helper),
        ])
        .find(|file| cli_link::is_executable(file))
}

fn arguments(words: &[&str]) -> Vec<String> {
    words.iter().map(|word| word.to_string()).collect()
}

/// The arguments of `claude` that list the installed plugins.
pub fn list_arguments() -> Vec<String> {
    arguments(&["plugin", "list", "--json"])
}

/// The arguments of `claude` that list the marketplaces.
pub fn marketplaces_arguments() -> Vec<String> {
    arguments(&["plugin", "marketplace", "list", "--json"])
}

/// The arguments of `claude` that add Scry's marketplace, for the user.
pub fn add_marketplace_arguments() -> Vec<String> {
    arguments(&["plugin", "marketplace", "add", MARKETPLACE_SOURCE])
}

/// The arguments of `claude` that install the plugin for every project.
pub fn install_arguments() -> Vec<String> {
    arguments(&["plugin", "install", "--scope", "user", PLUGIN])
}

/// The arguments of `claude` that turn the installed plugin on.
pub fn enable_arguments() -> Vec<String> {
    arguments(&["plugin", "enable", "--scope", "user", PLUGIN])
}

/// The arguments of `claude` that uninstall the plugin.
pub fn uninstall_arguments() -> Vec<String> {
    arguments(&["plugin", "uninstall", "--scope", "user", PLUGIN])
}

/// The arguments of `claude` that describe the server an earlier Scry
/// registered.
pub fn old_server_arguments() -> Vec<String> {
    arguments(&["mcp", "get", OLD_SERVER])
}

/// The arguments of `claude` that remove it.
pub fn remove_old_server_arguments() -> Vec<String> {
    arguments(&["mcp", "remove", "--scope", "user", OLD_SERVER])
}

/// The commands to copy into a terminal, which do what the button does:
/// one per line, which a shell runs in order.
pub fn command() -> String {
    format!("claude plugin marketplace add {MARKETPLACE_SOURCE}\nclaude plugin install {PLUGIN}")
}

#[cfg(test)]
mod tests {
    use super::*;

    const USER: &str = "scry:\n  Scope: User config (available in all your projects)\n  Status: ✔ Connected\n  Type: stdio\n  Command: /opt/homebrew/bin/git-scry\n  Args: mcp\n  Environment:\n\nTo remove this server, run: claude mcp remove scry -s user\n";
    const LOCAL: &str = "scry:\n  Scope: Local config (private to you in this project)\n  Status: ✔ Connected\n  Type: stdio\n  Command: /tmp/git-scry\n  Args: mcp\n";

    fn executable(file: &Path) {
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, "#!/bin/sh\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(file, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
    }

    /// The plugin counts when installed for the user, turned on or off;
    /// another plugin, or Scry's for one project, does not; an answer that
    /// is not the list is not read as "missing".
    #[test]
    fn claude_s_plugins_are_read() {
        let entry = |id: &str, scope: &str, enabled: bool| {
            format!(r#"{{"id":"{id}","version":"0.2.0","scope":"{scope}","enabled":{enabled}}}"#)
        };
        let list = |entries: &[String]| format!("[{}]", entries.join(","));
        assert_eq!(parse_plugins("[]"), Some(Plugin::Missing));
        assert_eq!(
            parse_plugins(&list(&[
                entry("saymock@saymock", "user", true),
                entry("git-scry@scry", "user", true)
            ])),
            Some(Plugin::Enabled)
        );
        assert_eq!(
            parse_plugins(&list(&[entry("git-scry@scry", "user", false)])),
            Some(Plugin::Disabled)
        );
        assert_eq!(
            parse_plugins(&list(&[entry("git-scry@scry", "project", true)])),
            Some(Plugin::Missing)
        );
        assert_eq!(
            parse_plugins(&list(&[entry("git-scry@elsewhere", "user", true)])),
            Some(Plugin::Missing)
        );
        assert_eq!(parse_plugins("Unknown command"), None);

        assert_eq!(
            parse_plugin_version(&list(&[
                entry("git-scry@scry", "project", true),
                entry("git-scry@scry", "user", false)
            ]))
            .as_deref(),
            Some("0.2.0")
        );
        assert_eq!(
            parse_plugin_version(r#"[{"id":"git-scry@scry","scope":"user"}]"#),
            None
        );
        // Only the user's install of Scry's plugin: not another plugin, not
        // Scry's for one project.
        assert_eq!(
            parse_plugin_version(
                r#"[{"id":"saymock@saymock","scope":"user","version":"9.0"},
                    {"id":"git-scry@scry","scope":"project","version":"8.0"},
                    {"id":"git-scry@scry","scope":"user","version":"0.3.0"}]"#
            )
            .as_deref(),
            Some("0.3.0")
        );
        assert_eq!(parse_plugin_version("[]"), None);
        assert_eq!(parse_plugin_version("Unknown command"), None);
    }

    /// Scry's marketplace is found by its name, from GitHub or a folder.
    #[test]
    fn claude_s_marketplaces_are_read() {
        assert!(parse_marketplaces(
            r#"[{"name":"claude-plugins-official","source":"github"},{"name":"scry","source":"directory","path":"/w/scry"}]"#
        ));
        assert!(!parse_marketplaces(
            r#"[{"name":"saymock","source":"github"}]"#
        ));
        assert!(!parse_marketplaces("[]"));
        assert!(!parse_marketplaces("error"));
    }

    /// Only a `scry` server registered for every project is an earlier
    /// Scry's; one folder's, or none, is not.
    #[test]
    fn the_old_server_is_the_user_s() {
        assert!(parse_old_server(USER, true));
        assert!(!parse_old_server(LOCAL, true));
        assert!(!parse_old_server("No MCP server named \"scry\".", false));
        assert!(!parse_old_server(USER, false));
    }

    /// The settings follow the plugin; without `claude`, only the commands
    /// to run later.
    #[test]
    fn the_state_follows_the_plugin() {
        assert_eq!(state(None), AgentState::NoClaude);
        assert_eq!(state(Some(Plugin::Missing)), AgentState::NotInstalled);
        assert_eq!(state(Some(Plugin::Disabled)), AgentState::Disabled);
        assert_eq!(state(Some(Plugin::Enabled)), AgentState::Installed);
    }

    /// `claude` is found on the `PATH` first, in its order, and else in the
    /// folders its installers use; a folder holding a `claude` that cannot
    /// run is passed over.
    #[test]
    fn claude_is_found_on_the_path_then_where_installers_put_it() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let first = dir.path().join("first");
        let second = dir.path().join("second");
        executable(&second.join("claude"));
        std::fs::create_dir_all(first.join("claude")).unwrap();
        let path = [first.clone(), second.clone()];
        assert_eq!(find_claude(&path, &home), Some(second.join("claude")));
        executable(&home.join(".claude/local/claude"));
        assert_eq!(
            find_claude(&[first], &home),
            Some(home.join(".claude/local/claude"))
        );
    }

    /// The plugin's launcher runs the `git-scry` on the `PATH` first, else
    /// the Scry in the user's Applications folder.
    #[test]
    fn the_plugin_finds_scry_on_the_path_then_in_applications() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let bin = dir.path().join("bin");
        let empty = dir.path().join("empty");
        std::fs::create_dir_all(&empty).unwrap();
        let installed = Path::new("/Applications/Scry.app/Contents/Helpers/git-scry");
        let in_applications = cli_link::is_executable(installed).then(|| installed.to_path_buf());
        assert_eq!(
            plugin_scry(std::slice::from_ref(&empty), &home),
            in_applications
        );
        executable(&bin.join("git-scry"));
        assert_eq!(
            plugin_scry(&[empty.clone(), bin.clone()], &home),
            Some(bin.join("git-scry"))
        );
        if in_applications.is_none() {
            let users = home.join("Applications/Scry.app/Contents/Helpers/git-scry");
            executable(&users);
            assert_eq!(plugin_scry(&[empty], &home), Some(users));
        }
    }

    /// The rules are added to what the settings hold, in its order, once;
    /// a file that is no JSON object is not rewritten; removing them keeps
    /// the user's other rules, including a rule for the whole server.
    #[test]
    fn the_tools_are_allowed_in_claude_s_settings() {
        assert!(!tools_allowed(None));
        assert!(!tools_allowed(Some("not json")));
        let added = allowing(None).unwrap().unwrap();
        assert!(tools_allowed(Some(&added)));
        assert!(added.ends_with("]\n  }\n}\n"), "{added}");
        assert_eq!(allowing(Some(&added)).unwrap(), None);

        let theirs = r#"{"model":"opus","permissions":{"allow":["Bash(ls *)","mcp__plugin_git-scry_scry__conflict"],"deny":[]},"theme":"dark"}"#;
        assert!(!tools_allowed(Some(theirs)));
        let added = allowing(Some(theirs)).unwrap().unwrap();
        let value: serde_json::Value = serde_json::from_str(&added).unwrap();
        assert_eq!(
            value.as_object().unwrap().keys().collect::<Vec<_>>(),
            ["model", "permissions", "theme"]
        );
        assert_eq!(
            value["permissions"]["allow"],
            serde_json::json!([
                "Bash(ls *)",
                "mcp__plugin_git-scry_scry__conflict",
                "mcp__plugin_git-scry_scry__overview",
                "mcp__plugin_git-scry_scry__conflicts",
                "mcp__plugin_git-scry_scry__explain",
                "mcp__plugin_git-scry_scry__suggest"
            ])
        );
        assert_eq!(value["permissions"]["deny"], serde_json::json!([]));
        let removed = disallowing(Some(&added)).unwrap().unwrap();
        let value: serde_json::Value = serde_json::from_str(&removed).unwrap();
        assert_eq!(
            value["permissions"]["allow"],
            serde_json::json!(["Bash(ls *)"])
        );
        assert_eq!(disallowing(Some(&removed)).unwrap(), None);
        assert_eq!(disallowing(None).unwrap(), None);
        assert_eq!(disallowing(Some(r#"{"model":"opus"}"#)).unwrap(), None);

        for server in [
            r#"{"permissions":{"allow":["mcp__plugin_git-scry_scry"]}}"#,
            r#"{"permissions":{"allow":["mcp__plugin_git-scry_scry__*"]}}"#,
        ] {
            assert!(tools_allowed(Some(server)));
            assert_eq!(disallowing(Some(server)).unwrap(), None);
        }
        assert_eq!(allowing(Some("  ")).unwrap(), allowing(None).unwrap());
        assert!(
            allowing(Some("[1]"))
                .unwrap_err()
                .contains("not a JSON object")
        );
        assert!(
            allowing(Some("{,}"))
                .unwrap_err()
                .starts_with("it is not JSON")
        );
        assert!(allowing(Some(r#"{"permissions":[]}"#)).is_err());
        assert!(allowing(Some(r#"{"permissions":{"allow":{}}}"#)).is_err());
        assert!(disallowing(Some("{,}")).is_err());
        assert_eq!(
            settings_file(Path::new("/Users/me")),
            Path::new("/Users/me/.claude/settings.json")
        );
    }

    /// The plugin is installed from Scry's marketplace, for every project;
    /// the commands to copy do the same.
    #[test]
    fn the_commands_install_the_plugin_for_every_project() {
        assert_eq!(list_arguments(), ["plugin", "list", "--json"]);
        assert_eq!(
            marketplaces_arguments(),
            ["plugin", "marketplace", "list", "--json"]
        );
        assert_eq!(
            add_marketplace_arguments(),
            ["plugin", "marketplace", "add", "hnic0las/scry"]
        );
        assert_eq!(
            install_arguments(),
            ["plugin", "install", "--scope", "user", "git-scry@scry"]
        );
        assert_eq!(
            enable_arguments(),
            ["plugin", "enable", "--scope", "user", "git-scry@scry"]
        );
        assert_eq!(
            uninstall_arguments(),
            ["plugin", "uninstall", "--scope", "user", "git-scry@scry"]
        );
        assert_eq!(old_server_arguments(), ["mcp", "get", "scry"]);
        assert_eq!(
            remove_old_server_arguments(),
            ["mcp", "remove", "--scope", "user", "scry"]
        );
        assert_eq!(
            command(),
            "claude plugin marketplace add hnic0las/scry\nclaude plugin install git-scry@scry"
        );
    }
}
