//! What a commit changes: its files with their added and removed line
//! counts, and one file's unified diff, read only. The overview shows them
//! while the user writes a reworded commit's message.

use crate::git::{Git, GitError};

/// A file the commit changes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangedFile {
    pub path: String,
    /// The last component of the path.
    pub name: String,
    /// The folder, with a trailing slash; empty at the root.
    pub directory: String,
    /// Lines added and removed; `None` for a binary file.
    pub added: Option<usize>,
    pub removed: Option<usize>,
}

/// What a line of a diff is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineKind {
    /// A hunk's start: its text is the heading git found, such as the
    /// enclosing function, or the line ranges when there is none.
    Hunk,
    Context,
    Added,
    Removed,
}

/// One line of a file's diff.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffLine {
    pub kind: LineKind,
    /// In the file before the commit; `None` for added lines and hunks.
    pub old_number: Option<usize>,
    /// In the file after the commit; `None` for removed lines and hunks.
    pub new_number: Option<usize>,
    /// The line without its `+`, `-` or space.
    pub text: String,
}

/// The files `commit` changes, in git's order. Renames show as a deletion
/// and an addition, so every path is one the commit has or had.
pub fn files(git: &dyn Git, commit: &str) -> Result<Vec<ChangedFile>, GitError> {
    let output = git.run(&[
        "show",
        "--format=",
        "--numstat",
        "-z",
        "--no-renames",
        "--no-color",
        "--no-show-signature",
        commit,
        "--",
    ])?;
    Ok(parse_numstat(&output))
}

/// The diff `commit` makes to `path`.
pub fn diff(git: &dyn Git, commit: &str, path: &str) -> Result<Vec<DiffLine>, GitError> {
    let output = git.output(&[
        "show",
        "--format=",
        "--no-renames",
        "--no-color",
        "--no-ext-diff",
        "--no-show-signature",
        commit,
        "--",
        path,
    ])?;
    if output.status != Some(0) {
        return Err(GitError::Failed {
            command: format!("show {commit} -- {path}"),
            stderr: output.stderr,
        });
    }
    Ok(parse_diff(&String::from_utf8_lossy(&output.stdout)))
}

/// `git show --numstat -z`: "added<TAB>removed<TAB>path" per file, each
/// ended by a NUL; "-" counts for a binary file.
pub fn parse_numstat(text: &str) -> Vec<ChangedFile> {
    text.split('\0')
        .filter_map(|entry| {
            // The first entry follows the empty format's newline.
            let entry = entry.trim_start_matches('\n');
            let mut parts = entry.splitn(3, '\t');
            let added = parts.next()?;
            let removed = parts.next()?;
            let path = parts.next()?;
            if path.is_empty() {
                return None;
            }
            let (directory, name) = match path.rfind('/') {
                Some(slash) => (&path[..=slash], &path[slash + 1..]),
                None => ("", path),
            };
            Some(ChangedFile {
                path: path.to_string(),
                name: name.to_string(),
                directory: directory.to_string(),
                added: added.parse().ok(),
                removed: removed.parse().ok(),
            })
        })
        .collect()
}

/// A unified diff's lines, numbered from each hunk's header. The file
/// header before the first hunk and "\ No newline at end of file" are
/// left out.
pub fn parse_diff(text: &str) -> Vec<DiffLine> {
    let mut lines = Vec::new();
    let mut old = 0;
    let mut new = 0;
    let mut in_hunk = false;
    for line in text.lines() {
        if let Some(header) = line.strip_prefix("@@ ") {
            // "@@ -12,7 +12,8 @@ final class SyncEngine"
            let (ranges, heading) = header.split_once(" @@").unwrap_or((header, ""));
            let mut ranges = ranges.split_whitespace();
            old = start(ranges.next());
            new = start(ranges.next());
            let heading = heading.trim();
            lines.push(DiffLine {
                kind: LineKind::Hunk,
                old_number: None,
                new_number: None,
                text: if heading.is_empty() {
                    ranges_text(header)
                } else {
                    heading.to_string()
                },
            });
            in_hunk = true;
            continue;
        }
        if !in_hunk || line.starts_with('\\') {
            continue;
        }
        // A new file's header, if several files are in the text.
        if line.starts_with("diff --git ") {
            in_hunk = false;
            continue;
        }
        let (kind, rest) = match line.chars().next() {
            Some('+') => (LineKind::Added, &line[1..]),
            Some('-') => (LineKind::Removed, &line[1..]),
            Some(' ') => (LineKind::Context, &line[1..]),
            // An empty context line whose space an editor trimmed.
            None => (LineKind::Context, ""),
            Some(_) => continue,
        };
        let (old_number, new_number) = match kind {
            LineKind::Added => (None, Some(new)),
            LineKind::Removed => (Some(old), None),
            _ => (Some(old), Some(new)),
        };
        if old_number.is_some() {
            old += 1;
        }
        if new_number.is_some() {
            new += 1;
        }
        lines.push(DiffLine {
            kind,
            old_number,
            new_number,
            text: rest.to_string(),
        });
    }
    lines
}

/// "-12,7" or "+12" gives 12.
fn start(range: Option<&str>) -> usize {
    range
        .map(|range| range.trim_start_matches(['-', '+']))
        .and_then(|range| range.split(',').next())
        .and_then(|number| number.parse().ok())
        .unwrap_or(0)
}

/// "-12,7 +12,8" from the header, when git found no heading.
fn ranges_text(header: &str) -> String {
    header
        .split(" @@")
        .next()
        .unwrap_or(header)
        .trim()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each file comes with its name, folder and counts, and a binary file
    /// with no counts, so the list can say "binary" instead of "+0 −0".
    #[test]
    fn numstat_lists_files_with_counts() {
        let files = parse_numstat("\n54\t0\tSources/Sync/SyncQueue.swift\0-\t-\ticon.png\0");
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].name, "SyncQueue.swift");
        assert_eq!(files[0].directory, "Sources/Sync/");
        assert_eq!(files[0].added, Some(54));
        assert_eq!(files[0].removed, Some(0));
        assert_eq!(files[1].directory, "");
        assert_eq!(files[1].added, None);
    }

    /// Lines are numbered in the old and new file from the hunk's header,
    /// so the diff can show both numbers as git's own tools do.
    #[test]
    fn diff_lines_are_numbered_from_the_hunk() {
        let lines = parse_diff(
            "diff --git a/a.swift b/a.swift\nindex 1..2 100644\n--- a/a.swift\n+++ b/a.swift\n\
             @@ -12,3 +12,3 @@ final class SyncEngine\n     let client\n-    var pending\n+    var outbox\n \
             \n\\ No newline at end of file\n@@ -30 +30,2 @@\n+    flush()\n",
        );
        let kinds: Vec<LineKind> = lines.iter().map(|line| line.kind).collect();
        assert_eq!(
            kinds,
            [
                LineKind::Hunk,
                LineKind::Context,
                LineKind::Removed,
                LineKind::Added,
                LineKind::Context,
                LineKind::Hunk,
                LineKind::Added
            ]
        );
        assert_eq!(lines[0].text, "final class SyncEngine");
        assert_eq!(lines[1].text, "    let client");
        assert_eq!((lines[2].old_number, lines[2].new_number), (Some(13), None));
        assert_eq!((lines[3].old_number, lines[3].new_number), (None, Some(13)));
        assert_eq!(
            (lines[4].old_number, lines[4].new_number),
            (Some(14), Some(14))
        );
        assert_eq!(lines[5].text, "-30 +30,2");
        assert_eq!(lines[6].new_number, Some(30));
    }
}
