//! `git-scry resolve`: the merge tool that `git mergetool` runs, in the
//! terminal until the app exists.
//!
//! Exit codes, read by git when `mergetool.scry.trustExitCode` is true:
//! 0 the file is resolved and written, 1 conflicts are left and nothing was
//! written, 2 the files could not be loaded or a `--take` was invalid.

use std::io::{self, BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use scry_core::git::GitCli;
use scry_core::operation;
use scry_core::resolution::CandidateKind;
use scry_core::session::{self, MergeSession, MergetoolFiles};

pub struct Options {
    pub files: MergetoolFiles,
    /// `<conflict>=<candidate>` choices given on the command line.
    pub takes: Vec<String>,
    pub suggested: bool,
}

pub fn run(options: Options) -> ExitCode {
    let files = &options.files;
    let git = GitCli::new(PathBuf::from("."));
    let (ours, theirs, summary) = match operation::current(&git) {
        Ok(Some(operation)) => {
            let theirs = match &operation.theirs {
                Some(side) => side.label(),
                None => String::from("theirs"),
            };
            (operation.ours.label(), theirs, Some(operation.summary()))
        }
        _ => (String::from("ours"), String::from("theirs"), None),
    };

    let mut session = match session::load(&git, files, &ours, &theirs) {
        Ok(session) => session,
        Err(error) => {
            eprintln!("git-scry: {error}");
            return ExitCode::from(2);
        }
    };
    let name = files.merged.display().to_string();

    if options.suggested {
        session.apply_suggestions();
    }
    for take in &options.takes {
        if let Err(message) = apply_take(&mut session, take) {
            eprintln!("git-scry: --take {take}: {message}");
            return ExitCode::from(2);
        }
    }

    let interactive = options.takes.is_empty() && !options.suggested && io::stdin().is_terminal();
    if interactive {
        println!("{name}");
        if let Some(summary) = &summary {
            println!("{summary}");
        }
        if session.rebuilt() {
            println!("(conflicts rebuilt from the three versions, to show the base)");
        }
        if !ask_each_conflict(&mut session, &ours, &theirs) {
            println!("Nothing written; {name} stays conflicted.");
            return ExitCode::from(1);
        }
    }

    let left = session.unresolved();
    if left > 0 {
        println!("{left} conflict(s) left in {name}; nothing written.");
        return ExitCode::from(1);
    }
    let Some(text) = session.merged_text() else {
        return ExitCode::from(1);
    };
    if interactive && !confirm(&format!("Write {name}?")) {
        println!("Nothing written; {name} stays conflicted.");
        return ExitCode::from(1);
    }
    if let Err(error) = session::write_atomically(&files.merged, &text) {
        eprintln!("git-scry: cannot write {name}: {error}");
        return ExitCode::from(2);
    }
    println!("{name} resolved.");
    ExitCode::SUCCESS
}

/// Applies one `<conflict>=<candidate>`, where `<conflict>` is a number or
/// an ID.
fn apply_take(session: &mut MergeSession, take: &str) -> Result<(), String> {
    let Some((reference, candidate)) = take.split_once('=') else {
        return Err(String::from("expected <conflict>=<candidate>"));
    };
    let Some(index) = session.find(reference) else {
        return Err(format!("no conflict {reference:?}"));
    };
    let Some(kind) = CandidateKind::from_id(candidate) else {
        return Err(format!("unknown candidate {candidate:?}"));
    };
    session
        .choose(index, kind)
        .map_err(|error| error.to_string())
}

/// The key that picks a candidate: the same digits as the app's shortcuts
/// (⌥⌘1 ours, ⌥⌘2 theirs, ⌥⌘3 both, ⌥⌘0 base, ⌥⌘9 none).
fn key(kind: CandidateKind) -> &'static str {
    match kind {
        CandidateKind::Ours => "1",
        CandidateKind::Theirs => "2",
        CandidateKind::OursThenTheirs => "3",
        CandidateKind::TheirsThenOurs => "4",
        CandidateKind::Neither => "9",
        CandidateKind::Base => "0",
    }
}

fn title(kind: CandidateKind) -> &'static str {
    match kind {
        CandidateKind::Ours => "ours",
        CandidateKind::Theirs => "theirs",
        CandidateKind::OursThenTheirs => "both, ours first",
        CandidateKind::TheirsThenOurs => "both, theirs first",
        CandidateKind::Neither => "neither",
        CandidateKind::Base => "base",
    }
}

/// Asks for a candidate for each conflict without a choice. Returns false
/// when the user quits.
fn ask_each_conflict(session: &mut MergeSession, ours: &str, theirs: &str) -> bool {
    let count = session.conflict_count();
    for index in 0..count {
        if session.choice(index).is_some() {
            continue;
        }
        let conflict = session.conflict(index);
        let analysis = session.analysis(index);
        println!();
        println!(
            "Conflict {} of {count} · {} · line {}",
            index + 1,
            analysis.id,
            conflict.start_line
        );
        println!("{}", analysis.kind.describe(ours, theirs));
        print_side(&format!("[1] ours · {ours}"), &conflict.ours.text);
        if let Some(base) = &conflict.base {
            print_side("[0] base", &base.text);
        }
        print_side(&format!("[2] theirs · {theirs}"), &conflict.theirs.text);

        let choices: Vec<String> = analysis
            .candidates
            .iter()
            .map(|candidate| format!("[{}] {}", key(candidate.kind), title(candidate.kind)))
            .collect();
        println!("{}", choices.join("  "));
        let suggestion = analysis.suggestion;
        let hint = match suggestion {
            Some(kind) => format!(" · Enter: suggested by rule, {}", title(kind)),
            None => String::new(),
        };
        println!("[s] skip  [q] quit without writing{hint}");

        loop {
            let Some(answer) = prompt("> ") else {
                return false;
            };
            let answer = answer.trim();
            if answer == "q" {
                return false;
            }
            if answer == "s" {
                break;
            }
            let kind = if answer.is_empty() {
                suggestion
            } else {
                session
                    .analysis(index)
                    .candidates
                    .iter()
                    .map(|candidate| candidate.kind)
                    .find(|kind| key(*kind) == answer)
            };
            match kind {
                Some(kind) if session.choose(index, kind).is_ok() => break,
                _ => println!("Type one of the keys in brackets."),
            }
        }
    }
    true
}

fn print_side(heading: &str, text: &str) {
    println!("  {heading}");
    if text.is_empty() {
        println!("  │ (nothing)");
    }
    for line in text.lines() {
        println!("  │ {line}");
    }
}

/// Reads one line from the terminal; `None` at the end of input.
fn prompt(question: &str) -> Option<String> {
    print!("{question}");
    io::stdout().flush().ok()?;
    let mut line = String::new();
    match io::stdin().lock().read_line(&mut line) {
        Ok(0) | Err(_) => None,
        Ok(_) => Some(line),
    }
}

fn confirm(question: &str) -> bool {
    match prompt(&format!("{question} [Y/n] ")) {
        Some(answer) => matches!(answer.trim(), "" | "y" | "Y" | "yes"),
        None => false,
    }
}

/// The four paths in the order git passes them.
pub fn files(base: &Path, local: &Path, remote: &Path, merged: &Path) -> MergetoolFiles {
    MergetoolFiles {
        base: base.to_path_buf(),
        local: local.to_path_buf(),
        remote: remote.to_path_buf(),
        merged: merged.to_path_buf(),
    }
}
