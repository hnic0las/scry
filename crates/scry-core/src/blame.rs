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
/// it. Lines are taken from each side's own version of the file, at the
/// revision that side holds.
pub fn conflict_commits(
    git: &dyn Git,
    operation: &Operation,
    file: &str,
    session: &MergeSession,
) -> Vec<ConflictCommits> {
    (0..session.conflict_count())
        .map(|index| ConflictCommits {
            ours: side_commit(
                git,
                &operation.ours,
                file,
                session.side_lines(index, Pane::Ours),
            ),
            theirs: operation.theirs.as_ref().and_then(|side| {
                side_commit(git, side, file, session.side_lines(index, Pane::Theirs))
            }),
        })
        .collect()
}

/// The last commit to touch `lines` (first and last, from 1) at the side's
/// revision; for a side with no line there (it deleted them), the last
/// commit to touch the file on that side.
fn side_commit(
    git: &dyn Git,
    side: &Side,
    file: &str,
    lines: Option<(usize, usize)>,
) -> Option<LineCommit> {
    // A revert's "theirs" holds the content before the reverted commit.
    let revision = match side.role {
        SideRole::RevertedCommit => format!("{}^", side.commit.id),
        _ => side.commit.id.clone(),
    };
    let id = match lines {
        Some((first, last)) => last_commit_of_lines(git, &revision, file, first, last),
        None => last_commit_of_file(git, &revision, file),
    };
    read_commit(git, &id.ok()??).ok()
}

/// The newest commit, by committer time, among those `git blame` gives for
/// the lines.
fn last_commit_of_lines(
    git: &dyn Git,
    revision: &str,
    file: &str,
    first: usize,
    last: usize,
) -> Result<Option<String>, GitError> {
    let range = format!("{first},{last}");
    let output = git.run(&["blame", "--porcelain", "-L", &range, revision, "--", file])?;
    Ok(newest_in_porcelain(&output))
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

/// Picks the newest commit from `git blame --porcelain` output: each commit's
/// first line group carries its `committer-time`.
fn newest_in_porcelain(output: &str) -> Option<String> {
    let mut newest: Option<(i64, String)> = None;
    let mut current: Option<String> = None;
    for line in output.lines() {
        if let Some(time) = line.strip_prefix("committer-time ") {
            let (Some(id), Ok(time)) = (&current, time.parse::<i64>()) else {
                continue;
            };
            if newest
                .as_ref()
                .is_none_or(|(newest_time, _)| time > *newest_time)
            {
                newest = Some((time, id.clone()));
            }
        } else if !line.starts_with('\t') {
            // A group header starts with the full commit ID.
            let first = line.split(' ').next().unwrap_or("");
            if first.len() >= 40 && first.chars().all(|c| c.is_ascii_hexdigit()) {
                current = Some(first.to_string());
            }
        }
    }
    newest.map(|(_, id)| id)
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

    /// The newest commit wins, read from the group headers and their
    /// committer times; repeated groups of a commit carry no time and change
    /// nothing.
    #[test]
    fn newest_commit_is_picked_from_porcelain() {
        let old = "a".repeat(40);
        let new = "b".repeat(40);
        let output = format!(
            "{old} 1 1 1\nauthor A\ncommitter-time 100\nsummary old\nfilename f\n\tline 1\n\
             {new} 2 2 1\nauthor B\ncommitter-time 200\nsummary new\nfilename f\n\tcommitter-time 999\n\
             {old} 3 3 1\nfilename f\n\tline 3\n"
        );
        assert_eq!(newest_in_porcelain(&output), Some(new));
        assert_eq!(newest_in_porcelain(""), None);
    }
}
