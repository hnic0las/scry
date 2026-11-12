//! The plan of a rebase: the commit it replays onto, and each line of its
//! todo list (pick, reword, fixup, drop…) with where git stands. Git keeps
//! the lines already run in `rebase-merge/done` and those to come in
//! `rebase-merge/git-rebase-todo`; the last line done is the one git
//! stopped on.

use std::fs;
use std::path::Path;

use crate::git::{Git, GitError};

/// What a line of the todo list does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Pick,
    Reword,
    Edit,
    Squash,
    Fixup,
    Drop,
    Exec,
    Break,
    /// `label`, `reset`, `merge`, `update-ref`: the lines of a rebase that
    /// keeps merges, or updates other branches.
    Other,
}

impl Action {
    /// Git's own word, as the todo list spells it in full.
    pub fn name(self) -> &'static str {
        match self {
            Action::Pick => "pick",
            Action::Reword => "reword",
            Action::Edit => "edit",
            Action::Squash => "squash",
            Action::Fixup => "fixup",
            Action::Drop => "drop",
            Action::Exec => "exec",
            Action::Break => "break",
            Action::Other => "other",
        }
    }

    fn parse(word: &str) -> Action {
        match word {
            "p" | "pick" => Action::Pick,
            "r" | "reword" => Action::Reword,
            "e" | "edit" => Action::Edit,
            "s" | "squash" => Action::Squash,
            "f" | "fixup" => Action::Fixup,
            "d" | "drop" => Action::Drop,
            "x" | "exec" => Action::Exec,
            "b" | "break" => Action::Break,
            _ => Action::Other,
        }
    }

    /// Whether the line names a commit to replay.
    fn has_commit(self) -> bool {
        matches!(
            self,
            Action::Pick
                | Action::Reword
                | Action::Edit
                | Action::Squash
                | Action::Fixup
                | Action::Drop
        )
    }
}

/// Where git stands on a line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepState {
    Done,
    /// The line git stopped on: conflicts, an `edit` or a `break`.
    Current,
    Pending,
}

/// One line of the todo list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    /// From 1, as git counts them.
    pub number: usize,
    pub action: Action,
    /// The commit of the todo list, abbreviated: the one the user wrote,
    /// which stays once the step is done. `None` for `exec` and `break`.
    pub short_id: Option<String>,
    /// The commit's subject, or the command of an `exec`; once done, the
    /// subject of the commit git made, a reword's new one.
    pub subject: String,
    /// Once done, the commit git made of it, abbreviated.
    pub replayed_id: Option<String>,
    /// The subject before the step, when the commit made has another:
    /// a reword.
    pub was: Option<String>,
    pub state: StepState,
    /// A fixup or squash joins the commit of this step.
    pub joins: Option<usize>,
}

/// The rebase's commits, from the first replayed to the last.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    /// The commit the branch is replayed onto.
    pub onto_short_id: String,
    pub onto_subject: String,
    pub steps: Vec<Step>,
}

impl Plan {
    /// The line git stopped on, from 1.
    pub fn current(&self) -> Option<usize> {
        self.steps
            .iter()
            .find(|step| step.state == StepState::Current)
            .map(|step| step.number)
    }
}

/// The plan of the rebase in progress; `None` without one, or for the
/// old `apply` backend, which keeps no todo list.
pub fn read(git: &dyn Git) -> Result<Option<Plan>, GitError> {
    let git_dir = git.run(&["rev-parse", "--absolute-git-dir"])?;
    let dir = Path::new(git_dir.trim()).join("rebase-merge");
    let Ok(done) = fs::read_to_string(dir.join("done")) else {
        return Ok(None);
    };
    let todo = fs::read_to_string(dir.join("git-rebase-todo")).unwrap_or_default();
    let Some(onto) = fs::read_to_string(dir.join("onto"))
        .ok()
        .map(|text| text.trim().to_string())
    else {
        return Ok(None);
    };
    let output = git.run(&[
        "show",
        "--no-patch",
        "--no-show-signature",
        "--format=%h%x00%s",
        &onto,
        "--",
    ])?;
    let (short_id, subject) = output
        .trim_end_matches('\n')
        .split_once('\0')
        .unwrap_or((onto.as_str(), ""));
    let mut steps = steps(&done, &todo);
    let rewritten = fs::read_to_string(dir.join("rewritten-list")).unwrap_or_default();
    show_rewritten(git, &mut steps, &rewritten)?;
    Ok(Some(Plan {
        onto_short_id: short_id.to_string(),
        onto_subject: subject.to_string(),
        steps,
    }))
}

/// Tells, for the steps done, the commit git made of each: its id, and a
/// reword's new subject with the old one kept. The todo list keeps the
/// original lines; `rewritten-list` pairs each original commit with the
/// one replayed, one pair per line. A fixup or squash keeps its own line:
/// its commit no longer exists on its own.
fn show_rewritten(git: &dyn Git, steps: &mut [Step], rewritten: &str) -> Result<(), GitError> {
    let pairs: Vec<(&str, &str)> = rewritten
        .lines()
        .filter_map(|line| line.split_once(' '))
        .collect();
    // The steps to show again, with the commit each became.
    let mut found: Vec<(usize, String)> = Vec::new();
    for (index, step) in steps.iter().enumerate() {
        let replayed = matches!(step.action, Action::Pick | Action::Reword | Action::Edit);
        let Some(id) = &step.short_id else { continue };
        if step.state != StepState::Done || !replayed {
            continue;
        }
        // The last pair wins: a fixup folded in later rewrites it again.
        if let Some((_, new)) = pairs
            .iter()
            .rev()
            .find(|(old, _)| old.starts_with(id.as_str()))
        {
            found.push((index, new.trim().to_string()));
        }
    }
    if found.is_empty() {
        return Ok(());
    }
    let mut args = vec![
        "show",
        "--no-patch",
        "--no-show-signature",
        "--format=%h%x00%s",
    ];
    args.extend(found.iter().map(|(_, new)| new.as_str()));
    args.push("--");
    let output = git.run(&args)?;
    for ((index, _), line) in found.iter().zip(output.lines()) {
        if let Some((short_id, subject)) = line.split_once('\0') {
            let step = &mut steps[*index];
            step.replayed_id = Some(short_id.to_string());
            if step.subject != subject {
                step.was = Some(std::mem::replace(&mut step.subject, subject.to_string()));
            }
        }
    }
    Ok(())
}

/// The steps from the text of `done` and `git-rebase-todo`.
pub fn steps(done: &str, todo: &str) -> Vec<Step> {
    let done = lines(done);
    let todo = lines(todo);
    let current = done.len();
    let mut steps: Vec<Step> = Vec::new();
    for (index, line) in done.iter().chain(todo.iter()).enumerate() {
        let number = index + 1;
        let state = if number < current {
            StepState::Done
        } else if number == current {
            StepState::Current
        } else {
            StepState::Pending
        };
        let mut step = step(number, line, state);
        if matches!(step.action, Action::Fixup | Action::Squash) {
            // The commit it joins: the last one before it that is not a
            // fixup or squash itself, nor dropped.
            step.joins = steps
                .iter()
                .rev()
                .find(|earlier| {
                    matches!(earlier.action, Action::Pick | Action::Reword | Action::Edit)
                })
                .map(|earlier| earlier.number);
        }
        steps.push(step);
    }
    steps
}

/// The lines that are steps: not blank, not comments.
fn lines(text: &str) -> Vec<&str> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .collect()
}

fn step(number: usize, line: &str, state: StepState) -> Step {
    let (word, rest) = line.split_once(char::is_whitespace).unwrap_or((line, ""));
    let action = Action::parse(word);
    let mut rest = rest.trim_start();
    let (short_id, subject) = if action.has_commit() {
        // `fixup -C <commit>` and `fixup -c <commit>` keep the message of
        // the commit they name.
        if action == Action::Fixup && (rest.starts_with("-C ") || rest.starts_with("-c ")) {
            rest = rest[3..].trim_start();
        }
        let (id, subject) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
        (
            Some(id.chars().take(7).collect()),
            subject.trim().to_string(),
        )
    } else if action == Action::Other {
        (None, line.to_string())
    } else {
        (None, rest.to_string())
    };
    Step {
        number,
        action,
        short_id,
        subject,
        state,
        joins: None,
        replayed_id: None,
        was: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Lines done are done but the last, which git stopped on; the todo
    /// list is to come. Comments and blank lines are not steps.
    #[test]
    fn states_follow_done_and_todo() {
        let steps = steps(
            "pick 0572f717b5 Queue uploads\nreword e076115851 Explain the queue\n",
            "# Rebase 1234..5678\n\npick d66f4d49b8 Show the full name\n",
        );
        let states: Vec<StepState> = steps.iter().map(|step| step.state).collect();
        assert_eq!(
            states,
            [StepState::Done, StepState::Current, StepState::Pending]
        );
        assert_eq!(steps[1].action, Action::Reword);
        assert_eq!(steps[1].short_id.as_deref(), Some("e076115"));
        assert_eq!(steps[1].subject, "Explain the queue");
        assert_eq!(steps[2].number, 3);
    }

    /// A fixup or squash says which commit it joins, past other fixups and
    /// dropped commits, so the plan can draw it beside that commit.
    #[test]
    fn fixups_join_the_commit_before() {
        let steps = steps(
            "pick aaaaaaa One\n",
            "f bbbbbbb fixup! One\ndrop ccccccc Two\nsquash ddddddd squash! One\n\
             fixup -C eeeeeee amend! One\nexec make test\nbreak\nlabel onto\n",
        );
        assert_eq!(steps[1].action, Action::Fixup);
        assert_eq!(steps[1].joins, Some(1));
        assert_eq!(steps[3].joins, Some(1));
        assert_eq!(steps[4].short_id.as_deref(), Some("eeeeeee"));
        assert_eq!(steps[4].joins, Some(1));
        assert_eq!(steps[2].joins, None);
        assert_eq!(steps[5].action, Action::Exec);
        assert_eq!(steps[5].subject, "make test");
        assert_eq!(steps[6].action, Action::Break);
        assert_eq!(steps[7].action, Action::Other);
        assert_eq!(steps[7].subject, "label onto");
    }
}
