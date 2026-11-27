//! `git-scry difftool`: what `git difftool` runs once Scry is git's diff
//! tool. It opens the app on the two files, or the two folders of
//! `--dir-diff`, and waits for the window to close: git removes its
//! temporary files as soon as the tool exits.
//!
//! Without `--dir-diff` git runs it once per file, the next when this one
//! exits: the window is told its place among them, and its Stop button
//! ends the whole `git difftool`.

use std::ffi::OsString;
use std::path::PathBuf;
use std::process::ExitCode;

pub struct Options {
    pub local: PathBuf,
    pub remote: PathBuf,
    /// The file's path in the repository (`$MERGED`); empty with
    /// `--dir-diff`.
    pub name: Option<String>,
    pub app: Option<PathBuf>,
}

/// The app's option for `git difftool`'s files.
const DIFFTOOL: &str = "--difftool";

/// What the app answers when the user stops instead of going on.
const STOP: i32 = 1;

/// The exit code that ends `git difftool`: from 126 up git takes the tool
/// as failed or interrupted and opens no other file, whatever
/// `difftool.trustExitCode` says. 130 is what Ctrl-C would give.
const INTERRUPTED: u8 = 130;

/// This file's place among those `git difftool` opens one after the
/// other, as git's environment gives it: `(2, 13)`. `None` for a single
/// file, with `--dir-diff`, or outside git.
fn place(counter: Option<String>, total: Option<String>) -> Option<(u32, u32)> {
    let counter: u32 = counter?.parse().ok()?;
    let total: u32 = total?.parse().ok()?;
    (total > 1 && (1..=total).contains(&counter)).then_some((counter, total))
}

pub fn run(options: Options) -> ExitCode {
    let Some(app) = options.app.or_else(crate::resolve::find_app) else {
        eprintln!(
            "git-scry: Scry.app not found in /Applications or ~/Applications; \
             pass --app=<path> or set SCRY_APP"
        );
        return ExitCode::from(2);
    };
    // Each path follows an option: AppKit takes a bare path for a file to
    // open. The app does not run in git's folder, so the paths are
    // absolute; `/dev/null`, for a file one side does not have, already is.
    let mut arguments: Vec<OsString> = vec![DIFFTOOL.into()];
    for (option, path) in [("--old", &options.local), ("--new", &options.remote)] {
        match std::path::absolute(path) {
            Ok(path) => {
                arguments.push(option.into());
                arguments.push(path.into_os_string());
            }
            Err(error) => {
                eprintln!("git-scry: {}: {error}", path.display());
                return ExitCode::from(2);
            }
        }
    }
    if let Some(name) = options.name.filter(|name| !name.is_empty()) {
        arguments.push("--name".into());
        arguments.push(name.into());
    }
    let variable = |name: &str| std::env::var(name).ok();
    if let Some((counter, total)) = place(
        variable("GIT_DIFF_PATH_COUNTER"),
        variable("GIT_DIFF_PATH_TOTAL"),
    ) {
        arguments.push("--place".into());
        arguments.push(counter.to_string().into());
        arguments.push("--of".into());
        arguments.push(total.to_string().into());
    }
    let answer = if app.is_dir() {
        crate::resolve::run_bundle(&app, arguments)
    } else {
        crate::resolve::run_executable(&app, arguments)
    };
    match answer {
        Ok(Some(STOP)) => ExitCode::from(INTERRUPTED),
        // Reading changes nothing: a window closed, or an app that died,
        // lets git go on to the next file.
        Ok(_) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("git-scry: cannot run {}: {error}", app.display());
            ExitCode::from(2)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::place;

    fn text(value: &str) -> Option<String> {
        Some(value.to_string())
    }

    #[test]
    fn the_place_is_read_from_gits_counter_and_total() {
        assert_eq!(place(text("2"), text("13")), Some((2, 13)));
        assert_eq!(place(text("13"), text("13")), Some((13, 13)));
        assert_eq!(place(text("1"), text("2")), Some((1, 2)));
    }

    /// One file is no queue, and values git would not give are ignored.
    #[test]
    fn there_is_no_place_for_one_file_or_without_git() {
        assert_eq!(place(text("1"), text("1")), None);
        assert_eq!(place(None, None), None);
        assert_eq!(place(text("2"), None), None);
        assert_eq!(place(None, text("3")), None);
        assert_eq!(place(text("0"), text("3")), None);
        assert_eq!(place(text("4"), text("3")), None);
        assert_eq!(place(text("two"), text("3")), None);
    }
}
