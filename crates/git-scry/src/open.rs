//! `git scry` (or `git scry open`): opens the Scry app on the current
//! repository, from any folder in it: on the overview of the merge, rebase,
//! cherry-pick or revert in progress, or else on what is not committed.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use std::ffi::OsString;

use scry_core::git::GitCli;
use scry_core::operation;
use scry_core::overview;

/// Opens the app on the repository holding `path` (the current folder by
/// default) and returns at once: the app stays open on its own.
pub fn run(path: Option<PathBuf>, app: Option<PathBuf>) -> ExitCode {
    let folder = match path {
        Some(path) => path,
        None => match std::env::current_dir() {
            Ok(folder) => folder,
            Err(error) => {
                eprintln!("git-scry: cannot read the current folder: {error}");
                return ExitCode::from(2);
            }
        },
    };
    let Ok(root) = overview::root(&GitCli::discover(folder.clone())) else {
        eprintln!("git-scry: {} is not in a git repository", folder.display());
        return ExitCode::from(2);
    };
    // Nothing to resolve: the window shows what is not committed. Files
    // left with conflicts without an operation (`git stash pop`) belong to
    // the overview too.
    let git = GitCli::discover(root.clone());
    let resolving = operation::current(&git).ok().flatten().is_some()
        || operation::conflicted_paths(&git).is_ok_and(|paths| !paths.is_empty());
    let mut arguments: Vec<OsString> = Vec::new();
    if !resolving {
        arguments.push(DIFF.into());
    }
    arguments.push(root.clone().into_os_string());
    open_app(app, &root, arguments)
}

/// The app's option for the Changes window: the repository's root follows
/// it, then what `git scry diff` was asked.
pub const DIFF: &str = "--diff";

/// Starts the app with `arguments` and says so, or says why it could not.
pub fn open_app(app: Option<PathBuf>, root: &Path, arguments: Vec<OsString>) -> ExitCode {
    let Some(app) = app.or_else(crate::resolve::find_app) else {
        eprintln!(
            "git-scry: Scry.app not found in /Applications or ~/Applications; \
             pass --app=<path> or set SCRY_APP"
        );
        return ExitCode::from(2);
    };
    match launch(&app, &arguments) {
        Ok(true) => {
            println!("Opening {} in Scry", root.display());
            ExitCode::SUCCESS
        }
        Ok(false) => {
            eprintln!("git-scry: {} did not open", app.display());
            ExitCode::from(2)
        }
        Err(error) => {
            eprintln!("git-scry: cannot run {}: {error}", app.display());
            ExitCode::from(2)
        }
    }
}

/// A bundle opens through Launch Services, which brings it to the front;
/// `-n` starts a new window process even when Scry is already open on
/// another repository. Any other executable runs as the app, for tests.
fn launch(app: &Path, arguments: &[OsString]) -> std::io::Result<bool> {
    let status = if app.is_dir() {
        std::process::Command::new("open")
            .args(["-n", "-a"])
            .arg(app)
            .arg("--args")
            .args(arguments)
            .status()?
    } else {
        std::process::Command::new(app).args(arguments).status()?
    };
    Ok(status.success())
}
