//! `git scry` (or `git scry open`): opens the Scry app on the current
//! repository's overview, from any folder in it.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use scry_core::git::GitCli;
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
    let Some(app) = app.or_else(crate::resolve::find_app) else {
        eprintln!(
            "git-scry: Scry.app not found in /Applications or ~/Applications; \
             pass --app=<path> or set SCRY_APP"
        );
        return ExitCode::from(2);
    };
    match launch(&app, &root) {
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
fn launch(app: &Path, root: &Path) -> std::io::Result<bool> {
    let status = if app.is_dir() {
        std::process::Command::new("open")
            .args(["-n", "-a"])
            .arg(app)
            .arg("--args")
            .arg(root)
            .status()?
    } else {
        std::process::Command::new(app).arg(root).status()?
    };
    Ok(status.success())
}
