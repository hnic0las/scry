//! Explain on lines selected in the Changes window: the asks the app
//! makes and the answers it reads (`scry_core::line_asks` keeps them).

use std::time::{SystemTime, UNIX_EPOCH};

use scry_core::line_asks::{self, Explained};
use scry_core::{asks, records};

use crate::changes::{Asked, ChangeSet};
use crate::{Explanation, ScryError, failed};

/// Lines of the file the user asked about, and where the ask stands.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct LineExplanation {
    pub id: String,
    /// Where the lines are in the file as shown, from 1; where they were
    /// with `stale`.
    pub first: u32,
    pub last: u32,
    /// The file no longer holds the lines the explanation is about.
    pub stale: bool,
    /// When the user asked, in milliseconds since 1970, while the ask
    /// waits for its answer.
    pub asked_at: Option<u64>,
    /// The last answer, kept while a new ask waits.
    pub answer: Option<Explanation>,
}

/// Where Explain stands on the lines of one file.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct LineExplanations {
    /// Lines of this file can be asked about: it is a text file in a
    /// repository.
    pub available: bool,
    /// A Claude Code session watches the repository (`git scry wait`).
    pub watching: bool,
    /// Ordered by line.
    pub items: Vec<LineExplanation>,
}

fn explained(asked: &Asked) -> Vec<Explained> {
    let lines = line_asks::side(&asked.diff, asked.old_side);
    asked.store.explained(&asked.path, lines, &asked.target)
}

fn refused() -> ScryError {
    ScryError::Refused {
        message: String::from("Explain needs a text file in a git repository."),
    }
}

#[uniffi::export]
impl ChangeSet {
    /// Where Explain stands on the lines of the file at `index` of the
    /// last list: read from the repository each time, since the agent
    /// answers from outside.
    pub fn line_explanations(&self, index: u32) -> LineExplanations {
        let Some(asked) = self.asked(index) else {
            return LineExplanations {
                available: false,
                watching: false,
                items: Vec::new(),
            };
        };
        LineExplanations {
            available: true,
            watching: asks::watching(&asked.git_dir, SystemTime::now()),
            items: explained(&asked)
                .into_iter()
                .map(|one| LineExplanation {
                    first: one.first as u32,
                    last: one.last as u32,
                    stale: one.stale,
                    asked_at: one.waiting.then_some(one.ask.at),
                    answer: one.answer.map(|answer| Explanation {
                        text: answer.text,
                        by: records::agent_name(&answer.by),
                        at: answer.at,
                        failed: answer.failed,
                    }),
                    id: one.ask.id,
                })
                .collect(),
        }
    }

    /// Asks the agent watching the repository to explain lines
    /// `first..=last` (from 1) of the file at `index`: of its new side,
    /// or of the old one for a deleted file. An ask touching lines
    /// already explained replaces those explanations and covers their
    /// lines too. Gives the ask's id.
    pub fn ask_lines(&self, index: u32, first: u32, last: u32) -> Result<String, ScryError> {
        let asked = self.asked(index).ok_or_else(refused)?;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |time| time.as_millis() as u64);
        let (ask, replaced) = line_asks::prepare(
            &asked.path,
            &asked.diff,
            asked.old_side,
            first as usize,
            last as usize,
            asked.target.clone(),
            &asked.what,
            &explained(&asked),
            now,
        )
        .map_err(|error| ScryError::Refused {
            message: error.to_string(),
        })?;
        let id = ask.id.clone();
        asked
            .store
            .ask(&asked.path, ask, &replaced)
            .map_err(|error| failed(format!("The ask could not be kept: {error}")))?;
        Ok(id)
    }

    /// Takes back the waiting ask `id` about the file at `index`; its
    /// last answer stays.
    pub fn cancel_lines(&self, index: u32, id: String) -> Result<(), ScryError> {
        let asked = self.asked(index).ok_or_else(refused)?;
        asked
            .store
            .cancel(&asked.path, &id)
            .map_err(|error| failed(format!("The ask could not be taken back: {error}")))
    }

    /// Removes the explanation `id` of the file at `index`.
    pub fn forget_lines(&self, index: u32, id: String) -> Result<(), ScryError> {
        let asked = self.asked(index).ok_or_else(refused)?;
        asked
            .store
            .forget(&asked.path, &id)
            .map_err(|error| failed(format!("The explanation could not be removed: {error}")))
    }
}

#[cfg(test)]
mod tests {
    use crate::ChangeSet;
    use scry_core::line_asks::{LineAnswer, LineStore};
    use std::fs;
    use std::process::Command;

    fn git(dir: &std::path::Path, args: &[&str]) {
        let status = Command::new("git")
            .current_dir(dir)
            .args([
                "-c",
                "user.name=Scry Test",
                "-c",
                "user.email=test@example.com",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(args)
            .status();
        assert!(status.unwrap().success(), "git {args:?}");
    }

    /// The window asks about lines, the agent answers from outside, the
    /// file changes under the answer: it follows its lines, then is
    /// stale. A deleted file is asked about on its old side.
    #[test]
    fn lines_are_asked_about_answered_and_followed() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        git(root, &["init", "--quiet", "--initial-branch=main"]);
        fs::write(root.join("a.txt"), "one\ntwo\nthree\n").unwrap();
        fs::write(root.join("gone.txt"), "old\nlines\n").unwrap();
        git(root, &["add", "."]);
        git(root, &["commit", "--quiet", "-m", "Start"]);
        fs::write(root.join("a.txt"), "one\ntwo\nthree\nfour\nfive\n").unwrap();
        fs::remove_file(root.join("gone.txt")).unwrap();

        let open = || ChangeSet::open(root.display().to_string(), false, None, Vec::new()).unwrap();
        let changes = open();
        let files = changes.files().unwrap();
        let place = |name: &str| files.iter().position(|file| file.path == name).unwrap() as u32;
        let a = place("a.txt");

        let state = changes.line_explanations(a);
        assert!(state.available && !state.watching && state.items.is_empty());
        let id = changes.ask_lines(a, 4, 5).unwrap();
        let state = changes.line_explanations(a);
        assert_eq!(state.items.len(), 1);
        assert_eq!((state.items[0].first, state.items[0].last), (4, 5));
        assert!(state.items[0].asked_at.is_some() && state.items[0].answer.is_none());
        assert!(changes.ask_lines(a, 5, 9).is_err());

        let store = LineStore::new(&root.join(".git"));
        let asks = store.asks("a.txt");
        assert_eq!(asks[&id].lines, ["four", "five"]);
        assert_eq!(asks[&id].what, changes.header().sentence);
        store
            .answer(
                "a.txt",
                LineAnswer {
                    id: id.clone(),
                    by: String::from("claude-code 2.1.330"),
                    text: String::from("Two more numbers."),
                    at: u64::MAX,
                    failed: false,
                },
            )
            .unwrap();
        let state = changes.line_explanations(a);
        assert!(state.items[0].asked_at.is_none());
        assert_eq!(state.items[0].answer.as_ref().unwrap().by, "Claude Code");

        // A line added above: the list is read again, as the window does.
        fs::write(root.join("a.txt"), "zero\none\ntwo\nthree\nfour\nfive\n").unwrap();
        changes.files().unwrap();
        let state = changes.line_explanations(a);
        assert_eq!(
            (
                state.items[0].first,
                state.items[0].last,
                state.items[0].stale
            ),
            (5, 6, false)
        );
        fs::write(root.join("a.txt"), "zero\none\ntwo\nthree\n4\nfive\n").unwrap();
        changes.files().unwrap();
        assert!(changes.line_explanations(a).items[0].stale);
        changes.forget_lines(a, id).unwrap();
        assert!(changes.line_explanations(a).items.is_empty());

        // The deleted file: its old side's lines, in the commit.
        let gone = place("gone.txt");
        let id = changes.ask_lines(gone, 1, 2).unwrap();
        assert_eq!(store.asks("gone.txt")[&id].lines, ["old", "lines"]);
        changes.cancel_lines(gone, id).unwrap();
        assert!(changes.line_explanations(gone).items.is_empty());
    }
}
