//! Running git.
//!
//! The core never spawns processes directly: it goes through the [`Git`] trait
//! (portability rule 3), so tests and other front ends can supply their own.

use std::fmt;
use std::io;
use std::path::PathBuf;
use std::process::Command;

/// Something that runs git commands in one repository.
pub trait Git {
    /// Runs git with these arguments and returns its standard output.
    fn run(&self, args: &[&str]) -> Result<String, GitError>;
}

/// Runs the `git` executable found on the `PATH`.
pub struct GitCli {
    dir: PathBuf,
}

impl GitCli {
    /// Runs git as if started in `dir`.
    pub fn new(dir: PathBuf) -> GitCli {
        GitCli { dir }
    }
}

impl Git for GitCli {
    fn run(&self, args: &[&str]) -> Result<String, GitError> {
        let output = Command::new("git")
            .arg("-C")
            .arg(&self.dir)
            .args(args)
            // Scry reads while the user's own git commands may run: never take
            // the index lock just to refresh it.
            .env("GIT_OPTIONAL_LOCKS", "0")
            .output()
            .map_err(GitError::Spawn)?;
        if !output.status.success() {
            return Err(GitError::Failed {
                command: args.join(" "),
                stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
            });
        }
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
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
