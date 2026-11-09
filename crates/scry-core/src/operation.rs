//! Which git operation is in progress, and who is on each side of its conflicts.
//!
//! Git names the sides "ours" and "theirs". During a rebase, "ours" is the
//! branch being rebased onto and "theirs" is the user's own commit: the
//! opposite of what people expect. Scry shows a human label for each side.

use std::fs;
use std::path::Path;

use crate::git::{Git, GitError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperationKind {
    Merge,
    Rebase,
    CherryPick,
    Revert,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Commit {
    pub id: String,
    pub short_id: String,
    pub subject: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SideRole {
    /// Merge, cherry-pick or revert: the branch checked out.
    CurrentBranch,
    /// Rebase: the branch being rebased onto, plus the user's commits
    /// already replayed on top of it.
    RebaseTarget { replayed: usize },
    /// Rebase: the user's commit being replayed.
    CommitBeingRebased,
    /// Merge: the branch or commit being merged in.
    MergedBranch,
    /// Cherry-pick: the commit being picked.
    CherryPickedCommit,
    /// Revert: the commit being reverted.
    RevertedCommit,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Side {
    pub role: SideRole,
    /// A branch name when one points at this side, otherwise a short commit
    /// ID; "detached HEAD" for a current branch that is not a branch.
    pub name: String,
    /// The commit whose content this side holds.
    pub commit: Commit,
}

impl Side {
    /// The label shown in the side's header, for example
    /// "main · rebase target" or "Your commit a1b2c3d · Retry uploads".
    pub fn label(&self) -> String {
        let commit = &self.commit;
        match self.role {
            SideRole::CurrentBranch => format!("{} · current branch", self.name),
            SideRole::RebaseTarget { replayed: 0 } => format!("{} · rebase target", self.name),
            SideRole::RebaseTarget { replayed: 1 } => {
                format!("{} + 1 rebased commit · rebase target", self.name)
            }
            SideRole::RebaseTarget { replayed } => {
                format!("{} + {replayed} rebased commits · rebase target", self.name)
            }
            SideRole::CommitBeingRebased => {
                format!("Your commit {} · {}", commit.short_id, commit.subject)
            }
            SideRole::MergedBranch => format!("{} · being merged", self.name),
            SideRole::CherryPickedCommit => {
                format!(
                    "Cherry-picked commit {} · {}",
                    commit.short_id, commit.subject
                )
            }
            SideRole::RevertedCommit => {
                format!("Revert of {} · {}", commit.short_id, commit.subject)
            }
        }
    }

    /// The commit a branch-like side is at, when the label does not already
    /// name it, for example "at 9f8e7d6 Add a 30 s timeout".
    pub fn detail(&self) -> Option<String> {
        match self.role {
            SideRole::CurrentBranch | SideRole::RebaseTarget { .. } | SideRole::MergedBranch => {
                Some(format!(
                    "at {} {}",
                    self.commit.short_id, self.commit.subject
                ))
            }
            SideRole::CommitBeingRebased
            | SideRole::CherryPickedCommit
            | SideRole::RevertedCommit => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Progress {
    /// The step being applied, from 1.
    pub current: usize,
    pub total: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Operation {
    pub kind: OperationKind,
    /// Git's "ours".
    pub ours: Side,
    /// Git's "theirs". `None` when a rebase stopped with no commit being
    /// applied (an `edit` or a `break`).
    pub theirs: Option<Side>,
    /// The branch being rebased, or the branch checked out; `None` on a
    /// detached HEAD.
    pub branch: Option<String>,
    /// Rebase only.
    pub progress: Option<Progress>,
}

impl Operation {
    /// The operation in plain words, for example
    /// "Rebasing feature onto main · commit 3 of 7: a1b2c3d Retry uploads".
    pub fn summary(&self) -> String {
        let branch = self.branch.as_deref().unwrap_or("detached HEAD");
        let theirs_commit = match &self.theirs {
            Some(side) => format!("{} {}", side.commit.short_id, side.commit.subject),
            None => String::from("?"),
        };
        match self.kind {
            OperationKind::Rebase => {
                let mut text = format!("Rebasing {branch} onto {}", self.ours.name);
                if let Some(progress) = self.progress {
                    text += &format!(" · commit {} of {}", progress.current, progress.total);
                }
                if self.theirs.is_some() {
                    text += &format!(": {theirs_commit}");
                }
                text
            }
            OperationKind::Merge => {
                let theirs = match &self.theirs {
                    Some(side) => side.name.as_str(),
                    None => "?",
                };
                format!("Merging {theirs} into {branch}")
            }
            OperationKind::CherryPick => format!("Cherry-picking {theirs_commit} onto {branch}"),
            OperationKind::Revert => format!("Reverting {theirs_commit} on {branch}"),
        }
    }
}

/// The operation in progress in the repository, if any.
pub fn current(git: &dyn Git) -> Result<Option<Operation>, GitError> {
    let git_dir = git.run(&["rev-parse", "--absolute-git-dir"])?;
    let git_dir = Path::new(git_dir.trim());

    // Checked first: a rebase can stop on a merge or a pick that writes the
    // other files too.
    if let Some(operation) = rebase(git, git_dir)? {
        return Ok(Some(operation));
    }

    let others = [
        (OperationKind::Merge, "MERGE_HEAD", SideRole::MergedBranch),
        (
            OperationKind::CherryPick,
            "CHERRY_PICK_HEAD",
            SideRole::CherryPickedCommit,
        ),
        (
            OperationKind::Revert,
            "REVERT_HEAD",
            SideRole::RevertedCommit,
        ),
    ];
    for (kind, file, role) in others {
        let Some(theirs_id) = first_line(&git_dir.join(file)) else {
            continue;
        };
        let branch = current_branch(git_dir);
        let ours = Side {
            role: SideRole::CurrentBranch,
            name: branch
                .clone()
                .unwrap_or_else(|| String::from("detached HEAD")),
            commit: commit(git, "HEAD")?,
        };
        let theirs_commit = commit(git, &theirs_id)?;
        let theirs_name = match branch_at(git, &theirs_commit.id, branch.as_deref())? {
            Some(name) => name,
            None => theirs_commit.short_id.clone(),
        };
        let theirs = Side {
            role,
            name: theirs_name,
            commit: theirs_commit,
        };
        return Ok(Some(Operation {
            kind,
            ours,
            theirs: Some(theirs),
            branch,
            progress: None,
        }));
    }
    Ok(None)
}

/// The files git left with conflicts, relative to the repository root.
pub fn conflicted_paths(git: &dyn Git) -> Result<Vec<String>, GitError> {
    let output = git.run(&["diff", "--name-only", "--diff-filter=U", "-z"])?;
    Ok(output
        .split('\0')
        .filter(|path| !path.is_empty())
        .map(String::from)
        .collect())
}

fn rebase(git: &dyn Git, git_dir: &Path) -> Result<Option<Operation>, GitError> {
    let merge_dir = git_dir.join("rebase-merge");
    let apply_dir = git_dir.join("rebase-apply");
    // `git am` uses rebase-apply too; only a rebase writes `rebasing` there.
    let (dir, step_file, total_file) = if merge_dir.is_dir() {
        (merge_dir, "msgnum", "end")
    } else if apply_dir.join("rebasing").exists() {
        (apply_dir, "next", "last")
    } else {
        return Ok(None);
    };

    let branch = first_line(&dir.join("head-name"))
        .and_then(|name| name.strip_prefix("refs/heads/").map(String::from));
    let progress = match (number(&dir.join(step_file)), number(&dir.join(total_file))) {
        (Some(current), Some(total)) => Some(Progress { current, total }),
        _ => None,
    };

    let head = commit(git, "HEAD")?;
    let (onto_name, replayed) = match first_line(&dir.join("onto")) {
        Some(onto) => {
            let name = match branch_at(git, &onto, branch.as_deref())? {
                Some(name) => name,
                None => commit(git, &onto)?.short_id,
            };
            (name, count(git, &format!("{onto}..HEAD"))?)
        }
        None => (head.short_id.clone(), 0),
    };
    let ours = Side {
        role: SideRole::RebaseTarget { replayed },
        name: onto_name,
        commit: head,
    };

    let theirs = match first_line(&git_dir.join("REBASE_HEAD")) {
        Some(id) => {
            let commit = commit(git, &id)?;
            Some(Side {
                role: SideRole::CommitBeingRebased,
                name: commit.short_id.clone(),
                commit,
            })
        }
        None => None,
    };

    Ok(Some(Operation {
        kind: OperationKind::Rebase,
        ours,
        theirs,
        branch,
        progress,
    }))
}

/// The branch HEAD points to, read from the HEAD file of this working tree.
fn current_branch(git_dir: &Path) -> Option<String> {
    let head = first_line(&git_dir.join("HEAD"))?;
    head.strip_prefix("ref: refs/heads/").map(String::from)
}

fn commit(git: &dyn Git, revision: &str) -> Result<Commit, GitError> {
    let args = [
        "show",
        "--no-patch",
        "--no-show-signature",
        "--format=%H%x00%h%x00%s",
        revision,
        "--",
    ];
    let output = git.run(&args)?;
    let mut fields = output.trim_end_matches('\n').split('\0');
    match (fields.next(), fields.next(), fields.next()) {
        (Some(id), Some(short_id), Some(subject)) if !id.is_empty() => Ok(Commit {
            id: id.to_string(),
            short_id: short_id.to_string(),
            subject: subject.to_string(),
        }),
        _ => Err(GitError::UnexpectedOutput {
            command: args.join(" "),
            output,
        }),
    }
}

/// A local branch, or else a remote-tracking branch, pointing at `id`,
/// other than `exclude`.
fn branch_at(git: &dyn Git, id: &str, exclude: Option<&str>) -> Result<Option<String>, GitError> {
    // Sorted by ref name, so local branches come before remote ones.
    let output = git.run(&[
        "for-each-ref",
        "--points-at",
        id,
        "--format=%(refname)",
        "refs/heads",
        "refs/remotes",
    ])?;
    for refname in output.lines() {
        if refname.ends_with("/HEAD") {
            continue;
        }
        let name = refname
            .strip_prefix("refs/heads/")
            .or_else(|| refname.strip_prefix("refs/remotes/"));
        if let Some(name) = name
            && Some(name) != exclude
        {
            return Ok(Some(name.to_string()));
        }
    }
    Ok(None)
}

fn count(git: &dyn Git, range: &str) -> Result<usize, GitError> {
    let args = ["rev-list", "--count", range];
    let output = git.run(&args)?;
    output
        .trim()
        .parse()
        .map_err(|_| GitError::UnexpectedOutput {
            command: args.join(" "),
            output,
        })
}

fn first_line(path: &Path) -> Option<String> {
    let text = fs::read_to_string(path).ok()?;
    let line = text.lines().next()?.trim();
    if line.is_empty() {
        None
    } else {
        Some(line.to_string())
    }
}

fn number(path: &Path) -> Option<usize> {
    first_line(path)?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn side(role: SideRole, name: &str) -> Side {
        Side {
            role,
            name: name.to_string(),
            commit: Commit {
                id: String::from("a1b2c3d4e5f6"),
                short_id: String::from("a1b2c3d"),
                subject: String::from("Retry failed uploads"),
            },
        }
    }

    /// During a rebase git's ours and theirs are swapped from the user's view; the labels must
    /// say which side is the rebase target and which is the user's commit.
    #[test]
    fn rebase_labels_say_who_is_who() {
        assert_eq!(
            side(SideRole::RebaseTarget { replayed: 0 }, "main").label(),
            "main · rebase target"
        );
        assert_eq!(
            side(SideRole::RebaseTarget { replayed: 1 }, "main").label(),
            "main + 1 rebased commit · rebase target"
        );
        assert_eq!(
            side(SideRole::RebaseTarget { replayed: 2 }, "main").label(),
            "main + 2 rebased commits · rebase target"
        );
        assert_eq!(
            side(SideRole::CommitBeingRebased, "a1b2c3d").label(),
            "Your commit a1b2c3d · Retry failed uploads"
        );
    }

    /// Merge, cherry-pick and revert sides get labels that say what each side is.
    #[test]
    fn other_labels() {
        assert_eq!(
            side(SideRole::CurrentBranch, "feature").label(),
            "feature · current branch"
        );
        assert_eq!(
            side(SideRole::MergedBranch, "main").label(),
            "main · being merged"
        );
        assert_eq!(
            side(SideRole::CherryPickedCommit, "a1b2c3d").label(),
            "Cherry-picked commit a1b2c3d · Retry failed uploads"
        );
        assert_eq!(
            side(SideRole::RevertedCommit, "a1b2c3d").label(),
            "Revert of a1b2c3d · Retry failed uploads"
        );
    }

    /// A branch side shows the commit it points at; a commit side has no detail, since its
    /// label already names the commit.
    #[test]
    fn detail_only_for_branch_like_sides() {
        assert_eq!(
            side(SideRole::MergedBranch, "main").detail().as_deref(),
            Some("at a1b2c3d Retry failed uploads")
        );
        assert_eq!(side(SideRole::CommitBeingRebased, "a1b2c3d").detail(), None);
    }

    /// The rebase summary names the branch, the target, the progress and the commit being
    /// replayed, as the window subtitle shows it.
    #[test]
    fn rebase_summary() {
        let operation = Operation {
            kind: OperationKind::Rebase,
            ours: side(SideRole::RebaseTarget { replayed: 2 }, "main"),
            theirs: Some(side(SideRole::CommitBeingRebased, "a1b2c3d")),
            branch: Some(String::from("feature/offline-sync")),
            progress: Some(Progress {
                current: 3,
                total: 7,
            }),
        };
        assert_eq!(
            operation.summary(),
            "Rebasing feature/offline-sync onto main · commit 3 of 7: a1b2c3d Retry failed uploads"
        );
    }

    /// A merge on a detached HEAD still gets a readable summary.
    #[test]
    fn merge_summary_on_detached_head() {
        let operation = Operation {
            kind: OperationKind::Merge,
            ours: side(SideRole::CurrentBranch, "detached HEAD"),
            theirs: Some(side(SideRole::MergedBranch, "main")),
            branch: None,
            progress: None,
        };
        assert_eq!(operation.summary(), "Merging main into detached HEAD");
    }
}
