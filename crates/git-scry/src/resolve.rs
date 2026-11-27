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
use scry_core::git::{Git, GitCli};
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
    // `git mergetool` runs the tool from the repository's root, with
    // paths from there.
    let git = GitCli::discover(PathBuf::from("."));
    let interactive = io::stdin().is_terminal();
    let mut input = io::stdin().lock();
    let mut output = io::stdout();
    let mut console = Console {
        interactive,
        input: &mut input,
        output: &mut output,
    };
    ExitCode::from(run_with(&options, &git, &mut console))
}

/// Where `resolve` asks its questions: the terminal, or prepared answers
/// in tests.
struct Console<'a> {
    /// Whether someone can answer: questions are asked only then.
    interactive: bool,
    input: &'a mut dyn BufRead,
    output: &'a mut dyn Write,
}

impl Console<'_> {
    /// Writes a line. A closed output (`git-scry … | head`) is no reason to
    /// stop resolving.
    fn say(&mut self, text: &str) {
        let _ = writeln!(self.output, "{text}");
    }

    /// Asks a question and reads one line; `None` at the end of input.
    fn prompt(&mut self, question: &str) -> Option<String> {
        write!(self.output, "{question}").ok()?;
        self.output.flush().ok()?;
        let mut line = String::new();
        match self.input.read_line(&mut line) {
            Ok(0) | Err(_) => None,
            Ok(_) => Some(line),
        }
    }
}

/// `run` with git and the console given; returns the exit code.
fn run_with(options: &Options, git: &dyn Git, console: &mut Console) -> u8 {
    let files = &options.files;
    let (mut session, operation) = match session::open_mergetool(git, files) {
        Ok(opened) => opened,
        Err(error) => {
            eprintln!("git-scry: {error}");
            return 2;
        }
    };
    let (ours, theirs) = presentation::side_labels(operation.as_ref());
    let summary = operation.as_ref().map(|operation| operation.summary());
    let name = files.merged.display().to_string();

    if options.suggested
        && let Err(error) = session.apply(Command::ApplySuggestions)
    {
        eprintln!("git-scry: {error}");
        return 2;
    }
    for take in &options.takes {
        if let Err(message) = apply_take(&mut session, take) {
            eprintln!("git-scry: --take {take}: {message}");
            return 2;
        }
    }

    let interactive = options.takes.is_empty() && !options.suggested && console.interactive;
    if interactive {
        console.say(&name);
        if let Some(summary) = &summary {
            console.say(summary);
        }
        match session.origin() {
            Origin::Rebuilt => {
                console.say("(conflicts rebuilt from the three versions, to show the base)")
            }
            Origin::EditedFile => console.say("(the file was edited since git wrote it: its conflicts are used as they are, without the base)"),
            Origin::File | Origin::Resolved => {}
        }
        if !ask_each_conflict(console, &mut session, &ours, &theirs) {
            console.say(&format!("Nothing written; {name} stays conflicted."));
            return 1;
        }
    }

    let left = session.unresolved();
    if left > 0 {
        console.say(&format!(
            "{left} conflict(s) left in {name}; nothing written."
        ));
        return 1;
    }
    if let ParseCheck::DoesNotParse { line } = session.merged_check() {
        let language = session.language().map_or("", |language| language.name());
        let problem = format!("{name} does not parse as {language} near line {line}");
        let write_anyway = if interactive {
            ask_yes_no(console, &format!("{problem}. Write it anyway?"), false)
        } else {
            options.allow_invalid
        };
        if !write_anyway {
            console.say(&format!("{problem}; nothing written."));
            return 1;
        }
    } else if interactive && !ask_yes_no(console, &format!("Write {name}?"), true) {
        console.say(&format!("Nothing written; {name} stays conflicted."));
        return 1;
    }
    match session.apply(Command::Save { overwrite: false }) {
        Ok(_) => {}
        Err(CommandError::Save(SaveError::ChangedOnDisk(_))) => {
            let problem = format!("{name} changed on disk since Scry read it");
            if !(interactive && ask_yes_no(console, &format!("{problem}. Overwrite it?"), false)) {
                console.say(&format!("{problem}; nothing written."));
                return 1;
            }
            if let Err(error) = session.apply(Command::Save { overwrite: true }) {
                eprintln!("git-scry: {error}");
                return 2;
            }
        }
        Err(error) => {
            eprintln!("git-scry: {error}");
            return 2;
        }
    }
    console.say(&format!("{name} resolved."));
    0
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
pub fn run_bundle(
    app: &Path,
    mut arguments: Vec<std::ffi::OsString>,
) -> std::io::Result<Option<i32>> {
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

pub fn run_executable(
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
fn ask_each_conflict(
    console: &mut Console,
    session: &mut MergeSession,
    ours: &str,
    theirs: &str,
) -> bool {
    loop {
        if !ask_unchosen_conflicts(console, session, ours, theirs) {
            return false;
        }
        let skipped = session.unresolved();
        if skipped == 0
            || !ask_yes_no(
                console,
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
fn ask_unchosen_conflicts(
    console: &mut Console,
    session: &mut MergeSession,
    ours: &str,
    theirs: &str,
) -> bool {
    let count = session.conflict_count();
    for index in 0..count {
        if session.choice(index).is_some() {
            continue;
        }
        let conflict = session.conflict(index);
        let analysis = session.analysis(index);
        console.say("");
        console.say(&format!(
            "Conflict {} of {count} · {} · line {}",
            index + 1,
            analysis.id,
            conflict.start_line
        ));
        console.say(&analysis.kind.describe(ours, theirs));
        print_side(console, &format!("[1] ours · {ours}"), &conflict.ours.text);
        if let Some(base) = &conflict.base {
            print_side(console, "[0] base", &base.text);
        }
        print_side(
            console,
            &format!("[2] theirs · {theirs}"),
            &conflict.theirs.text,
        );

        let choices: Vec<String> = analysis.candidates.iter().map(choice_label).collect();
        console.say(&choices.join("  "));
        if analysis
            .candidates
            .iter()
            .any(|candidate| matches!(candidate.parse, ParseCheck::DoesNotParse { .. }))
        {
            console.say("✗ the file would not parse with this candidate");
        }
        let suggestion = analysis.suggestion;
        let hint = match suggestion {
            Some(kind) => format!(" · Enter: suggested by rule, {}", kind.title()),
            None => String::new(),
        };
        console.say(&format!("[s] skip  [q] quit without writing{hint}"));

        loop {
            let Some(answer) = console.prompt("> ") else {
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
                _ => console.say("Type one of the keys in brackets."),
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

fn print_side(console: &mut Console, heading: &str, text: &str) {
    console.say(&format!("  {heading}"));
    if text.is_empty() {
        console.say("  │ (nothing)");
    }
    for line in text.lines() {
        console.say(&format!("  │ {line}"));
    }
}

/// Asks a yes-or-no question; Enter gives `default`.
fn ask_yes_no(console: &mut Console, question: &str, default: bool) -> bool {
    let hint = if default { "[Y/n]" } else { "[y/N]" };
    yes_or_no(
        console.prompt(&format!("{question} {hint} ")).as_deref(),
        default,
    )
}

/// The meaning of an answer to a yes-or-no question: Enter gives
/// `default`; anything but a yes, or no answer at all (end of input), is no.
fn yes_or_no(answer: Option<&str>, default: bool) -> bool {
    match answer.map(str::trim) {
        Some("") => default,
        Some("y" | "Y" | "yes") => true,
        _ => false,
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::{Cursor, Read};

    /// `git mergetool`'s four files in a folder, the merged one written by
    /// `git merge-file` in git's default style, and git with the user's
    /// configuration left out.
    struct Merge {
        dir: tempfile::TempDir,
        files: MergetoolFiles,
    }

    impl Merge {
        /// Two conflicts, `A1`/`A2` and `B1`/`B2`, around a kept line.
        fn two_conflicts() -> Merge {
            Merge::new(
                "a.txt",
                "a\nkeep\nb\n",
                "A1\nkeep\nB1\n",
                "A2\nkeep\nB2\n",
                None,
            )
        }

        fn new(name: &str, base: &str, ours: &str, theirs: &str, merged: Option<&str>) -> Merge {
            let dir = tempfile::tempdir().unwrap();
            fs::write(dir.path().join("gitconfig"), "").unwrap();
            let files = files(
                &dir.path().join("BASE"),
                &dir.path().join("LOCAL"),
                &dir.path().join("REMOTE"),
                &dir.path().join(name),
            );
            fs::write(&files.base, base).unwrap();
            fs::write(&files.local, ours).unwrap();
            fs::write(&files.remote, theirs).unwrap();
            let merge = Merge { dir, files };
            let merged = match merged {
                Some(merged) => merged.to_string(),
                None => merge.merge_file(),
            };
            fs::write(&merge.files.merged, merged).unwrap();
            merge
        }

        fn git(&self) -> GitCli {
            let config = self.dir.path().join("gitconfig");
            GitCli::new(self.dir.path().to_path_buf())
                .with_env("GIT_CONFIG_GLOBAL", &config.display().to_string())
                .with_env("GIT_CONFIG_NOSYSTEM", "1")
        }

        fn merge_file(&self) -> String {
            let output = self
                .git()
                .output(&["merge-file", "-p", "LOCAL", "BASE", "REMOTE"])
                .unwrap();
            String::from_utf8(output.stdout).unwrap()
        }

        fn merged(&self) -> String {
            fs::read_to_string(&self.files.merged).unwrap()
        }

        /// `resolve` in a terminal answering `answers`: its exit code and
        /// what it wrote.
        fn resolve(&self, answers: &str) -> (u8, String) {
            self.resolve_from(&mut Cursor::new(answers.as_bytes().to_vec()))
        }

        fn resolve_from(&self, input: &mut dyn BufRead) -> (u8, String) {
            self.resolve_with(&[], false, input)
        }

        /// `resolve` in a terminal with `--take` choices or `--suggested`.
        fn resolve_with(
            &self,
            takes: &[&str],
            suggested: bool,
            input: &mut dyn BufRead,
        ) -> (u8, String) {
            let options = Options {
                files: files(
                    &self.files.base,
                    &self.files.local,
                    &self.files.remote,
                    &self.files.merged,
                ),
                takes: takes.iter().map(|take| take.to_string()).collect(),
                suggested,
                allow_invalid: false,
            };
            let mut output = Capped(Vec::new());
            let mut console = Console {
                interactive: true,
                input,
                output: &mut output,
            };
            let code = run_with(&options, &self.git(), &mut console);
            (code, String::from_utf8(output.0).unwrap())
        }
    }

    /// The console's output in tests, refused beyond 1 MB. A mutant that keeps asking
    /// (a `prompt` that always answers) then stops at once, as at the end of input; an
    /// unbounded buffer filled memory and swap until the machine crashed (2026-11-15).
    struct Capped(Vec<u8>);

    impl Write for Capped {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            if self.0.len() + bytes.len() > 1 << 20 {
                return Err(io::Error::other("test output over 1 MB"));
            }
            self.0.extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    /// A console whose output fills up ends the questions like a closed input, rather than
    /// asking forever: nothing is written.
    #[test]
    fn a_full_output_ends_the_questions() {
        let merge = Merge::two_conflicts();
        let written = merge.merged();
        let mut output = Capped(vec![0; 1 << 20]);
        let mut input = Cursor::new(b"1\n2\n\n".to_vec());
        let mut console = Console {
            interactive: true,
            input: &mut input,
            output: &mut output,
        };
        let options = Options {
            files: files(
                &merge.files.base,
                &merge.files.local,
                &merge.files.remote,
                &merge.files.merged,
            ),
            takes: Vec::new(),
            suggested: false,
            allow_invalid: false,
        };
        assert_eq!(run_with(&options, &merge.git(), &mut console), 1);
        assert_eq!(merge.merged(), written);
    }

    /// In a terminal each conflict is shown with its sides and keys, then the file is
    /// written once the user confirms; Enter confirms, since writing is what they came
    /// for.
    #[test]
    fn each_conflict_is_asked_then_the_file_written() {
        let merge = Merge::two_conflicts();
        let (code, output) = merge.resolve("1\n2\n\n");
        assert_eq!(code, 0, "{output}");
        assert_eq!(merge.merged(), "A1\nkeep\nB2\n");
        assert!(output.contains("Conflict 1 of 2"), "{output}");
        assert!(output.contains("Conflict 2 of 2"), "{output}");
        assert!(output.contains("  │ A1\n"), "{output}");
        assert!(output.contains("[1] Ours"), "{output}");
        assert!(output.contains("[Y/n]"), "{output}");
        assert!(output.ends_with("resolved.\n"), "{output}");
    }

    /// Choices given on the command line (`--take`, `--suggested`) are the whole answer,
    /// even in a terminal: nothing is asked, so a script or an alias runs unattended.
    #[test]
    fn choices_given_on_the_command_line_ask_nothing() {
        let merge = Merge::two_conflicts();
        let mut input = Cursor::new(Vec::new());
        let (code, output) = merge.resolve_with(&["1=ours", "2=theirs"], false, &mut input);
        assert_eq!(code, 0, "{output}");
        assert!(!output.contains("[Y/n]"), "{output}");
        assert_eq!(merge.merged(), "A1\nkeep\nB2\n");

        let merge = Merge::two_conflicts();
        let (code, output) = merge.resolve_with(&[], true, &mut input);
        assert_eq!(code, 1, "{output}");
        assert!(!output.contains("Conflict 1 of 2"), "{output}");
        assert!(output.contains("2 conflict(s) left"), "{output}");
    }

    /// Declining to write, quitting, or the input closing all leave the file as git wrote
    /// it, conflicted, with exit code 1 so git keeps it unmerged.
    #[test]
    fn declining_quitting_or_closing_writes_nothing() {
        for answers in ["1\n2\nn\n", "q\n", "1\n", ""] {
            let merge = Merge::two_conflicts();
            let written = merge.merged();
            let (code, output) = merge.resolve(answers);
            assert_eq!(code, 1, "{answers:?}: {output}");
            assert_eq!(merge.merged(), written, "{answers:?}");
            assert!(output.contains("Nothing written"), "{answers:?}: {output}");
        }
    }

    /// A skipped conflict is offered again, keeping the choices made; refusing to go back
    /// leaves the file unwritten, as a half-resolved file cannot be written.
    #[test]
    fn skipped_conflicts_are_offered_again() {
        let merge = Merge::two_conflicts();
        let (code, output) = merge.resolve("s\n2\n\n1\n\n");
        assert_eq!(code, 0, "{output}");
        assert!(output.contains("1 conflict(s) skipped"), "{output}");
        assert_eq!(merge.merged(), "A1\nkeep\nB2\n");

        let merge = Merge::two_conflicts();
        let written = merge.merged();
        let (code, output) = merge.resolve("s\ns\nn\n");
        assert_eq!(code, 1, "{output}");
        assert!(output.contains("2 conflict(s) left"), "{output}");
        assert_eq!(merge.merged(), written);
    }

    /// An unknown key, or Enter where no rule suggests a candidate, asks again rather than
    /// choosing something the user did not ask for.
    #[test]
    fn an_unknown_answer_asks_again() {
        let merge = Merge::two_conflicts();
        let (code, output) = merge.resolve("x\n\n7\n2\n1\n\n");
        assert_eq!(code, 0, "{output}");
        assert_eq!(
            output.matches("Type one of the keys in brackets.").count(),
            3
        );
        assert_eq!(merge.merged(), "A2\nkeep\nB1\n");
    }

    /// A choice that makes the file fail to parse is marked, and writing it needs an
    /// explicit yes: Enter keeps the file conflicted.
    #[test]
    fn a_file_that_does_not_parse_is_written_only_on_yes() {
        let merged = "func upload() {\n    send()\n}\n\n<<<<<<< ours\nfunc cancel() {\n    queue.cancelAll()\n\
            ||||||| base\n=======\nfunc pause() {\n    queue.suspend()\n>>>>>>> theirs\n}\n";
        let new = || {
            Merge::new(
                "a.swift",
                "func upload() {\n    send()\n}\n",
                "func upload() {\n    send()\n}\n\nfunc cancel() {\n    queue.cancelAll()\n}\n",
                "func upload() {\n    send()\n}\n\nfunc pause() {\n    queue.suspend()\n}\n",
                Some(merged),
            )
        };
        let merge = new();
        let (code, output) = merge.resolve("3\n\n");
        assert_eq!(code, 1, "{output}");
        assert!(output.contains("✗"), "{output}");
        assert!(output.contains("does not parse as Swift"), "{output}");
        assert!(output.contains("[y/N]"), "{output}");
        assert_eq!(merge.merged(), merged);

        let merge = new();
        let (code, output) = merge.resolve("3\ny\n");
        assert_eq!(code, 0, "{output}");
        assert!(merge.merged().contains("queue.cancelAll()\nfunc pause()"));
    }

    /// Answers that change the merged file on disk as soon as the first question is read,
    /// as another editor or a `git checkout` would while the user is choosing.
    struct ChangedMeanwhile {
        answers: Cursor<Vec<u8>>,
        path: std::path::PathBuf,
        changed: bool,
    }

    impl Read for ChangedMeanwhile {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            self.answers.read(buffer)
        }
    }

    impl BufRead for ChangedMeanwhile {
        fn fill_buf(&mut self) -> io::Result<&[u8]> {
            if !self.changed {
                fs::write(&self.path, "edited elsewhere\n")?;
                self.changed = true;
            }
            self.answers.fill_buf()
        }

        fn consume(&mut self, amount: usize) {
            self.answers.consume(amount);
        }
    }

    /// A file changed on disk while the user was choosing is overwritten only on an
    /// explicit yes; otherwise the other change is kept.
    #[test]
    fn a_file_changed_meanwhile_is_overwritten_only_on_yes() {
        for (last, overwritten) in [("\n", false), ("y\n", true)] {
            let merge = Merge::two_conflicts();
            let mut input = ChangedMeanwhile {
                answers: Cursor::new(format!("1\n2\n\n{last}").into_bytes()),
                path: merge.files.merged.clone(),
                changed: false,
            };
            let (code, output) = merge.resolve_from(&mut input);
            assert!(output.contains("changed on disk"), "{output}");
            if overwritten {
                assert_eq!(code, 0, "{output}");
                assert_eq!(merge.merged(), "A1\nkeep\nB2\n");
            } else {
                assert_eq!(code, 1, "{output}");
                assert_eq!(merge.merged(), "edited elsewhere\n");
            }
        }
    }

    /// Enter takes the question's default, so "Write it?" defaults to yes and "Write it
    /// anyway?" to no; a closed input never says yes for the user.
    #[test]
    fn enter_takes_the_default_and_no_answer_is_no() {
        for default in [true, false] {
            assert_eq!(yes_or_no(Some("\n"), default), default);
            assert!(yes_or_no(Some("y\n"), default));
            assert!(yes_or_no(Some(" Y \n"), default));
            assert!(yes_or_no(Some("yes\n"), default));
            assert!(!yes_or_no(Some("n\n"), default));
            assert!(!yes_or_no(Some("sure\n"), default));
            assert!(!yes_or_no(None, default));
        }
    }

    /// A candidate reads as its key, its title, and a cross when the file would not parse
    /// with it: the key is what the user types.
    #[test]
    fn a_choice_shows_its_key_title_and_parse_mark() {
        let candidate = |kind, parse| Candidate {
            kind,
            text: String::new(),
            parse,
            aliases: Vec::new(),
        };
        assert_eq!(
            choice_label(&candidate(
                CandidateKind::OursThenTheirs,
                ParseCheck::DoesNotParse { line: 3 }
            )),
            "[3] Both · ours first ✗"
        );
        assert_eq!(
            choice_label(&candidate(CandidateKind::Theirs, ParseCheck::Parses)),
            "[2] Theirs"
        );
    }
}
