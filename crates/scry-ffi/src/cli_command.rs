//! The `git scry` command for the app's settings: whether the user's
//! terminal runs the app's own `git-scry`, and linking it.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use scry_core::cli_link::{self, CommandState};
use scry_core::mergetool;

use crate::{ScryError, failed};

/// What `git scry` runs in the user's terminal.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum CliCommandState {
    /// It runs the app's `git-scry`, found at `path`.
    Installed { path: String },
    /// No `git-scry` on the `PATH`: installing links one at `link`.
    NotInstalled { link: String },
    /// It runs `other`; installing links the app's at `link`, which
    /// replaces `other` or comes before it.
    Other { other: String, link: String },
    /// `other`, not a link, comes first; `remove` is the command that
    /// removes it.
    Blocked { other: String, remove: String },
    /// No known folder of the `PATH` can hold the link.
    NoFolder,
}

/// What `git scry` runs, for the app's `cli`. Runs the user's shell to read
/// its `PATH`, which can take a second: call it off the main thread.
#[uniffi::export]
pub fn cli_command_state(cli: String) -> CliCommandState {
    let home = home();
    let path = shell_path().unwrap_or_else(fallback_path);
    match cli_link::state(Path::new(&cli), &path, &home) {
        CommandState::Installed { path } => CliCommandState::Installed { path: shown(&path) },
        CommandState::NotInstalled { link } => CliCommandState::NotInstalled { link: shown(&link) },
        CommandState::Other { other, link } => CliCommandState::Other {
            other: shown(&other),
            link: shown(&link),
        },
        CommandState::Blocked { other, remove } => CliCommandState::Blocked {
            other: shown(&other),
            remove,
        },
        CommandState::NoFolder => CliCommandState::NoFolder,
    }
}

/// Links `link` (from `cli_command_state`) to the app's `cli`.
#[uniffi::export]
pub fn install_cli_command(cli: String, link: String) -> Result<(), ScryError> {
    if mergetool::translocated(Path::new(&cli)) {
        return Err(ScryError::Refused {
            message: String::from(
                "macOS runs Scry from a temporary copy, which the link would lose once Scry quits. Move Scry to the Applications folder, open it from there, then try again.",
            ),
        });
    }
    cli_link::install(Path::new(&cli), Path::new(&link)).map_err(failed)
}

fn shown(path: &Path) -> String {
    path.display().to_string()
}

fn home() -> PathBuf {
    std::env::var_os("HOME").map_or_else(|| PathBuf::from("/"), PathBuf::from)
}

/// Printed before the `PATH`, so what the shell's startup files print is
/// told apart from it.
const MARKER: &str = "__SCRY_PATH__";

/// How long the shell may take to start: startup files can be slow, or
/// wait for something that never comes.
const SHELL_TIMEOUT: Duration = Duration::from_secs(5);

/// The `PATH` the user's terminal has. An app opened from the Finder gets
/// launchd's (`/usr/bin:/bin:/usr/sbin:/sbin`), without what `~/.zprofile`
/// or `~/.zshrc` add (Homebrew, `~/.local/bin`), so the login shell is run,
/// interactive to read both. `printenv` prints it the same way whatever the
/// shell (fish keeps it as a list). `None` when the shell fails or is slow.
fn shell_path() -> Option<Vec<PathBuf>> {
    let shell = std::env::var("SHELL").unwrap_or_else(|_| String::from("/bin/zsh"));
    let script = format!("echo {MARKER}; /usr/bin/printenv PATH");
    let mut child = Command::new(shell)
        .args(["-i", "-l", "-c", &script])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let deadline = Instant::now() + SHELL_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(20));
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
    let mut output = String::new();
    child.stdout.take()?.read_to_string(&mut output).ok()?;
    path_in_output(&output)
}

/// The `PATH` printed after the marker line, as folders.
fn path_in_output(output: &str) -> Option<Vec<PathBuf>> {
    let mut lines = output.lines().skip_while(|line| line.trim() != MARKER);
    lines.next()?;
    let path = lines.next()?.trim();
    (!path.is_empty()).then(|| {
        path.split(':')
            .filter(|dir| !dir.is_empty())
            .map(PathBuf::from)
            .collect()
    })
}

/// When the shell cannot tell: the app's own `PATH`, then the folders
/// Homebrew and most installers put on it.
fn fallback_path() -> Vec<PathBuf> {
    let mut path: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|path| std::env::split_paths(&path).collect())
        .unwrap_or_default();
    for dir in ["/opt/homebrew/bin", "/usr/local/bin"] {
        let dir = PathBuf::from(dir);
        if !path.contains(&dir) {
            path.push(dir);
        }
    }
    path
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Whatever the shell's startup files print before it (a greeting, a
    /// version manager's notice), the `PATH` is the line after the marker.
    #[test]
    fn the_path_is_read_after_the_marker() {
        let output =
            "Welcome!\nnvm: using node 22\n__SCRY_PATH__\n/opt/homebrew/bin:/usr/bin::/bin\n";
        assert_eq!(
            path_in_output(output),
            Some(vec![
                PathBuf::from("/opt/homebrew/bin"),
                PathBuf::from("/usr/bin"),
                PathBuf::from("/bin"),
            ])
        );
        assert_eq!(path_in_output("no marker\n/usr/bin\n"), None);
        assert_eq!(path_in_output("__SCRY_PATH__\n"), None);
    }

    /// Without the shell, the folders Homebrew and installers use are still
    /// looked at, after the app's own `PATH`.
    #[test]
    fn the_fallback_adds_the_usual_folders() {
        let path = fallback_path();
        assert!(path.contains(&PathBuf::from("/opt/homebrew/bin")));
        assert!(path.contains(&PathBuf::from("/usr/local/bin")));
    }
}
