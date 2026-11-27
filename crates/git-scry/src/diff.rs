//! `git-scry diff --print`: the changed files and their rows as text, to
//! read a change, or check the core's diff, without the app.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::ValueEnum;
use scry_core::diff::{
    self, ChangedFile, Comparison, FileDiff, FileState, Group, LineKind, Marks, TextDiff,
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
}

pub fn run(options: Options) -> ExitCode {
    let Some(layout) = options.print else {
        eprintln!(
            "git-scry: the Changes window is not built yet. \
             Use --print to read the change in the terminal."
        );
        return ExitCode::from(2);
    };
    match print(&options, layout) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("git-scry: {message}");
            ExitCode::from(2)
        }
    }
}

fn print(options: &Options, layout: Layout) -> Result<(), String> {
    let here = GitCli::discover(PathBuf::from("."));
    let text = |error: scry_core::git::GitError| error.to_string();
    let root = PathBuf::from(
        here.run(&["rev-parse", "--show-toplevel"])
            .map_err(text)?
            .trim_end_matches(['\n', '\r']),
    );
    // Paths are given from the current folder, and git lists them from
    // the root.
    let prefix = here.run(&["rev-parse", "--show-prefix"]).map_err(text)?;
    let prefix = prefix.trim_end_matches(['\n', '\r']);
    let paths: Vec<String> = options
        .paths
        .iter()
        .map(|path| format!("{prefix}{path}"))
        .collect();
    let git = here.in_dir(root.clone());

    let (comparison, title) = comparison(&git, options)?;
    let files = diff::changed_files(&git, &root, &comparison, &paths).map_err(text)?;
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
    for file in &files {
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
    for file in &files {
        println!();
        println!("== {}", file.path);
        match diff::file_diff(&git, &root, &comparison, file, Marks::Auto)
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

/// What `--staged` and the revisions ask to compare, and its title.
fn comparison(git: &dyn Git, options: &Options) -> Result<(Comparison, String), String> {
    let text = |error: scry_core::git::GitError| error.to_string();
    let or_head = |name: &str| if name.is_empty() { "HEAD" } else { name }.to_string();
    let revisions = options.revisions.as_deref();
    if let Some((old, new)) = revisions.and_then(|revisions| revisions.split_once("...")) {
        if options.staged {
            return Err(String::from("--staged takes one commit, not a range"));
        }
        let (old, new) = (or_head(old), or_head(new));
        let comparison = Comparison::since_merge_base(git, &old, &new).map_err(text)?;
        return Ok((
            comparison,
            format!("{new}, against its merge base with {old}"),
        ));
    }
    if let Some((old, new)) = revisions.and_then(|revisions| revisions.split_once("..")) {
        if options.staged {
            return Err(String::from("--staged takes one commit, not a range"));
        }
        let (old, new) = (or_head(old), or_head(new));
        let title = format!("{new}, against {old}");
        return Ok((Comparison::Commits { old, new }, title));
    }
    match (options.staged, revisions) {
        (true, None) => Ok((
            Comparison::staged(git).map_err(text)?,
            String::from("Staged, against HEAD"),
        )),
        (true, Some(base)) => Ok((
            Comparison::Staged {
                base: base.to_string(),
            },
            format!("Staged, against {base}"),
        )),
        (false, None) => Ok((
            Comparison::uncommitted(git).map_err(text)?,
            String::from("Not committed, against HEAD"),
        )),
        (false, Some(base)) => Ok((
            Comparison::WorkingCopy {
                base: base.to_string(),
            },
            format!("Working copy, against {base}"),
        )),
    }
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
