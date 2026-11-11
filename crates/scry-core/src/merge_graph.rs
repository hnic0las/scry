//! The history of a merge in progress: the two branches since their merge
//! base, which the merge commit will join. Only facts git gives; the UI
//! draws them as two lanes.

use crate::git::{Git, GitError};

/// How many commits each side lists; the rest are counted.
pub const LIMIT: usize = 12;

/// A commit of the graph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphCommit {
    pub short_id: String,
    pub subject: String,
}

/// One branch of the merge, newest commit first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lane {
    /// The branch, or the commit's id when no branch points at it.
    pub name: String,
    pub commits: Vec<GraphCommit>,
    /// Commits since the base past the first `LIMIT`.
    pub more: usize,
}

/// Both branches since their merge base.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergeGraph {
    /// Git's "ours": the branch checked out.
    pub ours: Lane,
    /// Git's "theirs": the branch merged in.
    pub theirs: Lane,
    pub base: GraphCommit,
}

/// The graph of the merge between `HEAD` and `theirs` (a commit id),
/// named `ours_name` and `theirs_name`; `None` without a merge base
/// (unrelated histories).
pub fn read(
    git: &dyn Git,
    ours_name: &str,
    theirs: &str,
    theirs_name: &str,
) -> Result<Option<MergeGraph>, GitError> {
    let output = git.output(&["merge-base", "HEAD", theirs])?;
    if output.status != Some(0) {
        return Ok(None);
    }
    let base = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let Some(base_commit) = log(git, &base, 1, true)?.into_iter().next() else {
        return Ok(None);
    };
    Ok(Some(MergeGraph {
        ours: lane(git, ours_name, &format!("{base}..HEAD"))?,
        theirs: lane(git, theirs_name, &format!("{base}..{theirs}"))?,
        base: base_commit,
    }))
}

fn lane(git: &dyn Git, name: &str, range: &str) -> Result<Lane, GitError> {
    let commits = log(git, range, LIMIT, false)?;
    let count = git.run(&["rev-list", "--count", range])?;
    let total: usize = count.trim().parse().unwrap_or(commits.len());
    Ok(Lane {
        name: name.to_string(),
        more: total.saturating_sub(commits.len()),
        commits,
    })
}

/// Up to `limit` commits of `revision`, newest first; `single` reads the
/// one commit itself rather than a range.
fn log(
    git: &dyn Git,
    revision: &str,
    limit: usize,
    single: bool,
) -> Result<Vec<GraphCommit>, GitError> {
    let count = format!("-{limit}");
    let mut args = vec!["log", "--no-show-signature", "--format=%h%x00%s", &count];
    if single {
        args.push("--no-walk");
    }
    args.push(revision);
    args.push("--");
    let output = git.run(&args)?;
    Ok(parse(&output))
}

/// `%h NUL %s` lines into commits.
fn parse(output: &str) -> Vec<GraphCommit> {
    output
        .lines()
        .filter_map(|line| line.split_once('\0'))
        .map(|(short_id, subject)| GraphCommit {
            short_id: short_id.to_string(),
            subject: subject.to_string(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each line gives a commit's id and subject; a subject may hold
    /// anything but a newline.
    #[test]
    fn parses_ids_and_subjects() {
        assert_eq!(
            parse("a1b2c3d\0Fix: a, b\0c\n9f3e2c1\0Initial\n"),
            [
                GraphCommit {
                    short_id: String::from("a1b2c3d"),
                    subject: String::from("Fix: a, b\0c"),
                },
                GraphCommit {
                    short_id: String::from("9f3e2c1"),
                    subject: String::from("Initial"),
                },
            ]
        );
    }
}
