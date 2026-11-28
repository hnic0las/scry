//! Explain on selected lines of the Changes window: what the user asked
//! an agent about lines of a file, and the agent's answers.
//!
//! These asks are made outside any operation, so they are not kept with
//! a stop's review ([`crate::records::Store`]) but in
//! `<git dir>/scry/lines/`, one file of asks (written by the app) and one
//! of answers (written by the MCP server) per file of the repository.
//! Nothing appears in `git status`, and nothing is written into the
//! repository's files.
//!
//! An explanation is about the text of its lines, not their numbers: it
//! follows them when they move in the file, and is stale once the file
//! no longer holds them.

use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::diff::{LineKind, TextDiff, UnifiedLine, fnv};
use crate::file_blame::Target;
use crate::records::{decoded, encoded};
use crate::session::write_atomically;

/// The most lines one ask is about: more is a file to read, not lines to
/// explain.
pub const MAX_LINES: usize = 400;
/// Lines of the change kept around the selected ones, each way.
pub const CONTEXT: usize = 12;

/// What the user asked about lines of a file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LineAsk {
    /// `L` and eight hexadecimal digits: never a conflict's id.
    pub id: String,
    /// The first and last line, from 1, in the version `target` names,
    /// when asked.
    pub first: usize,
    pub last: usize,
    /// The text of those lines: what the answer is about.
    pub lines: Vec<String>,
    /// What the window compared, in its words ("Not committed, against
    /// HEAD").
    pub what: String,
    /// The change around the lines as the window showed it, for the
    /// agent: see [`excerpt`].
    pub excerpt: String,
    /// The version the lines are in.
    pub target: Target,
    /// Milliseconds since 1970: asking again opens the ask again.
    pub at: u64,
}

/// An agent's answer to a [`LineAsk`], written by the MCP server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LineAnswer {
    pub id: String,
    /// The agent's name and version, as its MCP client gave them.
    pub by: String,
    /// The explanation, or with `failed` why there is none.
    pub text: String,
    /// Milliseconds since 1970.
    pub at: u64,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub failed: bool,
}

/// An ask with where it stands in the version of the file being shown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Explained {
    pub ask: LineAsk,
    /// The last answer, kept while a newer ask waits.
    pub answer: Option<LineAnswer>,
    /// No answer came since it was asked.
    pub waiting: bool,
    /// Where its lines are now, from 1; where they were when the file no
    /// longer holds them.
    pub first: usize,
    pub last: usize,
    /// The file no longer holds the lines it is about.
    pub stale: bool,
}

/// Why lines cannot be asked about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AskError {
    /// The file has no such lines on that side.
    NoSuchLines,
    TooManyLines(usize),
}

impl fmt::Display for AskError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AskError::NoSuchLines => write!(f, "These lines are no longer in the file."),
            AskError::TooManyLines(count) => write!(
                f,
                "{count} lines are selected: Explain takes {MAX_LINES} at most."
            ),
        }
    }
}

impl std::error::Error for AskError {}

/// The lines of one side of `diff`.
pub fn side(diff: &TextDiff, old_side: bool) -> &[String] {
    match old_side {
        true => &diff.old_lines,
        false => &diff.new_lines,
    }
}

/// The ask about lines `first..=last` (from 1) of one side of `diff`,
/// and the ids of the explanations it replaces. Two explanations never
/// overlap: the ask is extended to the lines of those it touches, which
/// it replaces. Asked again on the very lines of one explanation, it
/// keeps its id, so the last answer stays until the new one comes.
#[allow(clippy::too_many_arguments)]
pub fn prepare(
    path: &str,
    diff: &TextDiff,
    old_side: bool,
    first: usize,
    last: usize,
    target: Target,
    what: &str,
    existing: &[Explained],
    now: u64,
) -> Result<(LineAsk, Vec<String>), AskError> {
    let lines = side(diff, old_side);
    if first == 0 || first > last || last > lines.len() {
        return Err(AskError::NoSuchLines);
    }
    let touched: Vec<&Explained> = existing
        .iter()
        .filter(|explained| explained.first <= last && first <= explained.last)
        .collect();
    let first = touched.iter().map(|one| one.first).fold(first, usize::min);
    let last = touched
        .iter()
        .map(|one| one.last)
        .fold(last, usize::max)
        .min(lines.len());
    if last - first + 1 > MAX_LINES {
        return Err(AskError::TooManyLines(last - first + 1));
    }
    let text = lines[first - 1..last].to_vec();
    let same = match touched.as_slice() {
        [one] if !one.stale && one.first == first && one.last == last => Some(one.ask.id.clone()),
        _ => None,
    };
    let replaced = match same {
        Some(_) => Vec::new(),
        None => touched.iter().map(|one| one.ask.id.clone()).collect(),
    };
    let id = same.unwrap_or_else(|| {
        let seed = format!("{path}\n{first}\n{now}\n{}", text.join("\n"));
        format!("L{:08x}", fnv(seed.as_bytes()) as u32)
    });
    let ask = LineAsk {
        id,
        first,
        last,
        lines: text,
        what: what.to_string(),
        excerpt: excerpt(diff, old_side, first, last),
        target,
        at: now,
    };
    Ok((ask, replaced))
}

/// The change around lines `first..=last` of one side of `diff`, as the
/// unified view shows it, with [`CONTEXT`] lines each way: one line per
/// row, `>` before the selected ones, the line's number, then `+` for an
/// added line, `-` for a removed one (numbered on the old side), and the
/// text.
pub fn excerpt(diff: &TextDiff, old_side: bool, first: usize, last: usize) -> String {
    let unified = diff.unified();
    let selected = |line: &UnifiedLine| {
        let number = if old_side { line.old } else { line.new };
        number.is_some_and(|number| (first..=last).contains(&number))
    };
    let (Some(start), Some(end)) = (
        unified.iter().position(selected),
        unified.iter().rposition(selected),
    ) else {
        return String::new();
    };
    let from = start.saturating_sub(CONTEXT);
    let to = (end + CONTEXT).min(unified.len() - 1);
    let text = |lines: &[String], number: Option<usize>| -> String {
        number
            .and_then(|number| lines.get(number.checked_sub(1)?))
            .cloned()
            .unwrap_or_default()
    };
    let mut excerpt = String::new();
    for line in &unified[from..=to] {
        let (sign, number, text) = match line.kind {
            LineKind::Added => ('+', line.new, text(&diff.new_lines, line.new)),
            LineKind::Removed => ('-', line.old, text(&diff.old_lines, line.old)),
            LineKind::Unchanged if old_side => (' ', line.old, text(&diff.old_lines, line.old)),
            LineKind::Unchanged => (' ', line.new, text(&diff.new_lines, line.new)),
        };
        let mark = if selected(line) { '>' } else { ' ' };
        let number = number.map(|number| number.to_string()).unwrap_or_default();
        excerpt.push_str(&format!("{mark}{number:>6} {sign} {text}\n"));
    }
    excerpt
}

/// Where the lines `ask` is about are in `lines`, from 1: where they
/// were, or the nearest place holding the same text. `None` when the
/// file no longer holds them.
pub fn locate(lines: &[String], ask: &LineAsk) -> Option<(usize, usize)> {
    let count = ask.lines.len();
    if count == 0 || count > lines.len() {
        return None;
    }
    let held_at = |start: &usize| lines[*start..*start + count] == ask.lines[..];
    let home = ask.first.saturating_sub(1);
    let start = if home + count <= lines.len() && held_at(&home) {
        home
    } else {
        (0..=lines.len() - count)
            .filter(held_at)
            .min_by_key(|start| start.abs_diff(home))?
    };
    Some((start + 1, start + count))
}

/// The asks about lines and their answers, for one repository.
pub struct LineStore {
    folder: PathBuf,
}

impl LineStore {
    /// The store of the repository whose git folder is `git_dir`.
    pub fn new(git_dir: &Path) -> LineStore {
        LineStore {
            folder: git_dir.join("scry").join("lines"),
        }
    }

    /// The folder the asks and answers are in.
    pub fn folder(&self) -> &Path {
        &self.folder
    }

    /// The asks about `path` (from the repository's root), by id.
    pub fn asks(&self, path: &str) -> BTreeMap<String, LineAsk> {
        read(&self.file("asks", path)).unwrap_or_default()
    }

    /// The answers about `path`, by ask id.
    pub fn answers(&self, path: &str) -> BTreeMap<String, LineAnswer> {
        read(&self.file("answers", path)).unwrap_or_default()
    }

    /// Keeps `ask`, in place of the explanations `replaces` names.
    pub fn ask(&self, path: &str, ask: LineAsk, replaces: &[String]) -> io::Result<()> {
        let mut asks = self.asks(path);
        for id in replaces {
            asks.remove(id);
        }
        asks.insert(ask.id.clone(), ask);
        self.write(&self.file("asks", path), &asks)?;
        let mut answers = self.answers(path);
        let before = answers.len();
        answers.retain(|id, _| !replaces.contains(id));
        if answers.len() != before {
            self.write(&self.file("answers", path), &answers)?;
        }
        Ok(())
    }

    /// Takes back the waiting ask `id`: its last answer stays; with none,
    /// the ask is gone.
    pub fn cancel(&self, path: &str, id: &str) -> io::Result<()> {
        let mut asks = self.asks(path);
        let Some(ask) = asks.get_mut(id) else {
            return Ok(());
        };
        match self.answers(path).get(id) {
            Some(answer) if answer.at >= ask.at => return Ok(()),
            Some(answer) => ask.at = answer.at,
            None => {
                asks.remove(id);
            }
        }
        self.write(&self.file("asks", path), &asks)
    }

    /// Removes the explanation `id` and its ask.
    pub fn forget(&self, path: &str, id: &str) -> io::Result<()> {
        let mut asks = self.asks(path);
        if asks.remove(id).is_some() {
            self.write(&self.file("asks", path), &asks)?;
        }
        let mut answers = self.answers(path);
        if answers.remove(id).is_some() {
            self.write(&self.file("answers", path), &answers)?;
        }
        Ok(())
    }

    /// Keeps the agent's answer, replacing an earlier one.
    pub fn answer(&self, path: &str, answer: LineAnswer) -> io::Result<()> {
        let mut answers = self.answers(path);
        answers.insert(answer.id.clone(), answer);
        self.write(&self.file("answers", path), &answers)
    }

    /// Every ask still waiting for its answer, with its file, oldest
    /// first.
    pub fn open_asks(&self) -> Vec<(String, LineAsk)> {
        let Ok(entries) = fs::read_dir(&self.folder) else {
            return Vec::new();
        };
        let mut open: Vec<(String, LineAsk)> = entries
            .filter_map(|entry| {
                let name = entry.ok()?.file_name().into_string().ok()?;
                decoded(name.strip_prefix("asks-")?.strip_suffix(".json")?)
            })
            .flat_map(|path| {
                let answers = self.answers(&path);
                self.asks(&path)
                    .into_values()
                    .filter(|ask| waiting(ask, answers.get(&ask.id)))
                    .map(|ask| (path.clone(), ask))
                    .collect::<Vec<_>>()
            })
            .collect();
        open.sort_by(|a, b| (a.1.at, &a.0).cmp(&(b.1.at, &b.0)));
        open
    }

    /// The explanations of `path` to show beside `lines`, the lines of
    /// the version `target` of the file, ordered by line. One whose lines
    /// are not in `lines` is stale when it was asked about this version,
    /// and left out when it was asked about another.
    pub fn explained(&self, path: &str, lines: &[String], target: &Target) -> Vec<Explained> {
        let mut answers = self.answers(path);
        let mut explained: Vec<Explained> = self
            .asks(path)
            .into_values()
            .filter_map(|ask| {
                let (first, last, stale) = match locate(lines, &ask) {
                    Some((first, last)) => (first, last, false),
                    None if ask.target == *target => (ask.first, ask.last, true),
                    None => return None,
                };
                let answer = answers.remove(&ask.id);
                Some(Explained {
                    waiting: waiting(&ask, answer.as_ref()),
                    ask,
                    answer,
                    first,
                    last,
                    stale,
                })
            })
            .collect();
        explained.sort_by_key(|one| (one.first, one.last));
        explained
    }

    fn file(&self, kind: &str, path: &str) -> PathBuf {
        self.folder.join(format!("{kind}-{}.json", encoded(path)))
    }

    fn write<T: Serialize>(&self, file: &Path, value: &T) -> io::Result<()> {
        fs::create_dir_all(&self.folder)?;
        let text = serde_json::to_string_pretty(value).map_err(io::Error::other)?;
        write_atomically(file, &text)
    }
}

fn read<T: for<'de> Deserialize<'de>>(file: &Path) -> Option<T> {
    serde_json::from_slice(&fs::read(file).ok()?).ok()
}

/// An ask waits until an answer comes at or after it.
fn waiting(ask: &LineAsk, answer: Option<&LineAnswer>) -> bool {
    !answer.is_some_and(|answer| answer.at >= ask.at)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::{Marks, parse_patch};

    /// Ten lines where line 3 was changed and a line was added after 6.
    fn diff() -> TextDiff {
        let patch = "\
diff --git a/f.txt b/f.txt
--- a/f.txt
+++ b/f.txt
@@ -1,9 +1,10 @@
 one
 two
-three
+THREE
 four
 five
 six
+six and a half
 seven
 eight
 nine
";
        TextDiff::from_patch(parse_patch(patch), Marks::Auto)
    }

    fn ask_on(
        first: usize,
        last: usize,
        existing: &[Explained],
        now: u64,
    ) -> (LineAsk, Vec<String>) {
        prepare(
            "f.txt",
            &diff(),
            false,
            first,
            last,
            Target::WorkingCopy,
            "Not committed, against HEAD",
            existing,
            now,
        )
        .unwrap()
    }

    fn answer(id: &str, at: u64) -> LineAnswer {
        LineAnswer {
            id: id.to_string(),
            by: String::from("claude-code 2.1.330"),
            text: String::from("These lines do this."),
            at,
            failed: false,
        }
    }

    #[test]
    fn an_ask_keeps_its_lines_and_the_change_around_them() {
        let (ask, replaced) = ask_on(3, 4, &[], 10);
        assert!(replaced.is_empty());
        assert!(ask.id.starts_with('L') && ask.id.len() == 9, "{}", ask.id);
        assert_eq!(ask.lines, ["THREE", "four"]);
        assert_eq!(
            ask.excerpt,
            "      1   one\n      2   two\n      3 - three\n>     3 + THREE\n>     4   four\n      5   five\n      6   six\n      7 + six and a half\n      8   seven\n      9   eight\n     10   nine\n"
        );
    }

    #[test]
    fn lines_outside_the_file_or_too_many_are_refused() {
        let refused = |first, last| {
            prepare(
                "f.txt",
                &diff(),
                false,
                first,
                last,
                Target::WorkingCopy,
                "",
                &[],
                1,
            )
        };
        assert_eq!(refused(0, 2), Err(AskError::NoSuchLines));
        assert_eq!(refused(4, 3), Err(AskError::NoSuchLines));
        assert_eq!(refused(9, 11), Err(AskError::NoSuchLines));
        // The old side has nine lines.
        assert!(
            prepare(
                "f.txt",
                &diff(),
                true,
                9,
                9,
                Target::WorkingCopy,
                "",
                &[],
                1
            )
            .is_ok()
        );
        assert_eq!(
            prepare(
                "f.txt",
                &diff(),
                true,
                10,
                10,
                Target::WorkingCopy,
                "",
                &[],
                1
            ),
            Err(AskError::NoSuchLines)
        );
    }

    #[test]
    fn an_explanation_follows_its_lines_and_is_stale_without_them() {
        let dir = tempfile::tempdir().unwrap();
        let store = LineStore::new(dir.path());
        let (ask, _) = ask_on(3, 4, &[], 10);
        let id = ask.id.clone();
        store.ask("f.txt", ask, &[]).unwrap();
        assert_eq!(store.open_asks().len(), 1);
        store.answer("f.txt", answer(&id, 20)).unwrap();
        assert!(store.open_asks().is_empty());

        let lines = diff().new_lines;
        let shown = store.explained("f.txt", &lines, &Target::WorkingCopy);
        assert_eq!((shown[0].first, shown[0].last), (3, 4));
        assert!(!shown[0].stale && !shown[0].waiting);

        // Two lines added above: the explanation moves with its lines.
        let mut moved = vec![String::from("zero"), String::from("zero")];
        moved.extend(lines.clone());
        let shown = store.explained("f.txt", &moved, &Target::WorkingCopy);
        assert_eq!(
            (shown[0].first, shown[0].last, shown[0].stale),
            (5, 6, false)
        );

        // The lines rewritten: stale where it was, in the version it was
        // asked about; absent from another version.
        let mut rewritten = lines.clone();
        rewritten[2] = String::from("3");
        let shown = store.explained("f.txt", &rewritten, &Target::WorkingCopy);
        assert_eq!(
            (shown[0].first, shown[0].last, shown[0].stale),
            (3, 4, true)
        );
        assert!(
            store
                .explained("f.txt", &rewritten, &Target::Commit(String::from("abc")))
                .is_empty()
        );
        // Another version holding the same lines shows it.
        assert_eq!(
            store
                .explained("f.txt", &lines, &Target::Commit(String::from("abc")))
                .len(),
            1
        );
    }

    #[test]
    fn asking_again_keeps_the_answer_until_the_new_one() {
        let dir = tempfile::tempdir().unwrap();
        let store = LineStore::new(dir.path());
        let lines = diff().new_lines;
        let (ask, _) = ask_on(3, 4, &[], 10);
        let id = ask.id.clone();
        store.ask("f.txt", ask, &[]).unwrap();
        store.answer("f.txt", answer(&id, 20)).unwrap();

        let shown = store.explained("f.txt", &lines, &Target::WorkingCopy);
        let (again, replaced) = ask_on(3, 4, &shown, 30);
        assert_eq!(again.id, id);
        assert!(replaced.is_empty());
        store.ask("f.txt", again, &replaced).unwrap();
        let shown = store.explained("f.txt", &lines, &Target::WorkingCopy);
        assert!(shown[0].waiting && shown[0].answer.is_some());

        // Taken back: the last answer stays, nothing waits.
        store.cancel("f.txt", &id).unwrap();
        let shown = store.explained("f.txt", &lines, &Target::WorkingCopy);
        assert!(!shown[0].waiting && shown[0].answer.is_some());
        assert!(store.open_asks().is_empty());
    }

    #[test]
    fn an_ask_never_answered_is_gone_when_taken_back() {
        let dir = tempfile::tempdir().unwrap();
        let store = LineStore::new(dir.path());
        let (ask, _) = ask_on(1, 1, &[], 10);
        let id = ask.id.clone();
        store.ask("f.txt", ask, &[]).unwrap();
        store.cancel("f.txt", &id).unwrap();
        assert!(store.asks("f.txt").is_empty());
    }

    #[test]
    fn an_ask_touching_explanations_replaces_them_on_all_their_lines() {
        let dir = tempfile::tempdir().unwrap();
        let store = LineStore::new(dir.path());
        let lines = diff().new_lines;
        let (first, _) = ask_on(2, 3, &[], 10);
        let (second, _) = ask_on(6, 7, &[], 11);
        let ids = [first.id.clone(), second.id.clone()];
        store.ask("f.txt", first, &[]).unwrap();
        store.ask("f.txt", second, &[]).unwrap();
        store.answer("f.txt", answer(&ids[0], 20)).unwrap();

        let shown = store.explained("f.txt", &lines, &Target::WorkingCopy);
        let (wide, replaced) = ask_on(3, 6, &shown, 30);
        assert_eq!((wide.first, wide.last), (2, 7));
        assert_eq!(replaced, ids);
        store.ask("f.txt", wide.clone(), &replaced).unwrap();
        assert_eq!(
            store.asks("f.txt").into_keys().collect::<Vec<_>>(),
            [wide.id]
        );
        assert!(store.answers("f.txt").is_empty());
    }

    #[test]
    fn a_selection_of_the_old_side_is_marked_on_removed_lines() {
        let text = excerpt(&diff(), true, 3, 3);
        assert!(text.contains(">     3 - three\n"), "{text}");
        assert!(text.contains("      3 + THREE\n"), "{text}");
        // Lines both sides hold are numbered on the old side too.
        assert!(text.contains("      7   seven\n"), "{text}");
    }

    /// A file of `count` lines, all added.
    fn added(count: usize) -> TextDiff {
        let mut patch = format!(
            "diff --git a/f.txt b/f.txt\n--- /dev/null\n+++ b/f.txt\n@@ -0,0 +1,{count} @@\n"
        );
        for number in 1..=count {
            patch += &format!("+line {number}\n");
        }
        TextDiff::from_patch(parse_patch(&patch), Marks::Auto)
    }

    #[test]
    fn at_most_four_hundred_lines_are_asked_about() {
        let diff = added(MAX_LINES + 1);
        let ask = |first, last| {
            prepare(
                "f.txt",
                &diff,
                false,
                first,
                last,
                Target::WorkingCopy,
                "",
                &[],
                1,
            )
        };
        assert!(ask(1, MAX_LINES).is_ok());
        assert!(ask(2, MAX_LINES + 1).is_ok());
        let refused = ask(1, MAX_LINES + 1);
        assert_eq!(refused, Err(AskError::TooManyLines(MAX_LINES + 1)));
        assert!(refused.unwrap_err().to_string().contains("401"));
        assert_eq!(
            AskError::NoSuchLines.to_string(),
            "These lines are no longer in the file."
        );
    }

    #[test]
    fn the_excerpt_keeps_twelve_lines_each_way() {
        let text = excerpt(&added(40), false, 20, 20);
        let numbers: Vec<&str> = text.lines().map(|line| line[1..7].trim()).collect();
        assert_eq!(numbers.first(), Some(&"8"));
        assert_eq!(numbers.last(), Some(&"32"));
        assert_eq!(numbers.len(), 25);
    }

    #[test]
    fn an_ask_beside_an_explanation_leaves_it_and_one_overlapping_replaces_it() {
        let dir = tempfile::tempdir().unwrap();
        let store = LineStore::new(dir.path());
        let lines = diff().new_lines;
        let (ask, _) = ask_on(2, 3, &[], 10);
        let id = ask.id.clone();
        store.ask("f.txt", ask, &[]).unwrap();
        store.answer("f.txt", answer(&id, 20)).unwrap();
        let shown = store.explained("f.txt", &lines, &Target::WorkingCopy);

        // Lines after it, and before it: not touched.
        let (after, replaced) = ask_on(5, 6, &shown, 30);
        assert!(replaced.is_empty());
        assert_eq!((after.first, after.last), (5, 6));
        let (before, replaced) = ask_on(1, 1, &shown, 30);
        assert!(replaced.is_empty() && before.id != id);

        // Overlapping it without being its lines: a new ask on both.
        let (wider, replaced) = ask_on(3, 4, &shown, 30);
        assert_eq!((wider.first, wider.last), (2, 4));
        assert_ne!(wider.id, id);
        assert_eq!(replaced, std::slice::from_ref(&id));

        // Its very lines, but stale: a new ask too.
        let mut stale = shown.clone();
        stale[0].stale = true;
        let (again, replaced) = ask_on(2, 3, &stale, 30);
        assert_ne!(again.id, id);
        assert_eq!(replaced, [id]);
    }

    #[test]
    fn lines_are_found_at_the_edges_of_a_file() {
        let (mut ask, _) = ask_on(9, 10, &[], 1);
        let lines = diff().new_lines;
        // At the very end of the file.
        assert_eq!(locate(&lines, &ask), Some((9, 10)));
        // The file is exactly the lines.
        assert_eq!(locate(&ask.lines.clone(), &ask), Some((1, 2)));
        // The file is shorter than the lines.
        assert_eq!(locate(&lines[..1], &ask), None);
        // The file shrank under the place they were: found where they are.
        ask.lines = vec![String::from("one")];
        ask.first = 2;
        assert_eq!(locate(&lines[..1], &ask), Some((1, 1)));
    }
}
