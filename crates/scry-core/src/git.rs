//! Running git.
//!
//! The core never spawns processes directly: it goes through the [`Git`] trait
//! (portability rule 3), so tests and other front ends can supply their own.

use std::fmt;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// What a git command produced.
pub struct GitOutput {
    /// The exit code; `None` when git was killed by a signal.
    pub status: Option<i32>,
    /// Raw bytes: file contents may not be UTF-8.
    pub stdout: Vec<u8>,
    pub stderr: String,
}

/// Something that runs git commands in one repository.
pub trait Git {
    /// Runs git with these arguments, whatever its exit code.
    fn output(&self, args: &[&str]) -> Result<GitOutput, GitError>;

    /// Runs git with `input` on its standard input, whatever its exit
    /// code.
    fn output_with_input(&self, args: &[&str], input: &[u8]) -> Result<GitOutput, GitError>;

    /// Runs git with these variables added to its environment, whatever
    /// its exit code: the editor git runs for a commit message, for one.
    fn output_with_env(&self, args: &[&str], env: &[(&str, &str)]) -> Result<GitOutput, GitError>;

    /// Runs git with these arguments and returns its standard output, or an
    /// error when git reports a failure.
    fn run(&self, args: &[&str]) -> Result<String, GitError> {
        let output = self.output(args)?;
        if output.status != Some(0) {
            return Err(GitError::Failed {
                command: args.join(" "),
                stderr: output.stderr,
            });
        }
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }

    /// Like `run`, with `input` on git's standard input.
    fn run_with_input(&self, args: &[&str], input: &[u8]) -> Result<String, GitError> {
        let output = self.output_with_input(args, input)?;
        if output.status != Some(0) {
            return Err(GitError::Failed {
                command: args.join(" "),
                stderr: output.stderr,
            });
        }
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }
}

/// Runs the `git` executable, the one found on the `PATH` unless another is
/// given.
#[derive(Clone)]
pub struct GitCli {
    program: PathBuf,
    dir: PathBuf,
    /// Set for every command, such as a configuration of its own for tests.
    environment: Vec<(String, String)>,
}

impl GitCli {
    /// Runs git as if started in `dir`.
    pub fn new(dir: PathBuf) -> GitCli {
        GitCli::with_program(PathBuf::from("git"), dir)
    }

    /// Runs the git the user runs (`program`), as if started in `dir`: what
    /// the app and the CLI use.
    pub fn discover(dir: PathBuf) -> GitCli {
        GitCli::with_program(program(), dir)
    }

    /// The user's git run from the folder of the file at `path`.
    pub fn beside(path: &Path) -> GitCli {
        let directory = match path.parent() {
            Some(parent) if !parent.as_os_str().is_empty() => parent.to_path_buf(),
            _ => PathBuf::from("."),
        };
        GitCli::discover(directory)
    }

    /// Runs `program` as git, as if started in `dir`: a front end that
    /// knows better than the `PATH` it was given which git the user runs.
    pub fn with_program(program: PathBuf, dir: PathBuf) -> GitCli {
        GitCli {
            program,
            dir,
            environment: Vec::new(),
        }
    }

    /// Sets `key` to `value` in every command's environment.
    pub fn with_env(mut self, key: &str, value: &str) -> GitCli {
        self.environment.push((key.to_string(), value.to_string()));
        self
    }

    /// The same git, with the same environment, started in `dir`: a front
    /// end at the repository root that reads a file from its folder.
    pub fn in_dir(&self, dir: PathBuf) -> GitCli {
        GitCli {
            dir,
            ..self.clone()
        }
    }
}

/// The git the user runs. Opened from the Finder, the Mac app gets
/// launchd's `PATH`, where `git` is Apple's `/usr/bin/git`: without Xcode
/// or its command-line tools, it asks to install them instead of running.
/// A git elsewhere on the `PATH`, or else Homebrew's, is the one the
/// user's terminal runs. Elsewhere than macOS those paths do not exist, and
/// the `PATH` decides.
pub fn program() -> PathBuf {
    let path = std::env::var_os("PATH").unwrap_or_default();
    choose_program(std::env::split_paths(&path).collect(), |candidate| {
        candidate.is_file()
    })
}

/// `program`'s choice among the `PATH`'s folders, with `exists` telling
/// which files exist.
pub fn choose_program(path: Vec<PathBuf>, exists: impl Fn(&Path) -> bool) -> PathBuf {
    let apple = Path::new("/usr/bin/git");
    let homebrew = [
        PathBuf::from("/opt/homebrew/bin/git"),
        PathBuf::from("/usr/local/bin/git"),
    ];
    path.into_iter()
        .map(|directory| directory.join("git"))
        .filter(|candidate| candidate != apple)
        .chain(homebrew)
        .find(|candidate| exists(candidate))
        .unwrap_or_else(|| PathBuf::from("git"))
}

impl GitCli {
    fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::new(&self.program);
        command
            .arg("-C")
            .arg(&self.dir)
            // A file-system monitor daemon would be started, or asked, for
            // every command Scry runs; reads stay correct without it.
            .args(["-c", "core.fsmonitor=false"])
            .args(args)
            // Scry reads while the user's own git commands may run: never take
            // the index lock just to refresh it.
            .env("GIT_OPTIONAL_LOCKS", "0")
            // Paths are file names, never patterns: `[id].tsx` must not
            // also match `i.tsx` or `d.tsx`.
            .env("GIT_LITERAL_PATHSPECS", "1")
            // A relative `GIT_DIR` from the caller (a hook, `git
            // --git-dir=… scry`) means the caller's directory, but git
            // would read it from `-C`'s.
            .envs(absolute_git_paths(
                std::env::vars_os(),
                std::env::current_dir().ok().as_deref(),
            ))
            .envs(self.environment.iter().map(|(key, value)| (key, value)));
        command
    }
}

/// The variables naming a repository's paths, those relative made
/// absolute from `current`, the directory they were set for. Others are
/// left out.
fn absolute_git_paths(
    variables: impl Iterator<Item = (std::ffi::OsString, std::ffi::OsString)>,
    current: Option<&Path>,
) -> Vec<(std::ffi::OsString, PathBuf)> {
    const PATHS: [&str; 5] = [
        "GIT_DIR",
        "GIT_WORK_TREE",
        "GIT_INDEX_FILE",
        "GIT_COMMON_DIR",
        "GIT_OBJECT_DIRECTORY",
    ];
    let Some(current) = current else {
        return Vec::new();
    };
    variables
        .filter(|(key, value)| {
            PATHS.iter().any(|name| key == name) && Path::new(value).is_relative()
        })
        .map(|(key, value)| {
            let path = current.join(value);
            (key, path)
        })
        .collect()
}

impl Git for GitCli {
    fn output(&self, args: &[&str]) -> Result<GitOutput, GitError> {
        let output = self.command(args).output().map_err(GitError::Spawn)?;
        Ok(git_output(output))
    }

    fn output_with_env(&self, args: &[&str], env: &[(&str, &str)]) -> Result<GitOutput, GitError> {
        let output = self
            .command(args)
            .envs(env.iter().copied())
            .output()
            .map_err(GitError::Spawn)?;
        Ok(git_output(output))
    }

    fn output_with_input(&self, args: &[&str], input: &[u8]) -> Result<GitOutput, GitError> {
        let mut child = self
            .command(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(GitError::Spawn)?;
        let mut stdin = child.stdin.take();
        // Written from another thread while the output is read, so neither
        // side waits on a full pipe. Git stopping early only closes the pipe.
        let output = std::thread::scope(|scope| {
            scope.spawn(move || {
                if let Some(stdin) = stdin.as_mut() {
                    let _ = stdin.write_all(input);
                }
            });
            child.wait_with_output()
        })
        .map_err(GitError::Spawn)?;
        Ok(git_output(output))
    }
}

fn git_output(output: std::process::Output) -> GitOutput {
    GitOutput {
        status: output.status.code(),
        stdout: output.stdout,
        stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
    }
}

/// Why a git command gave no usable answer.
#[derive(Debug)]
pub enum GitError {
    /// The `git` executable could not be started.
    Spawn(io::Error),
    /// Git ran and reported a failure.
    Failed { command: String, stderr: String },
    /// Git succeeded but its output is not what Scry expects.
    UnexpectedOutput { command: String, output: String },
}

impl fmt::Display for GitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GitError::Spawn(error) => write!(f, "cannot run git: {error}"),
            GitError::Failed { command, stderr } => write!(f, "git {command} failed: {stderr}"),
            GitError::UnexpectedOutput { command, output } => {
                write!(f, "unexpected output from git {command}: {output:?}")
            }
        }
    }
}

impl std::error::Error for GitError {}

#[cfg(test)]
mod tests {
    use super::*;

    /// Error messages name the git command, so the user can run it again.
    #[test]
    fn errors_say_which_git_command_failed() {
        let failed = GitError::Failed {
            command: String::from("add a.txt"),
            stderr: String::from("fatal: oops"),
        };
        assert_eq!(failed.to_string(), "git add a.txt failed: fatal: oops");
        let unexpected = GitError::UnexpectedOutput {
            command: String::from("rev-list --count"),
            output: String::from("x"),
        };
        assert_eq!(
            unexpected.to_string(),
            "unexpected output from git rev-list --count: \"x\""
        );
    }

    /// Git for a file runs from that file's folder, so a file in a
    /// subfolder is read relative to it.
    #[test]
    fn git_beside_a_file_runs_from_its_folder() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("sub")).unwrap();
        let isolated = |git: GitCli| {
            git.with_env("GIT_CONFIG_GLOBAL", "/dev/null")
                .with_env("GIT_CONFIG_NOSYSTEM", "1")
        };
        let init = isolated(GitCli::new(dir.path().to_path_buf()));
        init.run(&["init", "--quiet"]).unwrap();
        let git = isolated(GitCli::beside(&dir.path().join("sub").join("a.txt")));
        assert_eq!(git.run(&["rev-parse", "--show-prefix"]).unwrap(), "sub/\n");
    }

    /// A relative `GIT_DIR` set by whoever ran Scry is made absolute from
    /// their directory: Scry runs git with `-C` in another folder, where
    /// the relative path would name the wrong repository or none.
    #[test]
    fn relative_repository_paths_become_absolute() {
        // An absolute path names its drive on Windows.
        let root = Path::new(if cfg!(windows) { "C:\\repo" } else { "/repo" });
        let index = root.join("index");
        let variables = [
            ("GIT_DIR", ".git"),
            ("GIT_INDEX_FILE", index.to_str().unwrap()),
            ("GIT_WORK_TREE", "home"),
            ("HOME", "relative"),
        ]
        .map(|(key, value)| (key.into(), value.into()));
        let paths = absolute_git_paths(variables.into_iter(), Some(root));
        assert_eq!(
            paths,
            [
                ("GIT_DIR".into(), root.join(".git")),
                ("GIT_WORK_TREE".into(), root.join("home")),
            ]
        );
    }

    /// Opened from the Finder, the app must not pick Apple's `/usr/bin/git`
    /// over the git the user installed, which may ask to install Xcode's
    /// tools; a git on the user's own `PATH` comes first.
    #[test]
    fn picks_the_users_git() {
        let installed = |found: &'static [&'static str]| {
            move |candidate: &Path| found.iter().any(|path| candidate == Path::new(path))
        };
        let launchd = vec![PathBuf::from("/usr/bin"), PathBuf::from("/bin")];

        assert_eq!(
            choose_program(
                launchd.clone(),
                installed(&["/usr/bin/git", "/opt/homebrew/bin/git"])
            ),
            PathBuf::from("/opt/homebrew/bin/git")
        );
        assert_eq!(
            choose_program(
                vec![PathBuf::from("/nix/bin"), PathBuf::from("/usr/bin")],
                installed(&["/nix/bin/git", "/opt/homebrew/bin/git"])
            ),
            PathBuf::from("/nix/bin/git")
        );
        // Only Apple's: plain `git`, found on the PATH as before.
        assert_eq!(
            choose_program(launchd, installed(&["/usr/bin/git"])),
            PathBuf::from("git")
        );
    }
}
