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
/// gets, and quoted for the shell git runs it in.
pub fn command(cli: &Path, app: &Path) -> String {
    format!(
        "{} resolve --app={} \"$BASE\" \"$LOCAL\" \"$REMOTE\" \"$MERGED\"",
        quote(&cli.to_string_lossy()),
        quote(&app.to_string_lossy())
    )
}

/// `text` in double quotes, with the characters the shell still reads
/// inside them escaped.
fn quote(text: &str) -> String {
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

/// Makes Scry git's merge tool for every repository, running `command`.
/// Git trusts its exit code: 0 when the file is written, so git stages it.
pub fn install(git: &dyn Git, command: &str) -> Result<(), GitError> {
    for (key, value) in [
        ("merge.tool", TOOL),
        ("mergetool.scry.cmd", command),
        ("mergetool.scry.trustExitCode", "true"),
    ] {
        git.run(&["config", "--global", key, value])?;
    }
    Ok(())
}

/// A global setting; `None` when unset (git exits with 1).
fn global(git: &dyn Git, key: &str) -> Result<Option<String>, GitError> {
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
mod tests {
    use std::cell::RefCell;
    use std::collections::BTreeMap;

    use super::*;
    use crate::git::GitOutput;

    /// Git's global configuration in memory: `git config --global` reads
    /// and writes it, so tests never touch the user's.
    #[derive(Default)]
    struct FakeConfig {
        values: RefCell<BTreeMap<String, String>>,
    }

    impl Git for FakeConfig {
        fn output(&self, args: &[&str]) -> Result<GitOutput, GitError> {
            let (status, stdout) = match args {
                ["config", "--global", "--get", key] => match self.values.borrow().get(*key) {
                    Some(value) => (0, format!("{value}\n")),
                    None => (1, String::new()),
                },
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
             --app=\"/Applications/Scry.app\" \"$BASE\" \"$LOCAL\" \"$REMOTE\" \"$MERGED\""
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
