//! Making Scry git's diff tool, in the user's global git configuration:
//! what the app's settings show and the switch they offer.
//!
//! `git difftool` hands a tool two files, or two folders with
//! `--dir-diff`, and nothing else: no revisions, so Scry shows the diff
//! alone. `git scry diff` needs no setting and shows the whole change.

use std::path::Path;

use crate::git::{Git, GitError};
use crate::mergetool::{Setup, TOOL, global, quote};

/// The diff tool command that runs `cli` (`git-scry`) to open `app`, the
/// app bundle. Paths are absolute, so it does not depend on the `PATH` git
/// gets, and quoted for the shell git runs it in. `--` before the files:
/// git passes a file named `-dash.txt` as it is. `$MERGED` is the file's
/// name in the repository, empty with `--dir-diff`.
pub fn command(cli: &Path, app: &Path) -> String {
    format!(
        "{} difftool --app={} -- \"$LOCAL\" \"$REMOTE\" \"$MERGED\"",
        quote(&cli.to_string_lossy()),
        quote(&app.to_string_lossy())
    )
}

/// Reads the diff tool from git's global configuration; `command` is the
/// one this app would install.
pub fn setup(git: &dyn Git, command: &str) -> Result<Setup, GitError> {
    let tool = global(git, "diff.tool")?;
    Ok(match tool.as_deref() {
        None => Setup::NotSet,
        Some(TOOL) => Setup::Scry {
            current: global(git, "difftool.scry.cmd")?.as_deref() == Some(command),
        },
        Some(other) => Setup::Other {
            tool: other.to_string(),
        },
    })
}

/// Where Scry keeps the diff tool it replaced, so `uninstall` puts it back.
const PREVIOUS: &str = "difftool.scry.previousTool";
/// Set when Scry turned git's question off, so `uninstall` turns it back
/// on and leaves alone a choice the user made.
const SET_PROMPT: &str = "difftool.scry.setPrompt";

/// Makes Scry git's diff tool for every repository, running `command`.
///
/// Git asks "Launch 'scry' [Y/n]?" before every file unless
/// `difftool.prompt` is false: Scry sets it when the user has not, and
/// remembers that it did. The tool it replaces is kept for `uninstall`.
/// `diff.tool` is written last: a failure before leaves git with the tool
/// it had, never naming a Scry without a command.
pub fn install(git: &dyn Git, command: &str) -> Result<(), GitError> {
    if let Some(previous) = global(git, "diff.tool")?.filter(|tool| tool != TOOL) {
        git.run(&["config", "--global", PREVIOUS, &previous])?;
    }
    if global(git, "difftool.prompt")?.is_none() {
        git.run(&["config", "--global", "difftool.prompt", "false"])?;
        git.run(&["config", "--global", SET_PROMPT, "true"])?;
    }
    for (key, value) in [("difftool.scry.cmd", command), ("diff.tool", TOOL)] {
        git.run(&["config", "--global", key, value])?;
    }
    Ok(())
}

/// The diff tool Scry replaced, which `uninstall` puts back; `None` when
/// there was none.
pub fn previous_tool(git: &dyn Git) -> Result<Option<String>, GitError> {
    global(git, PREVIOUS)
}

/// Stops using Scry as git's diff tool: the tool it replaced is git's
/// again, or none, git asks before each file again if Scry had turned
/// that off, and Scry's settings are removed. A diff tool chosen since,
/// other than Scry, is left alone.
pub fn uninstall(git: &dyn Git) -> Result<(), GitError> {
    if global(git, "diff.tool")?.as_deref() == Some(TOOL) {
        match previous_tool(git)? {
            Some(previous) => git.run(&["config", "--global", "diff.tool", &previous])?,
            None => git.run(&["config", "--global", "--unset", "diff.tool"])?,
        };
    }
    if global(git, SET_PROMPT)?.is_some() {
        git.output(&["config", "--global", "--unset", "difftool.prompt"])?;
    }
    // Exits with an error when the section is already gone: nothing to do.
    git.output(&["config", "--global", "--remove-section", "difftool.scry"])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mergetool::tests::FakeConfig;

    fn scry_command() -> String {
        command(
            Path::new("/Applications/Scry.app/Contents/Helpers/git-scry"),
            Path::new("/Applications/Scry.app"),
        )
    }

    /// Installing writes what `git difftool` needs and turns off git's
    /// question before each file; the settings then say Scry is the diff
    /// tool, running this app. Removing it leaves nothing behind.
    #[test]
    fn install_makes_scry_the_diff_tool_and_uninstall_leaves_nothing() {
        let git = FakeConfig::default();
        let command = scry_command();
        assert_eq!(setup(&git, &command).unwrap(), Setup::NotSet);

        install(&git, &command).unwrap();
        let values = git.values.borrow().clone();
        assert_eq!(values["diff.tool"], "scry");
        assert_eq!(
            values["difftool.scry.cmd"],
            "\"/Applications/Scry.app/Contents/Helpers/git-scry\" difftool \
             --app=\"/Applications/Scry.app\" -- \"$LOCAL\" \"$REMOTE\" \"$MERGED\""
        );
        assert_eq!(values["difftool.prompt"], "false");
        assert_eq!(
            setup(&git, &command).unwrap(),
            Setup::Scry { current: true }
        );
        // An app moved since: Scry is named, with another command.
        assert_eq!(
            setup(&git, "another command").unwrap(),
            Setup::Scry { current: false }
        );

        uninstall(&git).unwrap();
        assert!(git.values.borrow().is_empty(), "{:?}", git.values.borrow());
        assert_eq!(setup(&git, &command).unwrap(), Setup::NotSet);
    }

    /// The tool Scry replaces comes back when Scry is removed, and a
    /// choice the user made about git's question is never touched.
    #[test]
    fn uninstall_puts_back_the_tool_and_the_users_own_prompt() {
        let git = FakeConfig::default();
        for (key, value) in [("diff.tool", "opendiff"), ("difftool.prompt", "true")] {
            git.run(&["config", "--global", key, value]).unwrap();
        }
        let command = scry_command();
        assert_eq!(
            setup(&git, &command).unwrap(),
            Setup::Other {
                tool: String::from("opendiff")
            }
        );

        install(&git, &command).unwrap();
        assert_eq!(previous_tool(&git).unwrap().as_deref(), Some("opendiff"));
        assert_eq!(git.values.borrow()["difftool.prompt"], "true");

        uninstall(&git).unwrap();
        let values = git.values.borrow().clone();
        assert_eq!(values["diff.tool"], "opendiff");
        assert_eq!(values["difftool.prompt"], "true");
        assert_eq!(values.len(), 2, "{values:?}");
    }

    /// Installing again over Scry keeps the tool first replaced, and a
    /// diff tool chosen after Scry is left alone when Scry is removed.
    #[test]
    fn a_second_install_and_a_later_choice_are_respected() {
        let git = FakeConfig::default();
        git.run(&["config", "--global", "diff.tool", "vimdiff"])
            .unwrap();
        let command = scry_command();
        install(&git, &command).unwrap();
        install(&git, &command).unwrap();
        assert_eq!(previous_tool(&git).unwrap().as_deref(), Some("vimdiff"));

        git.run(&["config", "--global", "diff.tool", "kaleidoscope"])
            .unwrap();
        uninstall(&git).unwrap();
        let values = git.values.borrow().clone();
        assert_eq!(values["diff.tool"], "kaleidoscope");
        assert!(!values.contains_key("difftool.scry.cmd"));
        assert!(!values.contains_key("difftool.prompt"));
    }
}
