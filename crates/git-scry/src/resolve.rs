//! `git-scry resolve`: the merge tool that `git mergetool` runs, in the
//! terminal until the app exists.
//!
//! Exit codes, read by git when `mergetool.scry.trustExitCode` is true:
//! 0 the file is resolved and written, 1 conflicts are left and nothing was
//! written, 2 the files could not be loaded or a `--take` was invalid.

use std::io::{self, BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use scry_core::command::{Command, CommandError};
use scry_core::git::GitCli;
use scry_core::presentation;
use scry_core::resolution::{Candidate, CandidateKind};
use scry_core::session::{self, MergeSession, MergetoolFiles, Origin, SaveError};
use scry_core::syntax::ParseCheck;

pub struct Options {
    pub files: MergetoolFiles,
    /// `<conflict>=<candidate>` choices given on the command line.
    pub takes: Vec<String>,
    pub suggested: bool,
    /// Write even when the merged file does not parse.
    pub allow_invalid: bool,
}

pub fn run(options: Options) -> ExitCode {
    let files = &options.files;
    // `git mergetool` runs the tool from the repository's root, with
    // paths from there.
    let git = GitCli::discover(PathBuf::from("."));
    let (mut session, operation) = match session::open_mergetool(&git, files) {
        Ok(opened) => opened,
        Err(error) => {
            eprintln!("git-scry: {error}");
            return ExitCode::from(2);
        }
    };
    let (ours, theirs) = presentation::side_labels(operation.as_ref());
    let summary = operation.as_ref().map(|operation| operation.summary());
    let name = files.merged.display().to_string();

    if options.suggested
        && let Err(error) = session.apply(Command::ApplySuggestions)
    {
        eprintln!("git-scry: {error}");
        return ExitCode::from(2);
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
        match session.origin() {
            Origin::Rebuilt => {
                println!("(conflicts rebuilt from the three versions, to show the base)")
            }
            Origin::EditedFile => println!(
                "(the file was edited since git wrote it: its conflicts are used as they are, without the base)"
            ),
            Origin::File => {}
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
    if let ParseCheck::DoesNotParse { line } = session.merged_check() {
        let language = session.language().map_or("", |language| language.name());
        let problem = format!("{name} does not parse as {language} near line {line}");
        let write_anyway = if interactive {
            ask_yes_no(&format!("{problem}. Write it anyway?"), false)
        } else {
            options.allow_invalid
        };
        if !write_anyway {
            println!("{problem}; nothing written.");
            return ExitCode::from(1);
        }
    } else if interactive && !ask_yes_no(&format!("Write {name}?"), true) {
        println!("Nothing written; {name} stays conflicted.");
        return ExitCode::from(1);
    }
    match session.apply(Command::Save { overwrite: false }) {
        Ok(_) => {}
        Err(CommandError::Save(SaveError::ChangedOnDisk(_))) => {
            let problem = format!("{name} changed on disk since Scry read it");
            if !(interactive && ask_yes_no(&format!("{problem}. Overwrite it?"), false)) {
                println!("{problem}; nothing written.");
                return ExitCode::from(1);
            }
            if let Err(error) = session.apply(Command::Save { overwrite: true }) {
                eprintln!("git-scry: {error}");
                return ExitCode::from(2);
            }
        }
        Err(error) => {
            eprintln!("git-scry: {error}");
            return ExitCode::from(2);
        }
    }
    println!("{name} resolved.");
    ExitCode::SUCCESS
}

/// Opens the Scry app on the files and waits for it, passing its answer
/// on: 0 once the file is written, 1 when the window closed without. `app`
/// is the app bundle, or an executable to run as the app; when not given,
/// the bundle is looked for.
pub fn run_in_app(app: Option<PathBuf>, files: MergetoolFiles) -> ExitCode {
    let Some(app) = app.or_else(find_app) else {
        eprintln!(
            "git-scry: Scry.app not found in /Applications or ~/Applications; \
             pass --app=<path> or set SCRY_APP"
        );
        return ExitCode::from(2);
    };
    // Each path follows an option: AppKit takes a bare path for a file to
    // open, and SwiftUI then opens no window. The app does not run in git's
    // directory, so the paths are absolute.
    let mut arguments = Vec::new();
    for (option, path) in [
        ("--base", &files.base),
        ("--local", &files.local),
        ("--remote", &files.remote),
        ("--merged", &files.merged),
    ] {
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
    let answer = if app.is_dir() {
        run_bundle(&app, arguments)
    } else {
        run_executable(&app, arguments)
    };
    match answer {
        Ok(Some(0)) => ExitCode::SUCCESS,
        Ok(Some(1)) => ExitCode::from(1),
        Ok(_) => {
            eprintln!("git-scry: {} stopped without an answer", app.display());
            ExitCode::from(2)
        }
        Err(error) => {
            eprintln!("git-scry: cannot run {}: {error}", app.display());
            ExitCode::from(2)
        }
    }
}

/// Opens the bundle through Launch Services (`open`), which brings the app
/// to the front: started as a plain process from the terminal, macOS keeps
/// the terminal active and the window opens behind it. `open` does not
/// pass the app's exit code on, so the app writes its answer to a file.
fn run_bundle(app: &Path, mut arguments: Vec<std::ffi::OsString>) -> std::io::Result<Option<i32>> {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or(0);
    let result = std::env::temp_dir().join(format!("scry-answer-{}-{stamp}", std::process::id()));
    arguments.push("--result".into());
    arguments.push(result.clone().into_os_string());
    // -W waits for the app to quit; -n starts a new one even when Scry is
    // already open on another file.
    let status = std::process::Command::new("open")
        .args(["-W", "-n", "-a"])
        .arg(app)
        .arg("--args")
        .args(&arguments)
        .status()?;
    let answer = std::fs::read_to_string(&result).ok();
    let _ = std::fs::remove_file(&result);
    if !status.success() {
        return Ok(None);
    }
    Ok(answer.and_then(|answer| answer.trim().parse().ok()))
}

fn run_executable(
    program: &Path,
    arguments: Vec<std::ffi::OsString>,
) -> std::io::Result<Option<i32>> {
    let status = std::process::Command::new(program)
        .args(&arguments)
        .status()?;
    Ok(status.code())
}

/// The app: `$SCRY_APP`; the bundle this executable lives in
/// (`Scry.app/Contents/Helpers/git-scry`, also when reached through a
/// symbolic link on the PATH); then the usual Applications folders.
pub fn find_app() -> Option<PathBuf> {
    if let Some(app) = std::env::var_os("SCRY_APP") {
        return Some(PathBuf::from(app));
    }
    if let Some(bundle) = std::env::current_exe()
        .and_then(std::fs::canonicalize)
        .ok()
        .and_then(|exe| {
            exe.ancestors()
                .find(|folder| {
                    folder
                        .extension()
                        .is_some_and(|extension| extension == "app")
                })
                .map(Path::to_path_buf)
        })
    {
        return Some(bundle);
    }
    let mut places = vec![PathBuf::from("/Applications/Scry.app")];
    if let Some(home) = std::env::var_os("HOME") {
        places.push(PathBuf::from(home).join("Applications/Scry.app"));
    }
    places.into_iter().find(|place| place.is_dir())
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
        .apply(Command::Choose {
            conflict: index,
            candidate: kind,
        })
        .map(|_| ())
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
        CandidateKind::OursThenTheirsClosed => "5",
        CandidateKind::TheirsThenOursClosed => "6",
    }
}

/// Asks for a candidate for each conflict without a choice, then offers
/// to go back to the skipped ones, keeping the choices made: once git
/// gets an exit code other than 0 it puts its own file back, so writing
/// the file half resolved would keep nothing either. Returns false when
/// the user quits.
fn ask_each_conflict(session: &mut MergeSession, ours: &str, theirs: &str) -> bool {
    loop {
        if !ask_unchosen_conflicts(session, ours, theirs) {
            return false;
        }
        let skipped = session.unresolved();
        if skipped == 0
            || !ask_yes_no(
                &format!(
                    "{skipped} conflict(s) skipped, so nothing can be written yet. Go back to them?"
                ),
                true,
            )
        {
            return true;
        }
    }
}

/// One pass over the conflicts without a choice. Returns false when the
/// user quits.
fn ask_unchosen_conflicts(session: &mut MergeSession, ours: &str, theirs: &str) -> bool {
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

        let choices: Vec<String> = analysis.candidates.iter().map(choice_label).collect();
        println!("{}", choices.join("  "));
        if analysis
            .candidates
            .iter()
            .any(|candidate| matches!(candidate.parse, ParseCheck::DoesNotParse { .. }))
        {
            println!("✗ the file would not parse with this candidate");
        }
        let suggestion = analysis.suggestion;
        let hint = match suggestion {
            Some(kind) => format!(" · Enter: suggested by rule, {}", kind.title()),
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
                Some(kind) if choose(session, index, kind) => break,
                _ => println!("Type one of the keys in brackets."),
            }
        }
    }
    true
}

/// Chooses a candidate; false when the conflict does not offer it.
fn choose(session: &mut MergeSession, index: usize, kind: CandidateKind) -> bool {
    session
        .apply(Command::Choose {
            conflict: index,
            candidate: kind,
        })
        .is_ok()
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

/// Asks a yes-or-no question; Enter gives `default`.
fn ask_yes_no(question: &str, default: bool) -> bool {
    let hint = if default { "[Y/n]" } else { "[y/N]" };
    match prompt(&format!("{question} {hint} ")) {
        Some(answer) => match answer.trim() {
            "" => default,
            "y" | "Y" | "yes" => true,
            _ => false,
        },
        None => false,
    }
}

/// "[3] Both · ours first ✗": the key, the title, and a mark when the file
/// would not parse with this candidate.
fn choice_label(candidate: &Candidate) -> String {
    let mark = match candidate.parse {
        ParseCheck::DoesNotParse { .. } => " ✗",
        ParseCheck::Parses | ParseCheck::NotChecked => "",
    };
    format!("[{}] {}{mark}", key(candidate.kind), candidate.kind.title())
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
