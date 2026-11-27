//! `git-scry`: the command-line entry point of Scry.
//!
//! Git runs any `git-<name>` executable on the PATH as `git <name>`, so this
//! binary is also available as `git scry`.

mod diff;
mod difftool;
mod highlight;
mod log;
mod open;
mod resolve;
mod wait;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use scry_core::git::GitCli;
use scry_core::operation;
use scry_core::overview;
use scry_core::review::{self, Resolution};
use scry_core::session::{self, LoadError};
use scry_core::syntax::ParseCheck;

#[derive(Parser)]
#[command(
    name = "git-scry",
    version,
    about = "Merge and review changes with git"
)]
struct Cli {
    /// Without a command, opens the app on the current repository, as
    /// `git scry open` does.
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Open the Scry app on the repository, from any folder in it: on its
    /// merge, rebase, cherry-pick or revert in progress, or else on what
    /// is not committed, as `git scry diff` does. `git scry` alone does
    /// the same.
    ///
    /// Exits with 0 once the app is open, 2 outside a repository or when
    /// the app cannot be found.
    Open {
        /// A folder in the repository; the current one by default.
        path: Option<PathBuf>,
        /// The app, when not $SCRY_APP, the bundle this git-scry belongs
        /// to, /Applications/Scry.app or ~/Applications/Scry.app.
        #[arg(long, value_name = "PATH", require_equals = true)]
        app: Option<PathBuf>,
    },
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
    /// Serve the operation in progress and its conflicts to an agent, over
    /// the Model Context Protocol on standard input and output. Claude Code
    /// starts it through Scry's plugin rather than you running it:
    ///
    ///   claude plugin marketplace add hnic0las/scry
    ///   claude plugin install git-scry@scry
    ///
    /// Answers about the repository of the folder the agent starts it in.
    /// Writes only the conflicts the agent resolves: it stages, continues
    /// and commits nothing.
    Mcp,
    /// Wait for what the user asks an agent in Scry (Explain on a
    /// conflict), and print each ask on one line:
    ///
    ///   Scry asks: explain src/Large.swift c12
    ///
    /// Scry's Claude Code plugin runs `wait --follow` in the background of
    /// the user's session, which answers through `git-scry mcp`. While it
    /// runs, Scry shows that an agent is watching.
    ///
    /// Asks already waiting are printed at once. Exits with 0 after
    /// printing, 2 outside a repository; with --follow, keeps printing
    /// until stopped, and stays silent outside a repository.
    Wait {
        /// Keep waiting after an ask, printing each new one.
        #[arg(long)]
        follow: bool,
    },
    /// Read a change in the Scry app: the files that differ, and each one
    /// whole with its added and removed lines.
    ///
    ///   git scry diff               everything not committed
    ///   git scry diff --staged      what the next commit holds
    ///   git scry diff main          the working copy against main
    ///   git scry diff main..topic   topic against main
    ///   git scry diff main...topic  what topic changed on its own
    ///
    /// Unlike `git diff`, no revision means everything not committed:
    /// staged, not staged and new files that are not ignored. With
    /// --print, the change is printed in the terminal instead. Exits with
    /// 0, or 2 when git cannot answer or the app cannot be found.
    #[command(verbatim_doc_comment)]
    Diff {
        /// A commit to compare the working copy with, or a range: a..b,
        /// or a...b for b against its merge base with a.
        revisions: Option<String>,
        /// Compare the index, not the working copy, with the commit.
        #[arg(long)]
        staged: bool,
        /// Print in the terminal: the list of files (--print), or the
        /// list and each file whole (--print=unified, --print=split).
        #[arg(
            long,
            value_name = "LAYOUT",
            value_enum,
            num_args = 0..=1,
            require_equals = true,
            default_missing_value = "list"
        )]
        print: Option<diff::Layout>,
        /// The app, when not $SCRY_APP, the bundle this git-scry belongs
        /// to, /Applications/Scry.app or ~/Applications/Scry.app.
        #[arg(
            long,
            value_name = "PATH",
            require_equals = true,
            conflicts_with = "print"
        )]
        app: Option<PathBuf>,
        /// Only these files or folders, after `--`.
        #[arg(last = true)]
        paths: Vec<String>,
    },
    /// Read one file's history in the Scry app: what is not committed of
    /// it, then the commits that touched it, followed through its
    /// renames, and what each one changed.
    ///
    ///   git scry log src/sync.rs
    ///
    /// With --print, the points are listed in the terminal instead.
    /// Exits with 0, or 2 when the file has no history, git cannot
    /// answer or the app cannot be found.
    #[command(verbatim_doc_comment)]
    Log {
        /// The file, from the current folder.
        path: String,
        /// List the points in the terminal.
        #[arg(long)]
        print: bool,
        /// The app, when not $SCRY_APP, the bundle this git-scry belongs
        /// to, /Applications/Scry.app or ~/Applications/Scry.app.
        #[arg(
            long,
            value_name = "PATH",
            require_equals = true,
            conflicts_with = "print"
        )]
        app: Option<PathBuf>,
    },
    /// Show two files, or two folders, in the Scry app, as the diff tool
    /// `git difftool` runs:
    ///
    ///   git config diff.tool scry
    ///   git config difftool.scry.cmd 'git-scry difftool -- "$LOCAL" "$REMOTE" "$MERGED"'
    ///   git config difftool.prompt false
    ///
    /// Git gives a diff tool two files and nothing else, so the window
    /// shows the diff alone, and git opens one window per file, one after
    /// the other; with `git difftool --dir-diff`, one window lists every
    /// file. `git scry diff` needs no setting and shows the whole change.
    /// Waits for the window to close. Exits with 0, or 2 when the app
    /// cannot be found.
    #[command(verbatim_doc_comment)]
    Difftool {
        local: PathBuf,
        remote: PathBuf,
        /// The file's path in the repository, for the window's title.
        merged: Option<String>,
        /// The app, when not $SCRY_APP, the bundle this git-scry belongs
        /// to, /Applications/Scry.app or ~/Applications/Scry.app.
        #[arg(long, value_name = "PATH", require_equals = true)]
        app: Option<PathBuf>,
    },
    /// Print a file with the syntax highlighting the app shows.
    ///
    /// Exits with 0, or 2 when the file cannot be read or Scry has no
    /// grammar or highlight query for it.
    Highlight {
        /// File to show.
        file: PathBuf,
        /// Print each line's spans and their roles instead of colours.
        #[arg(long)]
        spans: bool,
    },
    /// Resolve a conflicted file, as the merge tool `git mergetool` runs:
    ///
    ///   git config merge.tool scry
    ///   git config mergetool.scry.cmd 'git-scry resolve -- "$BASE" "$LOCAL" "$REMOTE" "$MERGED"'
    ///   git config mergetool.scry.trustExitCode true
    ///
    /// Asks for a candidate per conflict in the terminal, unless --take or
    /// --suggested are given, or opens the Scry app with --app. Writes the
    /// file only when every conflict is resolved. Exits with 0 when
    /// written, 1 when conflicts are left (nothing written), 2 on error.
    Resolve {
        base: PathBuf,
        local: PathBuf,
        remote: PathBuf,
        merged: PathBuf,
        /// Choose without asking: <conflict>=<candidate>, where <conflict>
        /// is a number or an ID and <candidate> one of ours, theirs,
        /// ours-then-theirs, theirs-then-ours, neither, base. Repeatable.
        #[arg(long = "take", value_name = "CONFLICT=CANDIDATE")]
        takes: Vec<String>,
        /// Choose the rule suggestion wherever there is one, without asking.
        #[arg(long)]
        suggested: bool,
        /// Without asking, write the file even when it does not parse.
        #[arg(long)]
        allow_invalid: bool,
        /// Resolve in the Scry app instead of the terminal, and wait for
        /// its window to close. The app is $SCRY_APP, the bundle this
        /// git-scry belongs to, /Applications/Scry.app or
        /// ~/Applications/Scry.app, unless a path is given
        /// (--app=/path/to/Scry.app).
        #[arg(
            long,
            value_name = "PATH",
            num_args = 0..=1,
            require_equals = true,
            conflicts_with_all = ["takes", "suggested", "allow_invalid"]
        )]
        app: Option<Option<PathBuf>>,
    },
}

fn main() -> ExitCode {
    quiet_on_closed_pipe();
    let cli = Cli::parse();
    let Some(command) = cli.command else {
        return open::run(None, None);
    };
    match command {
        Command::Open { path, app } => open::run(path, app),
        Command::Conflicts { file } => list_conflicts(&file),
        Command::Session => show_session(),
        Command::Mcp => serve_mcp(),
        Command::Wait { follow } => wait::run(follow),
        Command::Highlight { file, spans } => highlight::run(&file, spans),
        Command::Difftool {
            local,
            remote,
            merged,
            app,
        } => difftool::run(difftool::Options {
            local,
            remote,
            name: merged,
            app,
        }),
        Command::Log { path, print, app } => log::run(log::Options { path, print, app }),
        Command::Diff {
            revisions,
            staged,
            print,
            app,
            paths,
        } => diff::run(diff::Options {
            staged,
            revisions,
            paths,
            print,
            app,
        }),
        Command::Resolve {
            base,
            local,
            remote,
            merged,
            app: Some(app),
            ..
        } => resolve::run_in_app(app, resolve::files(&base, &local, &remote, &merged)),
        Command::Resolve {
            base,
            local,
            remote,
            merged,
            takes,
            suggested,
            allow_invalid,
            app: None,
        } => resolve::run(resolve::Options {
            files: resolve::files(&base, &local, &remote, &merged),
            takes,
            suggested,
            allow_invalid,
        }),
    }
}

/// The exit code of a tool killed by `SIGPIPE` (128 + 13).
const CLOSED_PIPE: i32 = 141;

/// `println!` and `eprintln!` panic when the reader of the output is gone
/// (`git scry highlight … | head -1`). Like other command-line tools,
/// git-scry then stops quietly instead of printing a panic: this hook
/// exits on that one panic and leaves every other to the default hook.
/// It exits with 141, as a tool killed by `SIGPIPE` does, never 0: the
/// command did not finish, and git trusts a merge tool's 0 as resolved.
fn quiet_on_closed_pipe() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let payload = info.payload();
        let message = payload
            .downcast_ref::<String>()
            .map(String::as_str)
            .or_else(|| payload.downcast_ref::<&str>().copied())
            .unwrap_or("");
        if is_closed_pipe(message) {
            std::process::exit(CLOSED_PIPE);
        }
        default(info);
    }));
}

/// Whether a panic's message is `println!` or `eprintln!` failing because
/// the reader is gone. The system's code is read, not its text, which
/// Windows writes in the user's language: `EPIPE` on Unix; on Windows
/// "the pipe is being closed" (232) or "the pipe has been ended" (109).
fn is_closed_pipe(message: &str) -> bool {
    const CODES: &[&str] = if cfg!(windows) {
        &["(os error 232)", "(os error 109)"]
    } else {
        &["(os error 32)"]
    };
    message.starts_with("failed printing to ") && CODES.iter().any(|code| message.contains(code))
}

fn serve_mcp() -> ExitCode {
    let git = GitCli::discover(PathBuf::from("."));
    match scry_mcp::serve(std::io::stdin().lock(), std::io::stdout().lock(), git) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("git-scry: {error}");
            ExitCode::from(2)
        }
    }
}

fn list_conflicts(file: &Path) -> ExitCode {
    let session = match session::open_file(&GitCli::beside(file), file) {
        Ok(session) => session,
        Err(LoadError::Parse(error)) => {
            eprintln!("git-scry: {}: {error}", file.display());
            return ExitCode::from(2);
        }
        Err(error) => {
            eprintln!("git-scry: {error}");
            return ExitCode::from(2);
        }
    };

    let count = session.conflict_count();
    if count == 0 {
        println!("{}: no conflict", file.display());
        return ExitCode::SUCCESS;
    }

    println!("{}: {count} conflict(s)", file.display());
    session.check_all();
    for index in 0..count {
        let conflict = session.conflict(index);
        let analysis = session.analysis(index);
        let ours = side_name(&conflict.ours.label, "ours");
        let theirs = side_name(&conflict.theirs.label, "theirs");
        println!();
        println!(
            "  {}. {}  line {}  {}",
            analysis.number,
            analysis.id,
            analysis.start_line,
            analysis.kind.describe(ours, theirs)
        );
        let base = match &conflict.base {
            Some(base) => format!(" · base {}", line_count(&base.text)),
            None => String::new(),
        };
        println!(
            "     ours {ours} {} · theirs {theirs} {}{base}",
            line_count(&conflict.ours.text),
            line_count(&conflict.theirs.text),
        );
        let candidates: Vec<String> = analysis
            .candidates
            .iter()
            .map(|candidate| {
                let mark = match candidate.parse {
                    ParseCheck::Parses => " ✓",
                    ParseCheck::DoesNotParse { .. } => " ✗",
                    ParseCheck::NotChecked => "",
                };
                format!("{}{mark}", candidate.kind.id())
            })
            .collect();
        println!("     candidates: {}", candidates.join(", "));
        if let Some(suggestion) = analysis.suggestion {
            println!("     suggested by rule: {}", suggestion.id());
        }
    }
    if let Some(language) = session.language() {
        println!();
        println!(
            "✓ parses as {} with this candidate, ✗ does not",
            language.name()
        );
    }
    ExitCode::from(1)
}

fn show_session() -> ExitCode {
    let git = GitCli::discover(PathBuf::from("."));
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
        let root = overview::root(&git).ok();
        for path in paths {
            let found = root
                .as_deref()
                .and_then(|root| review::find(&git, root, &path).ok().flatten());
            let Some(found) = found else {
                println!("  {path}");
                continue;
            };
            if found.changed_outside {
                println!("  {path}  (changed outside the conflicts)");
            } else {
                println!("  {path}");
            }
            for conflict in found.conflicts {
                let line = conflict
                    .line
                    .map(|line| format!("  line {line}"))
                    .unwrap_or_default();
                println!(
                    "    {}{line}  {}",
                    conflict.id,
                    resolution(&conflict.resolution)
                );
            }
        }
    }
    ExitCode::SUCCESS
}

/// What was done to a conflict, in a few words.
fn resolution(resolution: &Resolution) -> String {
    match resolution {
        Resolution::Open => String::from("open"),
        Resolution::Candidate(kind) => kind.id().to_string(),
        Resolution::Text(text) => format!("own text {}", line_count(text)),
        Resolution::Unknown => String::from("unknown"),
    }
}

/// The label git wrote on a marker, or a fallback when it wrote none.
fn side_name<'a>(label: &'a str, fallback: &'a str) -> &'a str {
    if label.is_empty() { fallback } else { label }
}

fn line_count(text: &str) -> String {
    match text.lines().count() {
        1 => String::from("(1 line)"),
        count => format!("({count} lines)"),
    }
}
