//! Running git.
//!
//! The core never spawns processes directly: it goes through the [`Git`] trait
//! (portability rule 3), so tests and other front ends can supply their own.

use std::fmt;
use std::io::{self, Write};
use std::path::PathBuf;
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
            .envs(self.environment.iter().map(|(key, value)| (key, value)));
        command
    }
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
