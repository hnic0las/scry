//! Making Scry git's merge tool, in the user's global git configuration:
//! what the app's settings show and the button they offer.

use std::path::Path;

use crate::git::{Git, GitError};

/// The tool's name in git's configuration (`merge.tool`).
pub const TOOL: &str = "scry";

/// What git's global configuration says about the merge tool.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Setup {
    /// No merge tool set: `git mergetool` would pick one itself.
    NotSet,
    /// Another tool, such as `opendiff` or `vimdiff`.
    Other { tool: String },
    /// Scry; `current` when its command runs this app, not one moved or
    /// deleted since.
    Scry { current: bool },
}

/// The merge tool command that runs `cli` (`git-scry`) to open `app`, the
/// app bundle. Paths are absolute, so it does not depend on the `PATH` git
/// gets, and quoted for the shell git runs it in. `--` before the files:
/// git passes a file named `-dash.txt` as it is.
pub fn command(cli: &Path, app: &Path) -> String {
    format!(
        "{} resolve --app={} -- \"$BASE\" \"$LOCAL\" \"$REMOTE\" \"$MERGED\"",
        quote(&cli.to_string_lossy()),
        quote(&app.to_string_lossy())
    )
}

/// `text` in double quotes, with the characters the shell still reads
/// inside them escaped.
pub(crate) fn quote(text: &str) -> String {
    let mut quoted = String::from("\"");
    for character in text.chars() {
        if matches!(character, '"' | '\\' | '$' | '`') {
            quoted.push('\\');
        }
        quoted.push(character);
    }
    quoted.push('"');
    quoted
}

/// Reads the merge tool from git's global configuration; `command` is the
/// one this app would install.
pub fn setup(git: &dyn Git, command: &str) -> Result<Setup, GitError> {
    let tool = global(git, "merge.tool")?;
    Ok(match tool.as_deref() {
        None => Setup::NotSet,
        Some(TOOL) => Setup::Scry {
            current: global(git, "mergetool.scry.cmd")?.as_deref() == Some(command),
        },
        Some(other) => Setup::Other {
            tool: other.to_string(),
        },
    })
}

/// Where Scry keeps the merge tool it replaced, so `uninstall` puts it back.
const PREVIOUS: &str = "mergetool.scry.previousTool";

/// Makes Scry git's merge tool for every repository, running `command`.
/// Git trusts its exit code: 0 when the file is written, so git stages it.
///
/// The tool it replaces is kept for `uninstall`. `merge.tool` is written
/// last: a failure before leaves git with the tool it had, never naming a
/// Scry without a command.
pub fn install(git: &dyn Git, command: &str) -> Result<(), GitError> {
    if let Some(previous) = global(git, "merge.tool")?.filter(|tool| tool != TOOL) {
        git.run(&["config", "--global", PREVIOUS, &previous])?;
    }
    for (key, value) in [
        ("mergetool.scry.cmd", command),
        ("mergetool.scry.trustExitCode", "true"),
        ("merge.tool", TOOL),
    ] {
        git.run(&["config", "--global", key, value])?;
    }
    Ok(())
}

/// The merge tool Scry replaced, which `uninstall` puts back; `None` when
/// there was none.
pub fn previous_tool(git: &dyn Git) -> Result<Option<String>, GitError> {
    global(git, PREVIOUS)
}

/// Stops using Scry as git's merge tool: the tool it replaced is git's
/// again, or none, and Scry's settings are removed. A merge tool chosen
/// since, other than Scry, is left alone.
pub fn uninstall(git: &dyn Git) -> Result<(), GitError> {
    if global(git, "merge.tool")?.as_deref() == Some(TOOL) {
        match previous_tool(git)? {
            Some(previous) => git.run(&["config", "--global", "merge.tool", &previous])?,
            None => git.run(&["config", "--global", "--unset", "merge.tool"])?,
        };
    }
    // Exits with an error when the section is already gone: nothing to do.
    git.output(&["config", "--global", "--remove-section", "mergetool.scry"])?;
    Ok(())
}

/// Whether macOS runs the app from a randomised read-only copy, as it does
/// for an app opened from Downloads with its quarantine: a command naming
/// that copy breaks once the app quits.
pub fn translocated(app: &Path) -> bool {
    app.to_string_lossy().contains("/AppTranslocation/")
}

/// A global setting; `None` when unset (git exits with 1).
pub(crate) fn global(git: &dyn Git, key: &str) -> Result<Option<String>, GitError> {
    let args = ["config", "--global", "--get", key];
    let output = git.output(&args)?;
    match output.status {
        Some(0) => Ok(Some(
            String::from_utf8_lossy(&output.stdout).trim().to_string(),
        )),
        Some(1) => Ok(None),
        _ => Err(GitError::Failed {
            command: args.join(" "),
            stderr: output.stderr,
        }),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::cell::RefCell;
    use std::collections::BTreeMap;

    use super::*;
    use crate::git::GitOutput;

    /// Git's global configuration in memory: `git config --global` reads
    /// and writes it, so tests never touch the user's.
    #[derive(Default)]
    pub(crate) struct FakeConfig {
        pub(crate) values: RefCell<BTreeMap<String, String>>,
    }

    impl Git for FakeConfig {
        fn output(&self, args: &[&str]) -> Result<GitOutput, GitError> {
            let (status, stdout) = match args {
                ["config", "--global", "--get", key] => match self.values.borrow().get(*key) {
                    Some(value) => (0, format!("{value}\n")),
                    None => (1, String::new()),
                },
                ["config", "--global", "--unset", key] => {
                    let removed = self.values.borrow_mut().remove(*key);
                    (if removed.is_some() { 0 } else { 5 }, String::new())
                }
                ["config", "--global", "--remove-section", section] => {
                    let prefix = format!("{section}.");
                    let mut values = self.values.borrow_mut();
                    let before = values.len();
                    values.retain(|key, _| !key.starts_with(&prefix));
                    (if values.len() < before { 0 } else { 128 }, String::new())
                }
                ["config", "--global", key, value] => {
                    self.values
                        .borrow_mut()
                        .insert(key.to_string(), value.to_string());
                    (0, String::new())
                }
                _ => (128, String::new()),
            };
            Ok(GitOutput {
                status: Some(status),
                stdout: stdout.into_bytes(),
                stderr: String::new(),
            })
        }

        fn output_with_input(&self, args: &[&str], _: &[u8]) -> Result<GitOutput, GitError> {
            self.output(args)
        }

        fn output_with_env(
            &self,
            args: &[&str],
            _: &[(&str, &str)],
        ) -> Result<GitOutput, GitError> {
            self.output(args)
        }
    }

    /// Installing writes the three settings `git mergetool` needs, and the
    /// settings then say Scry is the merge tool, running this app.
    #[test]
    fn install_makes_scry_the_merge_tool() {
        let git = FakeConfig::default();
        let command = command(
            Path::new("/Applications/Scry.app/Contents/Helpers/git-scry"),
            Path::new("/Applications/Scry.app"),
        );
        assert_eq!(setup(&git, &command).unwrap(), Setup::NotSet);

        install(&git, &command).unwrap();

        let values = git.values.borrow().clone();
        assert_eq!(values["merge.tool"], "scry");
        assert_eq!(
            values["mergetool.scry.cmd"],
            "\"/Applications/Scry.app/Contents/Helpers/git-scry\" resolve \
             --app=\"/Applications/Scry.app\" -- \"$BASE\" \"$LOCAL\" \"$REMOTE\" \"$MERGED\""
        );
        assert_eq!(values["mergetool.scry.trustExitCode"], "true");
        assert_eq!(
            setup(&git, &command).unwrap(),
            Setup::Scry { current: true }
        );
    }

    /// Another tool is named, so the settings can say what Scry replaces;
    /// a Scry command for an app moved since is not current.
    #[test]
    fn setup_tells_other_tools_and_moved_apps() {
        let git = FakeConfig::default();
        git.values
            .borrow_mut()
            .insert(String::from("merge.tool"), String::from("opendiff"));
        assert_eq!(
            setup(&git, "anything").unwrap(),
            Setup::Other {
                tool: String::from("opendiff")
            }
        );

        let old = command(Path::new("/old/git-scry"), Path::new("/old/Scry.app"));
        install(&git, &old).unwrap();
        let new = command(Path::new("/new/git-scry"), Path::new("/new/Scry.app"));
        assert_eq!(setup(&git, &new).unwrap(), Setup::Scry { current: false });
    }

    /// Stopping puts back the tool Scry replaced and removes Scry's settings, so trying
    /// Scry can be undone; with no tool before, git has none again.
    #[test]
    fn uninstall_puts_back_the_tool_scry_replaced() {
        let git = FakeConfig::default();
        git.values
            .borrow_mut()
            .insert(String::from("merge.tool"), String::from("opendiff"));
        install(&git, "cmd").unwrap();
        assert_eq!(previous_tool(&git).unwrap().as_deref(), Some("opendiff"));
        // Installing again (an update) keeps the tool from before Scry.
        install(&git, "new cmd").unwrap();
        assert_eq!(previous_tool(&git).unwrap().as_deref(), Some("opendiff"));

        uninstall(&git).unwrap();
        let values = git.values.borrow().clone();
        assert_eq!(
            values.get("merge.tool").map(String::as_str),
            Some("opendiff")
        );
        assert!(!values.keys().any(|key| key.starts_with("mergetool.scry.")));

        let git = FakeConfig::default();
        install(&git, "cmd").unwrap();
        uninstall(&git).unwrap();
        assert!(git.values.borrow().is_empty());
        // Nothing left to remove is not an error.
        uninstall(&git).unwrap();
    }

    /// An app macOS runs from a translocated copy would install a path that vanishes
    /// once it quits.
    #[test]
    fn a_translocated_app_is_recognised() {
        assert!(translocated(Path::new(
            "/private/var/folders/x/T/AppTranslocation/1234/d/Scry.app"
        )));
        assert!(!translocated(Path::new("/Applications/Scry.app")));
    }

    /// A path with spaces or shell characters stays one word, taken as it
    /// is.
    #[test]
    fn paths_are_quoted_for_the_shell() {
        assert_eq!(quote("/Users/a b/Scry.app"), "\"/Users/a b/Scry.app\"");
        assert_eq!(
            quote("/tmp/$HOME\"`x`\\"),
            "\"/tmp/\\$HOME\\\"\\`x\\`\\\\\""
        );
    }
}
