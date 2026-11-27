//! Who last wrote each line of a file: the blame gutter of the Changes
//! window. Git is asked for `--incremental` output, which names the
//! lines commit by commit as it finds them, the newest first: a long
//! file shows what is known while the rest is read.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::diff::Comparison;
use crate::git::{Git, GitError};

/// The version of the file that is blamed: the new side of what the
/// window shows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Target {
    /// The file as it is on disk.
    WorkingCopy,
    /// The file as it is staged.
    Index,
    /// The file in a commit.
    Commit(String),
}

impl Target {
    /// The new side of `comparison`.
    pub fn of(comparison: &Comparison) -> Target {
        match comparison {
            Comparison::WorkingCopy { .. } | Comparison::Unstaged => Target::WorkingCopy,
            Comparison::Staged { .. } => Target::Index,
            Comparison::Commits { new, .. } => Target::Commit(new.clone()),
        }
    }

    /// Lines can be in no commit yet.
    pub fn can_be_uncommitted(&self) -> bool {
        !matches!(self, Target::Commit(_))
    }
}

/// A commit that wrote lines of the file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlameCommit {
    pub id: String,
    pub author: String,
    /// Author time, in seconds since the Unix epoch.
    pub time: i64,
    pub subject: String,
    /// The file's name in that commit.
    pub path: String,
}

impl BlameCommit {
    pub fn short_id(&self) -> &str {
        self.id.get(..7).unwrap_or(&self.id)
    }
}

/// Consecutive lines one commit wrote.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Lines {
    /// The first line, from 1, in the blamed version.
    pub first: usize,
    pub count: usize,
    /// The commit's place among `Parser::commits`; `None` for lines not
    /// committed yet.
    pub commit: Option<usize>,
}

/// Reads `git blame --incremental` line by line.
#[derive(Debug, Default)]
pub struct Parser {
    commits: Vec<BlameCommit>,
    places: HashMap<String, usize>,
    /// The entry being read: its lines and its commit's id.
    current: Option<(Lines, String)>,
    /// The fields of a commit seen for the first time.
    author: String,
    time: i64,
    subject: String,
}

impl Parser {
    pub fn new() -> Parser {
        Parser::default()
    }

    /// The commits named so far, in the order git named them.
    pub fn commits(&self) -> &[BlameCommit] {
        &self.commits
    }

    /// Takes one line of git's output; gives the lines of an entry when
    /// this line ends it. An entry is a header `<id> <original line>
    /// <final line> <count>`, the commit's fields the first time it is
    /// named, and `filename`, always last.
    pub fn feed(&mut self, line: &str) -> Option<Lines> {
        if let Some(path) = line.strip_prefix("filename ") {
            let (mut lines, id) = self.current.take()?;
            if id.chars().any(|c| c != '0') {
                let place = match self.places.get(&id) {
                    Some(place) => *place,
                    None => {
                        self.commits.push(BlameCommit {
                            id: id.clone(),
                            author: std::mem::take(&mut self.author),
                            time: self.time,
                            subject: std::mem::take(&mut self.subject),
                            path: path.to_string(),
                        });
                        self.places.insert(id, self.commits.len() - 1);
                        self.commits.len() - 1
                    }
                };
                lines.commit = Some(place);
            }
            return Some(lines);
        }
        if self.current.is_some() {
            if let Some(author) = line.strip_prefix("author ") {
                self.author = author.to_string();
                return None;
            }
            if let Some(time) = line.strip_prefix("author-time ") {
                self.time = time.parse().unwrap_or(0);
                return None;
            }
            if let Some(subject) = line.strip_prefix("summary ") {
                self.subject = subject.to_string();
                return None;
            }
        }
        let mut fields = line.split(' ');
        let (Some(id), Some(_), Some(first), Some(count), None) = (
            fields.next(),
            fields.next(),
            fields.next(),
            fields.next(),
            fields.next(),
        ) else {
            return None;
        };
        if id.len() < 40 || !id.chars().all(|c| c.is_ascii_hexdigit()) {
            return None;
        }
        let (Ok(first), Ok(count)) = (first.parse(), count.parse()) else {
            return None;
        };
        self.current = Some((
            Lines {
                first,
                count,
                commit: None,
            },
            id.to_string(),
        ));
        None
    }
}

/// Blames `path` (as git in `git`'s directory sees it) as `target` holds
/// it. `each` gets the lines as git finds them, with the commits named
/// so far; it answers `false` to stop.
///
/// Fails when git has no history for the file: a file never committed,
/// which the caller shows as not committed when the target can be.
pub fn blame(
    git: &dyn Git,
    target: &Target,
    path: &str,
    each: &mut dyn FnMut(Lines, &[BlameCommit]) -> bool,
) -> Result<(), GitError> {
    read(git, target, path, None, each)
}

/// Who last wrote lines `first..=last` (from 1) of `path` in `target`,
/// by line: each group of lines with its commit, `None` for lines not
/// committed yet.
pub fn blame_lines(
    git: &dyn Git,
    target: &Target,
    path: &str,
    first: usize,
    last: usize,
) -> Result<Vec<(Lines, Option<BlameCommit>)>, GitError> {
    let mut found: Vec<(Lines, Option<BlameCommit>)> = Vec::new();
    read(
        git,
        target,
        path,
        Some((first, last)),
        &mut |lines, commits| {
            let commit = lines.commit.and_then(|index| commits.get(index)).cloned();
            found.push((lines, commit));
            true
        },
    )?;
    found.sort_by_key(|(lines, _)| lines.first);
    Ok(found)
}

fn read(
    git: &dyn Git,
    target: &Target,
    path: &str,
    range: Option<(usize, usize)>,
    each: &mut dyn FnMut(Lines, &[BlameCommit]) -> bool,
) -> Result<(), GitError> {
    let mut parser = Parser::new();
    let mut feed = |line: &str| match parser.feed(line) {
        Some(lines) => each(lines, parser.commits()),
        None => true,
    };
    let range = range.map(|(first, last)| format!("{first},{last}"));
    let mut arguments = vec!["blame", "--incremental"];
    if let Some(range) = &range {
        arguments.extend(["-L", range]);
    }
    match target {
        Target::WorkingCopy => {
            arguments.extend(["--", path]);
            git.lines(&arguments, &[], &mut feed)
        }
        Target::Commit(commit) => {
            arguments.extend([commit.as_str(), "--", path]);
            git.lines(&arguments, &[], &mut feed)
        }
        Target::Index => {
            // Git blames a commit or the working copy: the staged text is
            // given as if it were the file on disk.
            let staged = git.output(&["show", &format!(":{path}")])?;
            if staged.status != Some(0) {
                return Err(GitError::Failed {
                    command: format!("show :{path}"),
                    stderr: staged.stderr,
                });
            }
            arguments.extend(["--contents", "-", "--", path]);
            git.lines(&arguments, &staged.stdout, &mut feed)
        }
    }
}

/// The age as the gutter shows it: "now", "5 h", "8 d", "3 wk", "2 mo",
/// "4 y".
pub fn short_age(time: i64, now: i64) -> String {
    let seconds = (now - time).max(0);
    let days = seconds / 86_400;
    match days {
        0 if seconds < 3_600 => String::from("now"),
        0 => format!("{} h", seconds / 3_600),
        1..=13 => format!("{days} d"),
        14..=59 => format!("{} wk", days / 7),
        60..=729 => format!("{} mo", days / 30),
        _ => format!("{} y", days / 365),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: &str = "1111111111111111111111111111111111111111";
    const ZERO: &str = "0000000000000000000000000000000000000000";

    fn read(text: &str) -> (Vec<Lines>, Vec<BlameCommit>) {
        let mut parser = Parser::new();
        let lines = text.lines().filter_map(|line| parser.feed(line)).collect();
        (lines, parser.commits().to_vec())
    }

    #[test]
    fn entries_name_their_commit_once() {
        let text = format!(
            "{A} 1 1 2\nauthor Priya Shah\nauthor-mail <p@example.com>\nauthor-time 1700000000\n\
             author-tz +0100\ncommitter Someone\ncommitter-time 1700000500\nsummary author of a thing\n\
             filename old/name.rs\n{A} 7 9 1\nfilename old/name.rs\n\
             {ZERO} 3 3 4\nauthor Not Committed Yet\nauthor-time 1700009999\nsummary Version of x\n\
             filename name.rs\n"
        );
        let (lines, commits) = read(&text);
        assert_eq!(
            lines,
            vec![
                Lines {
                    first: 1,
                    count: 2,
                    commit: Some(0)
                },
                Lines {
                    first: 9,
                    count: 1,
                    commit: Some(0)
                },
                Lines {
                    first: 3,
                    count: 4,
                    commit: None
                },
            ]
        );
        assert_eq!(commits.len(), 1);
        assert_eq!(commits[0].author, "Priya Shah");
        assert_eq!(commits[0].time, 1_700_000_000);
        assert_eq!(commits[0].subject, "author of a thing");
        assert_eq!(commits[0].path, "old/name.rs");
        assert_eq!(commits[0].short_id(), "1111111");
    }

    #[test]
    fn ages_are_short() {
        let day = 86_400;
        assert_eq!(short_age(100, 100 + 60), "now");
        assert_eq!(short_age(0, 5 * 3_600), "5 h");
        assert_eq!(short_age(0, 8 * day), "8 d");
        assert_eq!(short_age(0, 21 * day), "3 wk");
        assert_eq!(short_age(0, 65 * day), "2 mo");
        assert_eq!(short_age(0, 800 * day), "2 y");
        assert_eq!(short_age(500, 100), "now");
    }
}
