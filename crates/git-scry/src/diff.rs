//! `git-scry diff`: opens the app's Changes window on a change, or with
//! `--print` gives the changed files and their rows as text, to read a
//! change, or check the core's diff, without the app.

use std::ffi::OsString;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::ValueEnum;
use scry_core::diff::{
    self, ChangedFile, FileDiff, FileState, Group, LineKind, Marks, Scope, TextDiff,
};
use scry_core::git::{Git, GitCli};

/// What `--print` prints.
#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Layout {
    /// The changed files with their counts.
    List,
    /// The list, then each file whole in one column: in each block the
    /// removed lines, then the added ones.
    Unified,
    /// The list, then each file whole with the old side beside the new.
    Split,
}

pub struct Options {
    pub staged: bool,
    pub revisions: Option<String>,
    pub paths: Vec<String>,
    pub print: Option<Layout>,
    pub app: Option<PathBuf>,
}

pub fn run(options: Options) -> ExitCode {
    let read = match Read::new(&options) {
        Ok(read) => read,
        Err(message) => {
            eprintln!("git-scry: {message}");
            return ExitCode::from(2);
        }
    };
    let Some(layout) = options.print else {
        return open(options, read);
    };
    match print(&read, layout) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("git-scry: {message}");
            ExitCode::from(2)
        }
    }
}

/// What was asked, read once: a wrong revision or a folder outside any
/// repository is reported in the terminal, before the app is opened.
struct Read {
    git: GitCli,
    root: PathBuf,
    /// The paths asked, from the repository's root.
    paths: Vec<String>,
    scope: Scope,
    files: Vec<ChangedFile>,
}

impl Read {
    fn new(options: &Options) -> Result<Read, String> {
        let here = GitCli::discover(PathBuf::from("."));
        let text = |error: scry_core::git::GitError| error.to_string();
        let root = PathBuf::from(
            here.run(&["rev-parse", "--show-toplevel"])
                .map_err(text)?
                .trim_end_matches(['\n', '\r']),
        );
        // Paths are given from the current folder, and git lists them
        // from the root.
        let prefix = here.run(&["rev-parse", "--show-prefix"]).map_err(text)?;
        let prefix = prefix.trim_end_matches(['\n', '\r']);
        let paths: Vec<String> = options
            .paths
            .iter()
            .map(|path| format!("{prefix}{path}"))
            .collect();
        let git = here.in_dir(root.clone());
        let scope = Scope::of(&git, options.staged, options.revisions.as_deref())
            .map_err(|error| error.to_string())?;
        let files = diff::changed_files(&git, &root, &scope.comparison, &paths).map_err(text)?;
        Ok(Read {
            git,
            root,
            paths,
            scope,
            files,
        })
    }
}

/// Opens the app's Changes window on what was asked: the repository's
/// root, then the same options, each path after `--path`.
fn open(options: Options, read: Read) -> ExitCode {
    let mut arguments: Vec<OsString> =
        vec![crate::open::DIFF.into(), read.root.clone().into_os_string()];
    if options.staged {
        arguments.push("--staged".into());
    }
    if let Some(revisions) = options.revisions {
        arguments.push("--revisions".into());
        arguments.push(revisions.into());
    }
    for path in read.paths {
        arguments.push("--path".into());
        arguments.push(path.into());
    }
    crate::open::open_app(options.app, &read.root, arguments)
}

fn print(read: &Read, layout: Layout) -> Result<(), String> {
    let Read {
        git, root, files, ..
    } = read;
    let (comparison, title) = (&read.scope.comparison, read.scope.sentence());
    if files.is_empty() {
        println!("{title}");
        println!("Nothing to show: no file differs.");
        return Ok(());
    }
    let added: usize = files.iter().filter_map(|file| file.added).sum();
    let removed: usize = files.iter().filter_map(|file| file.removed).sum();
    println!(
        "{title} · {} · +{added} −{removed}",
        count(files.len(), "file")
    );
    let mut group = None;
    for file in files {
        if file.group != group {
            group = file.group;
            match group {
                Some(Group::Staged) => println!("Staged"),
                Some(Group::NotStaged) => println!("Not staged"),
                Some(Group::Untracked) => println!("Untracked"),
                None => {}
            }
        }
        println!("  {}", list_line(file));
    }
    if layout == Layout::List {
        return Ok(());
    }
    for file in files {
        println!();
        println!("== {}", file.path);
        match diff::file_diff(git, root, comparison, file, Marks::Auto)
            .map_err(|error| error.to_string())?
        {
            FileDiff::Text(text) if layout == Layout::Unified => print_unified(&text),
            FileDiff::Text(text) => print_split(&text),
            FileDiff::Binary { old_size, new_size } => {
                println!("Binary file, {} → {}", size(old_size), size(new_size));
            }
            FileDiff::Submodule => println!("A repository of its own."),
            FileDiff::Conflicted => {
                println!("This file has conflicts: resolve them first (git scry).");
            }
        }
    }
    Ok(())
}

/// "M  src/a.rs  +3 −1 · partly staged"
fn list_line(file: &ChangedFile) -> String {
    let letter = match file.state {
        _ if file.binary => 'B',
        FileState::Added => 'A',
        FileState::Modified => 'M',
        FileState::Deleted => 'D',
        FileState::Renamed => 'R',
        FileState::Untracked => 'U',
        FileState::TypeChanged => 'T',
        FileState::Conflicted => '!',
    };
    let mut notes: Vec<String> = Vec::new();
    match (file.added, file.removed) {
        _ if file.binary => notes.push(String::from("binary")),
        (Some(added), Some(removed)) => notes.push(format!("+{added} −{removed}")),
        _ if file.state == FileState::Conflicted => notes.push(String::from("conflicted")),
        _ if file.submodule => notes.push(String::from("a repository of its own")),
        _ => notes.push(String::from("not counted")),
    }
    if file.partly_staged {
        notes.push(String::from("partly staged"));
    }
    if let Some(old_path) = &file.old_path {
        notes.push(format!("was {old_path}"));
    }
    format!("{letter}  {}  {}", file.path, notes.join(" · "))
}

/// The line with its changed words between `open` and `close`.
fn marked(line: &str, words: &[std::ops::Range<usize>], open: &str, close: &str) -> String {
    let mut out = String::new();
    let mut position = 0;
    for word in words {
        out.push_str(&line[position..word.start]);
        out.push_str(open);
        out.push_str(&line[word.clone()]);
        out.push_str(close);
        position = word.end;
    }
    out.push_str(&line[position..]);
    out
}

fn number(number: Option<usize>) -> String {
    number.map(|number| number.to_string()).unwrap_or_default()
}

/// Old number, new number, sign, line; changed words as git's word diff
/// writes them, `[-removed-]` and `{+added+}`.
fn print_unified(text: &TextDiff) {
    for line in text.unified() {
        let (sign, content) = match line.kind {
            LineKind::Unchanged => (' ', text.new_lines[line.new.unwrap_or(1) - 1].clone()),
            LineKind::Removed => (
                '-',
                marked(
                    &text.old_lines[line.old.unwrap_or(1) - 1],
                    &line.words,
                    "[-",
                    "-]",
                ),
            ),
            LineKind::Added => (
                '+',
                marked(
                    &text.new_lines[line.new.unwrap_or(1) - 1],
                    &line.words,
                    "{+",
                    "+}",
                ),
            ),
        };
        println!(
            "{:>5} {:>5} {sign} {content}",
            number(line.old),
            number(line.new)
        );
    }
    print_notes(text);
}

/// The old side beside the new one, each cut at 60 characters.
fn print_split(text: &TextDiff) {
    const WIDTH: usize = 60;
    let cut = |line: &str| -> String { line.chars().take(WIDTH).collect() };
    for row in &text.rows {
        let old = row
            .old
            .map(|number| marked(&text.old_lines[number - 1], &row.old_words, "[-", "-]"));
        let new = row
            .new
            .map(|number| marked(&text.new_lines[number - 1], &row.new_words, "{+", "+}"));
        println!(
            "{:>5} {:<WIDTH$} │ {:>5} {}",
            number(row.old),
            cut(old.as_deref().unwrap_or("")),
            number(row.new),
            cut(new.as_deref().unwrap_or(""))
        );
    }
    print_notes(text);
}

fn print_notes(text: &TextDiff) {
    if text.old_missing_final_newline != text.new_missing_final_newline {
        let side = if text.new_missing_final_newline {
            "new"
        } else {
            "old"
        };
        println!("(no line ending after the last line of the {side} side)");
    }
    if text.marks_skipped {
        println!("(large file: lines are not paired and words are not marked)");
    }
}

fn size(bytes: Option<u64>) -> String {
    match bytes {
        Some(bytes) => format!("{bytes} bytes"),
        None => String::from("nothing"),
    }
}

fn count(count: usize, noun: &str) -> String {
    if count == 1 {
        format!("1 {noun}")
    } else {
        format!("{count} {noun}s")
    }
}
