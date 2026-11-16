//! The `git scry` command: git runs it by finding an executable named
//! `git-scry` on the `PATH`. An app installed by hand carries its own
//! `git-scry` and links it nowhere, so the settings say whether `git scry`
//! runs it, and link it into a folder of the `PATH`.
//!
//! The `PATH` is given, not read here: a Mac app does not get the
//! terminal's, so the front end reads the user's shell for it.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// The executable git looks for to run `git scry`.
pub const NAME: &str = "git-scry";

/// What `git scry` runs, and what installing would do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandState {
    /// `git scry` runs the app's own `git-scry`, found at `path`.
    Installed { path: PathBuf },
    /// No `git-scry` on the `PATH`: installing links one at `link`.
    NotInstalled { link: PathBuf },
    /// `git scry` runs `other` (another copy of Scry, a build moved since):
    /// installing links the app's at `link`, which replaces `other` or
    /// comes before it on the `PATH`.
    Other { other: PathBuf, link: PathBuf },
    /// `other`, not a link, comes first on the `PATH` and is not replaced:
    /// it must be removed, with `remove`, for `git scry` to run the app's.
    Blocked { other: PathBuf, remove: String },
    /// No folder of the `PATH` that Scry links into (`~/.local/bin`,
    /// `/opt/homebrew/bin`, `/usr/local/bin`) exists and is writable.
    NoFolder,
}

/// The folders a link goes into, when they are on the `PATH`: the user's
/// own, then Homebrew's, then the traditional one. Other folders of the
/// `PATH` belong to other tools (`~/.cargo/bin`) or to the system.
pub fn link_folders(home: &Path) -> [PathBuf; 3] {
    [
        home.join(".local/bin"),
        PathBuf::from("/opt/homebrew/bin"),
        PathBuf::from("/usr/local/bin"),
    ]
}

/// What `git scry` runs with `path` as the `PATH`'s folders, in order, for
/// the app's `cli`.
pub fn state(cli: &Path, path: &[PathBuf], home: &Path) -> CommandState {
    let candidates = link_folders(home);
    // The first suitable folder, in the order the PATH lists them: one
    // holding a git-scry that is not a link is not, since `install` never
    // overwrites a file.
    let folder = path
        .iter()
        .position(|dir| candidates.contains(dir) && writable(dir) && !holds_file(&dir.join(NAME)));
    let found = path.iter().position(|dir| is_executable(&dir.join(NAME)));
    let Some(found) = found else {
        return match folder {
            Some(folder) => CommandState::NotInstalled {
                link: path[folder].join(NAME),
            },
            None => CommandState::NoFolder,
        };
    };
    let other = path[found].join(NAME);
    if same_file(&other, cli) {
        return CommandState::Installed { path: other };
    }
    let replaceable = fs::symlink_metadata(&other).is_ok_and(|meta| meta.is_symlink())
        && candidates.contains(&path[found])
        && writable(&path[found]);
    if replaceable {
        return CommandState::Other {
            link: other.clone(),
            other,
        };
    }
    match folder {
        Some(folder) if folder < found => CommandState::Other {
            other,
            link: path[folder].join(NAME),
        },
        _ => CommandState::Blocked {
            remove: remove_command(&other, home),
            other,
        },
    }
}

/// Links `link` to the app's `cli`, replacing a link already there; a file
/// that is not a link is never replaced. The new link is made beside it and
/// renamed over it, so `git scry` never finds the name missing.
pub fn install(cli: &Path, link: &Path) -> io::Result<()> {
    if holds_file(link) {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("{} is not a link to Scry; remove it first", link.display()),
        ));
    }
    let staging = link.with_file_name(format!(".{NAME}.scry-installing"));
    let _ = fs::remove_file(&staging);
    symlink(cli, &staging)?;
    fs::rename(&staging, link).inspect_err(|_| {
        let _ = fs::remove_file(&staging);
    })
}

#[cfg(unix)]
fn symlink(target: &Path, link: &Path) -> io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

#[cfg(not(unix))]
fn symlink(_: &Path, _: &Path) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "linking git-scry is only done on macOS and Linux",
    ))
}

/// How to remove another `git-scry`: `cargo uninstall` for one cargo
/// installed, `rm` otherwise.
fn remove_command(other: &Path, home: &Path) -> String {
    if other.parent() == Some(home.join(".cargo/bin").as_path()) {
        return format!("cargo uninstall {NAME}");
    }
    let shown = other.display().to_string();
    format!("rm '{}'", shown.replace('\'', "'\\''"))
}

/// Whether `path` is a file that can run, following links: a link to an app
/// moved or deleted since runs nothing, and git skips it.
fn is_executable(path: &Path) -> bool {
    let Ok(meta) = fs::metadata(path) else {
        return false;
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.is_file() && meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        meta.is_file()
    }
}

/// Whether `path` is there and is not a link.
fn holds_file(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|meta| !meta.is_symlink())
}

/// Whether `a` and `b` are the same file once links are followed.
fn same_file(a: &Path, b: &Path) -> bool {
    match (fs::canonicalize(a), fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

/// Whether a link can be made in `dir`: tried with a file, since
/// permissions alone do not tell (ownership, access lists, a read-only
/// volume).
fn writable(dir: &Path) -> bool {
    if !dir.is_dir() {
        return false;
    }
    let probe = dir.join(format!(".{NAME}.scry-probe-{}", std::process::id()));
    match fs::File::create(&probe) {
        Ok(_) => {
            let _ = fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}

#[cfg(all(test, unix))]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    /// A home folder with an app's `git-scry` and the folders a `PATH` may
    /// hold, all in a temporary directory.
    struct Sandbox {
        root: tempfile::TempDir,
    }

    impl Sandbox {
        fn new() -> Sandbox {
            let sandbox = Sandbox {
                root: tempfile::tempdir().unwrap(),
            };
            let cli = sandbox.cli();
            fs::create_dir_all(cli.parent().unwrap()).unwrap();
            executable(&cli);
            sandbox
        }

        fn home(&self) -> PathBuf {
            self.root.path().join("home")
        }

        fn cli(&self) -> PathBuf {
            self.root
                .path()
                .join("Applications/Scry.app/Contents/Helpers/git-scry")
        }

        /// A folder of the `PATH`, created.
        fn dir(&self, relative: &str) -> PathBuf {
            let dir = self.root.path().join(relative);
            fs::create_dir_all(&dir).unwrap();
            dir
        }

        fn state(&self, path: &[PathBuf]) -> CommandState {
            state(&self.cli(), path, &self.home())
        }
    }

    fn executable(path: &Path) {
        fs::write(path, "#!/bin/sh\n").unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }

    /// With nothing on the `PATH`, the link goes into the first of the known
    /// folders the `PATH` lists, and `git scry` then runs the app's: what a
    /// user of the release needs, without typing a command.
    #[test]
    fn installs_into_the_first_known_folder_of_the_path() {
        let sandbox = Sandbox::new();
        let local = sandbox.dir("home/.local/bin");
        let system = sandbox.dir("usr/bin");
        let path = [system, local.clone()];

        let link = local.join(NAME);
        assert_eq!(
            sandbox.state(&path),
            CommandState::NotInstalled { link: link.clone() }
        );
        install(&sandbox.cli(), &link).unwrap();
        assert_eq!(
            sandbox.state(&path),
            CommandState::Installed { path: link.clone() }
        );
        assert_eq!(fs::read_link(&link).unwrap(), sandbox.cli());
    }

    /// A folder that is not on the `PATH` is no use, however known: on a new
    /// Mac `~/.local/bin` is often missing from it, the 0.1.0 release notes'
    /// mistake.
    #[test]
    fn a_known_folder_off_the_path_is_not_used() {
        let sandbox = Sandbox::new();
        sandbox.dir("home/.local/bin");
        let system = sandbox.dir("usr/bin");
        assert_eq!(sandbox.state(&[system]), CommandState::NoFolder);
    }

    /// A link to another copy of Scry, or to one moved since that still
    /// exists, is replaced where it is, so `git scry` runs this app.
    #[test]
    fn a_link_to_another_copy_is_replaced() {
        let sandbox = Sandbox::new();
        let local = sandbox.dir("home/.local/bin");
        let old = sandbox.dir("Old/Scry.app/Contents/Helpers").join(NAME);
        executable(&old);
        let link = local.join(NAME);
        std::os::unix::fs::symlink(&old, &link).unwrap();
        let path = [local];

        assert_eq!(
            sandbox.state(&path),
            CommandState::Other {
                other: link.clone(),
                link: link.clone()
            }
        );
        install(&sandbox.cli(), &link).unwrap();
        assert_eq!(sandbox.state(&path), CommandState::Installed { path: link });
    }

    /// A link to an app deleted since runs nothing: git skips it, so it
    /// counts as not installed, and installing replaces it.
    #[test]
    fn a_link_to_a_deleted_app_counts_as_not_installed() {
        let sandbox = Sandbox::new();
        let local = sandbox.dir("home/.local/bin");
        let link = local.join(NAME);
        std::os::unix::fs::symlink("/nowhere/git-scry", &link).unwrap();

        assert_eq!(
            sandbox.state(std::slice::from_ref(&local)),
            CommandState::NotInstalled { link: link.clone() }
        );
        install(&sandbox.cli(), &link).unwrap();
        assert_eq!(fs::read_link(&link).unwrap(), sandbox.cli());
    }

    /// Another `git-scry` that is not a link (an old `cargo install`) and
    /// comes first is never replaced: the settings say how to remove it,
    /// with cargo for one cargo installed.
    #[test]
    fn another_binary_first_on_the_path_blocks() {
        let sandbox = Sandbox::new();
        let cargo = sandbox.dir("home/.cargo/bin");
        executable(&cargo.join(NAME));
        let local = sandbox.dir("home/.local/bin");

        assert_eq!(
            sandbox.state(&[cargo.clone(), local]),
            CommandState::Blocked {
                other: cargo.join(NAME),
                remove: String::from("cargo uninstall git-scry"),
            }
        );

        let usr = sandbox.dir("usr/local/bin");
        executable(&usr.join(NAME));
        let CommandState::Blocked { remove, .. } = sandbox.state(std::slice::from_ref(&usr)) else {
            panic!("not blocked");
        };
        assert_eq!(remove, format!("rm '{}'", usr.join(NAME).display()));
    }

    /// Another binary that comes after a known folder is shadowed by a link
    /// in that folder, so installing still works.
    #[test]
    fn another_binary_later_on_the_path_is_shadowed() {
        let sandbox = Sandbox::new();
        let local = sandbox.dir("home/.local/bin");
        let cargo = sandbox.dir("home/.cargo/bin");
        executable(&cargo.join(NAME));

        assert_eq!(
            sandbox.state(&[local.clone(), cargo.clone()]),
            CommandState::Other {
                other: cargo.join(NAME),
                link: local.join(NAME),
            }
        );
    }

    /// A link in a folder Scry does not link into (another tool's, such as
    /// `~/.cargo/bin`) is not replaced: removing it is the user's call.
    #[test]
    fn a_link_in_another_tools_folder_is_not_replaced() {
        let sandbox = Sandbox::new();
        let cargo = sandbox.dir("home/.cargo/bin");
        let old = sandbox.dir("Old/Scry.app/Contents/Helpers").join(NAME);
        executable(&old);
        std::os::unix::fs::symlink(&old, cargo.join(NAME)).unwrap();

        assert!(matches!(
            sandbox.state(&[cargo, sandbox.dir("home/.local/bin")]),
            CommandState::Blocked { .. }
        ));
    }

    /// A file that is not a link, in the very folder a link would go into,
    /// blocks: installing there would have to overwrite it.
    #[test]
    fn a_file_in_the_link_folder_blocks() {
        let sandbox = Sandbox::new();
        let local = sandbox.dir("home/.local/bin");
        executable(&local.join(NAME));

        assert!(matches!(
            sandbox.state(&[local]),
            CommandState::Blocked { .. }
        ));
    }

    /// A `git-scry` that cannot run is skipped, as git skips it: it is not
    /// what `git scry` runs, and the file is not overwritten either.
    #[test]
    fn a_file_that_cannot_run_is_skipped() {
        let sandbox = Sandbox::new();
        let local = sandbox.dir("home/.local/bin");
        let file = local.join(NAME);
        fs::write(&file, "not a program").unwrap();
        fs::set_permissions(&file, fs::Permissions::from_mode(0o644)).unwrap();
        let usr = sandbox.dir("usr/bin");

        // Not run, yet not overwritten either: no folder is left.
        assert_eq!(sandbox.state(&[usr, local]), CommandState::NoFolder);
    }

    /// A file that is not a link is never overwritten by installing, even
    /// when asked directly: it may be a build the user made.
    #[test]
    fn install_never_replaces_a_file() {
        let sandbox = Sandbox::new();
        let local = sandbox.dir("home/.local/bin");
        let file = local.join(NAME);
        executable(&file);

        assert!(install(&sandbox.cli(), &file).is_err());
        assert_eq!(fs::read_to_string(&file).unwrap(), "#!/bin/sh\n");
        assert!(!local.join(format!(".{NAME}.scry-installing")).exists());
    }

    /// A folder that cannot be written to is passed over for the next
    /// known one: a link there would fail.
    #[test]
    fn a_read_only_folder_is_passed_over() {
        let sandbox = Sandbox::new();
        let local = sandbox.dir("home/.local/bin");
        fs::set_permissions(&local, fs::Permissions::from_mode(0o555)).unwrap();
        let state = sandbox.state(std::slice::from_ref(&local));
        fs::set_permissions(&local, fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(state, CommandState::NoFolder);
    }
}
