//! `git-scry`: the command-line entry point of Scry.
//!
//! Git runs any `git-<name>` executable on the PATH as `git <name>`, so this
//! binary is also available as `git scry`.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use scry_core::conflict;
use scry_core::git::GitCli;
use scry_core::operation;

#[derive(Parser)]
#[command(
    name = "git-scry",
    version,
    about = "Merge and review changes with git"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// List the conflict regions of a file that contains conflict markers.
    ///
    /// Exits with 0 when the file has no conflict, 1 when it has some,
    /// and 2 when it cannot be read or parsed.
    Conflicts {
        /// File to inspect.
        file: PathBuf,
    },
    /// Show the merge, rebase, cherry-pick or revert in progress, who is on
    /// each side, and the files left with conflicts.
    ///
    /// Exits with 0, or 2 when git cannot answer (for example outside a
    /// repository).
    Session,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.command {
        Command::Conflicts { file } => list_conflicts(&file),
        Command::Session => show_session(),
    }
}

fn list_conflicts(file: &PathBuf) -> ExitCode {
    let text = match std::fs::read_to_string(file) {
        Ok(text) => text,
        Err(error) => {
            eprintln!("git-scry: cannot read {}: {error}", file.display());
            return ExitCode::from(2);
        }
    };
    let segments = match conflict::parse(&text) {
        Ok(segments) => segments,
        Err(error) => {
            eprintln!("git-scry: {}: {error}", file.display());
            return ExitCode::from(2);
        }
    };

    let conflicts: Vec<_> = conflict::conflicts(&segments).collect();
    if conflicts.is_empty() {
        println!("{}: no conflict", file.display());
        return ExitCode::SUCCESS;
    }

    println!("{}: {} conflict(s)", file.display(), conflicts.len());
    for conflict in conflicts {
        let base = match &conflict.base {
            Some(base) => format!(", base {} line(s)", base.text.lines().count()),
            None => String::new(),
        };
        println!(
            "  line {}: ours {:?} {} line(s), theirs {:?} {} line(s){base}",
            conflict.start_line,
            conflict.ours.label,
            conflict.ours.text.lines().count(),
            conflict.theirs.label,
            conflict.theirs.text.lines().count(),
        );
    }
    ExitCode::from(1)
}

fn show_session() -> ExitCode {
    let git = GitCli::new(PathBuf::from("."));
    let result = operation::current(&git)
        .and_then(|operation| Ok((operation, operation::conflicted_paths(&git)?)));
    let (operation, paths) = match result {
        Ok(found) => found,
        Err(error) => {
            eprintln!("git-scry: {error}");
            return ExitCode::from(2);
        }
    };

    let Some(operation) = operation else {
        println!("No merge, rebase, cherry-pick or revert in progress.");
        return ExitCode::SUCCESS;
    };

    println!("{}", operation.summary());
    println!();
    let sides = [
        ("ours", Some(&operation.ours)),
        ("theirs", operation.theirs.as_ref()),
    ];
    for (git_name, side) in sides {
        let Some(side) = side else {
            println!("  {git_name:<6}  (no commit being applied)");
            continue;
        };
        println!("  {git_name:<6}  {}", side.label());
        if let Some(detail) = side.detail() {
            println!("          {detail}");
        }
    }
    println!();
    if paths.is_empty() {
        println!("No file left with conflicts.");
    } else {
        println!("Files with conflicts ({}):", paths.len());
        for path in paths {
            println!("  {path}");
        }
    }
    ExitCode::SUCCESS
}
