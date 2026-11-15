//! Which commit last touched each side of a conflict, for the lens rows:
//! "why each side changed", without reading the history by hand.

use crate::git::{Git, GitError};
use crate::operation::{Operation, Side, SideRole};
use crate::session::{MergeSession, Pane};

/// A commit as the lens row shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineCommit {
    pub short_id: String,
    pub subject: String,
    pub author: String,
    /// Committer time, in seconds since the Unix epoch.
    pub time: i64,
}

/// The last commit on each side of one conflict; `None` when git has no
/// answer (a new file, a renamed path).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ConflictCommits {
    pub ours: Option<LineCommit>,
    pub theirs: Option<LineCommit>,
}

/// For every conflict of `session`, the commit that last touched its lines
/// on each side. `file` is the file's path as git in `git`'s directory sees
/// it.
///
/// Each side's text (the common text with that side of every conflict) is
/// blamed once against the revision that side holds, and each conflict
/// takes the newest commit among its lines; each commit is read once. The
/// common text also has the other side's clean changes, so its line numbers
/// are not the revision's: git matches the lines itself, and the other
/// side's lines count as not committed.
pub fn conflict_commits(
    git: &dyn Git,
    operation: &Operation,
    file: &str,
    session: &MergeSession,
) -> Vec<ConflictCommits> {
    let mut commits = Commits::default();
    let ours = side_commits(
        git,
        &operation.ours,
        file,
        session,
        Pane::Ours,
        &mut commits,
    );
    let theirs = match &operation.theirs {
        Some(side) => side_commits(git, side, file, session, Pane::Theirs, &mut commits),
        None => vec![None; session.conflict_count()],
    };
    ours.into_iter()
        .zip(theirs)
        .map(|(ours, theirs)| ConflictCommits { ours, theirs })
        .collect()
}

/// Commits read so far, by ID; `None` when git could not read one.
#[derive(Default)]
struct Commits(std::collections::HashMap<String, Option<LineCommit>>);

impl Commits {
    fn read(&mut self, git: &dyn Git, id: &str) -> Option<LineCommit> {
        self.0
            .entry(id.to_string())
            .or_insert_with(|| read_commit(git, id).ok())
            .clone()
    }
}

/// For each conflict, the last commit to touch its lines on one side,
/// blamed from the side's revision; for a conflict with no line on that
/// side (it deleted them), the last commit to touch the file there.
fn side_commits(
    git: &dyn Git,
    side: &Side,
    file: &str,
    session: &MergeSession,
    pane: Pane,
    commits: &mut Commits,
) -> Vec<Option<LineCommit>> {
    // A revert's "theirs" holds the content before the reverted commit.
    let revision = match side.role {
        SideRole::RevertedCommit => format!("{}^", side.commit.id),
        _ => side.commit.id.clone(),
    };
    let text = session.side_text(pane);
    // git before 2.41 (the one Xcode 15 and 16 ship) refuses `--contents`
    // with a revision: the revision's own file is blamed instead, and each
    // conflict's lines found in it by their text.
    let (blamed, in_revision) = match blame(git, &revision, file, &text) {
        Ok(blamed) => (blamed, false),
        Err(_) => (
            blame_revision(git, &revision, file).unwrap_or_default(),
            true,
        ),
    };
    let mut file_commit: Option<Option<String>> = None;
    (0..session.conflict_count())
        .map(|index| {
            let lines = session.side_lines(index, pane).and_then(|(first, last)| {
                if !in_revision {
                    return Some((first, last));
                }
                let conflict = session.conflict(index);
                let side = match pane {
                    Pane::Ours => &conflict.ours.text,
                    Pane::Theirs => &conflict.theirs.text,
                };
                let wanted: Vec<&str> = side.lines().collect();
                find_block(&blamed.texts, &wanted, first)
                    .map(|start| (start, start + wanted.len() - 1))
            });
            let id = match lines {
                Some((first, last)) => newest(&blamed, first, last),
                None if session.side_lines(index, pane).is_some() => None,
                None => file_commit
                    .get_or_insert_with(|| last_commit_of_file(git, &revision, file).ok().flatten())
                    .clone(),
            };
            commits.read(git, &id?)
        })
        .collect()
}

/// A blamed file, by line from 1 (index 0 unused): each line's commit with
/// its committer time (`None` for a line not in the revision), and its
/// text.
#[derive(Default)]
struct Blame {
    commits: Vec<Option<(String, i64)>>,
    texts: Vec<String>,
}

/// `text` blamed from `revision` in one `git blame` run.
fn blame(git: &dyn Git, revision: &str, file: &str, text: &str) -> Result<Blame, GitError> {
    let args = [
        "blame",
        "--porcelain",
        "--contents",
        "-",
        revision,
        "--",
        file,
    ];
    let output = git.run_with_input(&args, text.as_bytes())?;
    Ok(lines_in_porcelain(&output))
}

/// The file as `revision` holds it, blamed.
fn blame_revision(git: &dyn Git, revision: &str, file: &str) -> Result<Blame, GitError> {
    let output = git.run(&["blame", "--porcelain", revision, "--", file])?;
    Ok(lines_in_porcelain(&output))
}

/// Where `wanted` starts among `texts` (from 1), the place nearest `near`
/// when it occurs more than once.
fn find_block(texts: &[String], wanted: &[&str], near: usize) -> Option<usize> {
    if wanted.is_empty() || texts.len() <= wanted.len() {
        return None;
    }
    (1..=texts.len() - wanted.len())
        .filter(|&start| {
            texts[start..start + wanted.len()]
                .iter()
                .zip(wanted)
                .all(|(text, wanted)| text == wanted)
        })
        .min_by_key(|start| start.abs_diff(near))
}

/// The newest commit, by committer time, among lines `first` to `last`;
/// the first one found on a tie.
fn newest(blamed: &Blame, first: usize, last: usize) -> Option<String> {
    let mut newest: Option<&(String, i64)> = None;
    for line in blamed.commits.iter().take(last + 1).skip(first).flatten() {
        if newest.is_none_or(|found| line.1 > found.1) {
            newest = Some(line);
        }
    }
    newest.map(|(id, _)| id.clone())
}

/// Each line's commit and committer time from `git blame --porcelain`: a
/// header `<id> <original line> <final line> [<count>]` per line, the
/// commit's `committer-time` after its first header only.
fn lines_in_porcelain(output: &str) -> Blame {
    let mut times: std::collections::HashMap<String, i64> = std::collections::HashMap::new();
    let mut lines: Vec<(usize, String)> = Vec::new();
    let mut texts: Vec<(usize, String)> = Vec::new();
    let mut current: Option<String> = None;
    for line in output.lines() {
        // Each header is followed by its line's text, after a tab.
        if let Some(text) = line.strip_prefix('\t') {
            if let Some((number, _)) = lines.last() {
                texts.push((*number, text.to_string()));
            }
            continue;
        }
        if let Some(time) = line.strip_prefix("committer-time ") {
            if let (Some(id), Ok(time)) = (&current, time.parse::<i64>()) {
                times.entry(id.clone()).or_insert(time);
            }
            continue;
        }
        let mut fields = line.split(' ');
        let (Some(id), Some(_), Some(final_line)) = (fields.next(), fields.next(), fields.next())
        else {
            continue;
        };
        if id.len() < 40 || !id.chars().all(|c| c.is_ascii_hexdigit()) {
            continue;
        }
        let Ok(final_line) = final_line.parse::<usize>() else {
            continue;
        };
        current = Some(id.to_string());
        lines.push((final_line, id.to_string()));
    }
    let count = lines.iter().map(|(line, _)| *line).max().unwrap_or(0);
    let mut blamed = Blame {
        commits: vec![None; count + 1],
        texts: vec![String::new(); count + 1],
    };
    for (line, id) in lines {
        // Lines not in the revision ("Not Committed Yet") have an all-zero
        // ID: no commit to show.
        if id.chars().any(|c| c != '0') {
            blamed.commits[line] = times.get(&id).map(|time| (id, *time));
        }
    }
    for (line, text) in texts {
        blamed.texts[line] = text;
    }
    blamed
}

fn last_commit_of_file(
    git: &dyn Git,
    revision: &str,
    file: &str,
) -> Result<Option<String>, GitError> {
    let output = git.run(&["log", "-1", "--format=%H", revision, "--", file])?;
    let id = output.trim();
    Ok((!id.is_empty()).then(|| id.to_string()))
}

fn read_commit(git: &dyn Git, id: &str) -> Result<LineCommit, GitError> {
    let args = [
        "show",
        "--no-patch",
        "--no-show-signature",
        "--format=%h%x00%s%x00%an%x00%ct",
        id,
        "--",
    ];
    let output = git.run(&args)?;
    let fields: Vec<&str> = output.trim_end_matches('\n').split('\0').collect();
    match fields.as_slice() {
        [short_id, subject, author, time] => Ok(LineCommit {
            short_id: short_id.to_string(),
            subject: subject.to_string(),
            author: author.to_string(),
            time: time.parse().unwrap_or(0),
        }),
        _ => Err(GitError::UnexpectedOutput {
            command: args.join(" "),
            output,
        }),
    }
}

/// How long ago `time` was, from the clock: the same words as `age`.
pub fn age_from_now(time: i64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs() as i64)
        .unwrap_or(time);
    age(time, now)
}

/// How long ago `time` was, as the lens row says it: "today", "yesterday",
/// "3 days ago", "2 weeks ago", "5 months ago", "2 years ago".
pub fn age(time: i64, now: i64) -> String {
    let days = (now - time).max(0) / 86_400;
    let plural = |count: i64, unit: &str| {
        if count == 1 {
            format!("1 {unit} ago")
        } else {
            format!("{count} {unit}s ago")
        }
    };
    match days {
        0 => String::from("today"),
        1 => String::from("yesterday"),
        2..=13 => plural(days, "day"),
        14..=59 => plural(days / 7, "week"),
        60..=729 => plural(days / 30, "month"),
        _ => plural(days / 365, "year"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The lens reads ages from the clock, in the same words as `age`.
    #[test]
    fn ages_are_read_from_the_clock() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        assert_eq!(age_from_now(now - 3 * 86_400), "3 days ago");
    }

    /// With git before 2.41, a conflict's side is found in the blamed
    /// revision by its text: nearest the line it had, nowhere for an empty
    /// side or a revision shorter than the side.
    #[test]
    fn a_side_is_found_by_its_text_nearest_its_line() {
        let texts: Vec<String> = ["", "a", "b", "x", "a", "b"]
            .iter()
            .map(|text| text.to_string())
            .collect();
        assert_eq!(find_block(&texts, &["a", "b"], 1), Some(1));
        assert_eq!(find_block(&texts, &["a", "b"], 5), Some(4));
        assert_eq!(find_block(&texts, &[], 2), None);
        assert_eq!(find_block(&texts[..2], &["a", "b", "c"], 1), None);
    }

    /// Ages read like the prototype's lens rows, with the unit growing as the
    /// commit gets older so the text stays short.
    #[test]
    fn ages_are_short_and_human() {
        let day = 86_400;
        let now = 1_000 * day;
        assert_eq!(age(now - 3600, now), "today");
        assert_eq!(age(now - day, now), "yesterday");
        assert_eq!(age(now - 2 * day, now), "2 days ago");
        assert_eq!(age(now - 20 * day, now), "2 weeks ago");
        assert_eq!(age(now - 150 * day, now), "5 months ago");
        assert_eq!(age(now - 800 * day, now), "2 years ago");
        // A clock behind the commit never gives a negative age.
        assert_eq!(age(now + day, now), "today");
    }

    /// Each line gets its group's commit and time, read once per commit
    /// from its first header; the newest commit of a range wins, and lines
    /// not committed count for nothing.
    #[test]
    fn lines_and_their_newest_commit_are_read_from_porcelain() {
        let old = "a".repeat(40);
        let new = "b".repeat(40);
        let none = "0".repeat(40);
        let output = format!(
            "{old} 1 1 1\nauthor A\ncommitter-time 100\nsummary old\nfilename f\n\tline 1\n\
             {new} 2 2 1\nauthor B\ncommitter-time 200\nsummary new\nfilename f\n\tcommitter-time 999\n\
             {old} 3 3 1\nfilename f\n\tline 3\n\
             {none} 4 4 1\nauthor Not Committed Yet\ncommitter-time 300\nfilename f\n\tline 4\n"
        );
        let blamed = lines_in_porcelain(&output);
        assert_eq!(newest(&blamed, 1, 3), Some(new));
        assert_eq!(newest(&blamed, 3, 4), Some(old));
        assert_eq!(newest(&blamed, 4, 4), None);
        assert_eq!(newest(&lines_in_porcelain(""), 1, 1), None);
    }
}
