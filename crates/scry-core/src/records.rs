//! What Scry keeps about a review in the repository's git folder: who
//! resolved each conflict outside Scry and why (written by the MCP
//! server's `resolve`), which resolutions the user reviewed (written by
//! the app), what the user asked an agent about a conflict (written by
//! the app) and the agent's answers (written by the MCP server).
//!
//! Everything lives in `<git dir>/scry/review/<stop>/`, so nothing appears
//! in `git status`, each linked worktree has its own, and a stop of the
//! operation (a merge, one commit of a rebase) never sees another's. The
//! server and the app each write their own files, atomically, so neither
//! needs a lock.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::git::{Git, GitError};
use crate::resolution::CandidateKind;
use crate::session::write_atomically;

/// How an agent resolved one conflict, as it said.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Record {
    /// The conflict's id.
    pub conflict: String,
    /// The agent's name and version, as its MCP client gave them: its
    /// word, not verified.
    pub by: String,
    /// The candidate it picked, when it picked one.
    pub pick: Option<CandidateKind>,
    /// The text written in the file, so a record no longer matching the
    /// file is told apart.
    pub text: String,
    pub reason: String,
    /// For text of its own: what of either side it left out.
    pub dropped: Option<String>,
    /// Seconds since 1970.
    pub at: u64,
}

/// What the user can ask an agent about a conflict.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AskKind {
    /// What each side meant, the risk of each candidate.
    Explain,
}

impl AskKind {
    /// The word the asks are printed with, as serialized.
    pub fn word(self) -> &'static str {
        match self {
            AskKind::Explain => "explain",
        }
    }
}

/// What the user asked an agent about one conflict, from the app. It is
/// open until an answer of the same kind comes after it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ask {
    /// The conflict's id.
    pub conflict: String,
    pub kind: AskKind,
    /// Milliseconds since 1970: asking again right after an answer opens
    /// the ask again.
    pub at: u64,
}

/// An agent's answer to an ask, written by the MCP server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Answer {
    /// The conflict's id.
    pub conflict: String,
    pub kind: AskKind,
    /// The agent's name and version, as its MCP client gave them.
    pub by: String,
    pub text: String,
    /// Milliseconds since 1970.
    pub at: u64,
}

/// An ask still waiting for its answer, with the file it is about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenAsk {
    /// The file, from the repository root.
    pub path: String,
    pub ask: Ask,
}

/// The review of the operation's current stop.
pub struct Store {
    folder: PathBuf,
}

/// The heads that name what git is applying, in the order they are looked
/// for: one of them, with `HEAD`, names the stop.
const OPERATION_HEADS: [&str; 4] = [
    "MERGE_HEAD",
    "REBASE_HEAD",
    "CHERRY_PICK_HEAD",
    "REVERT_HEAD",
];

impl Store {
    /// The store of the stop the repository is at, with `git` running in
    /// it.
    pub fn open(git: &dyn Git) -> Result<Store, GitError> {
        let git_dir = git.run(&["rev-parse", "--absolute-git-dir"])?;
        let commit = |name: &str| -> Option<String> {
            git.run(&[
                "rev-parse",
                "--quiet",
                "--verify",
                &format!("{name}^{{commit}}"),
            ])
            .ok()
            .map(|id| id.trim().to_string())
        };
        let head = commit("HEAD").unwrap_or_else(|| String::from("none"));
        let applied = OPERATION_HEADS
            .iter()
            .find_map(|name| commit(name))
            .unwrap_or_else(|| String::from("none"));
        Ok(Store::at(
            &Path::new(git_dir.trim_end()).join("scry").join("review"),
            &format!("{head}-{applied}"),
        ))
    }

    /// The store of `stop` in `reviews` (`<git dir>/scry/review`).
    fn at(reviews: &Path, stop: &str) -> Store {
        Store {
            folder: reviews.join(stop),
        }
    }

    /// The records of `path` (from the repository root), by conflict id;
    /// none when nothing was recorded or the file cannot be read.
    pub fn records(&self, path: &str) -> BTreeMap<String, Record> {
        read(&self.file("records", path)).unwrap_or_default()
    }

    /// Records how a conflict of `path` was resolved, replacing what was
    /// recorded for that conflict before.
    pub fn add_record(&self, path: &str, record: Record) -> io::Result<()> {
        let mut records = self.records(path);
        records.insert(record.conflict.clone(), record);
        self.write(&self.file("records", path), &records)
    }

    /// The conflicts of `path` the user reviewed, each with the text it
    /// held then: a resolution changed since is no longer reviewed.
    pub fn reviewed(&self, path: &str) -> BTreeMap<String, String> {
        read(&self.file("reviewed", path)).unwrap_or_default()
    }

    /// Replaces the reviewed conflicts of `path`.
    pub fn set_reviewed(&self, path: &str, reviewed: &BTreeMap<String, String>) -> io::Result<()> {
        self.write(&self.file("reviewed", path), reviewed)
    }

    /// What the user asked about the conflicts of `path`, by conflict id:
    /// the last ask of each conflict.
    pub fn asks(&self, path: &str) -> BTreeMap<String, Ask> {
        read(&self.file("asks", path)).unwrap_or_default()
    }

    /// Asks about a conflict of `path`, replacing an earlier ask about it.
    pub fn ask(&self, path: &str, ask: Ask) -> io::Result<()> {
        let mut asks = self.asks(path);
        asks.insert(ask.conflict.clone(), ask);
        self.write(&self.file("asks", path), &asks)
    }

    /// Takes back the ask about a conflict of `path`, answered or not.
    pub fn cancel_ask(&self, path: &str, conflict: &str) -> io::Result<()> {
        let mut asks = self.asks(path);
        if asks.remove(conflict).is_some() {
            self.write(&self.file("asks", path), &asks)?;
        }
        Ok(())
    }

    /// The agents' answers about the conflicts of `path`, by conflict id.
    pub fn answers(&self, path: &str) -> BTreeMap<String, Answer> {
        read(&self.file("answers", path)).unwrap_or_default()
    }

    /// Keeps an agent's answer about a conflict of `path`, replacing an
    /// earlier one: an agent correcting itself leaves one answer.
    pub fn answer(&self, path: &str, answer: Answer) -> io::Result<()> {
        let mut answers = self.answers(path);
        answers.insert(answer.conflict.clone(), answer);
        self.write(&self.file("answers", path), &answers)
    }

    /// Whether the ask about `conflict` of `path` waits for its answer.
    pub fn is_open(&self, path: &str, conflict: &str) -> bool {
        self.asks(path)
            .get(conflict)
            .is_some_and(|ask| open(ask, self.answers(path).get(conflict)))
    }

    /// Every ask of this stop still waiting for its answer, oldest first.
    pub fn open_asks(&self) -> Vec<OpenAsk> {
        let Ok(entries) = fs::read_dir(&self.folder) else {
            return Vec::new();
        };
        let mut open_asks: Vec<OpenAsk> = entries
            .filter_map(|entry| {
                let name = entry.ok()?.file_name().into_string().ok()?;
                decoded(name.strip_prefix("asks-")?.strip_suffix(".json")?)
            })
            .flat_map(|path| {
                let answers = self.answers(&path);
                self.asks(&path)
                    .into_values()
                    .filter(|ask| open(ask, answers.get(&ask.conflict)))
                    .map(|ask| OpenAsk {
                        path: path.clone(),
                        ask,
                    })
                    .collect::<Vec<_>>()
            })
            .collect();
        open_asks.sort_by(|a, b| (a.ask.at, &a.path).cmp(&(b.ask.at, &b.path)));
        open_asks
    }

    /// The folder this stop's files are in.
    pub fn folder(&self) -> &Path {
        &self.folder
    }

    /// Removes what other stops kept: the operation moved on or ended,
    /// and their conflicts will never come back.
    pub fn prune(&self) -> io::Result<()> {
        let Some(reviews) = self.folder.parent() else {
            return Ok(());
        };
        let entries = match fs::read_dir(reviews) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error),
        };
        for entry in entries {
            let path = entry?.path();
            if path != self.folder && path.is_dir() {
                fs::remove_dir_all(path)?;
            }
        }
        Ok(())
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

/// An ask is open until an answer of its kind comes at or after it.
fn open(ask: &Ask, answer: Option<&Answer>) -> bool {
    !answer.is_some_and(|answer| answer.kind == ask.kind && answer.at >= ask.at)
}

/// The path [`encoded`] gave, `None` for a name it cannot have given.
fn decoded(name: &str) -> Option<String> {
    let bytes = name.as_bytes();
    let mut path = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let hex = std::str::from_utf8(bytes.get(index + 1..index + 3)?).ok()?;
            path.push(u8::from_str_radix(hex, 16).ok()?);
            index += 3;
        } else {
            path.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(path).ok()
}

/// `path` as one file name: letters, digits, `.`, `_` and `-` as they are,
/// every other byte as `%` and two hexadecimal digits, so `a/b` and `a%2Fb`
/// never share a file.
fn encoded(path: &str) -> String {
    path.bytes()
        .map(|byte| match byte {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'.' | b'_' | b'-' => {
                char::from(byte).to_string()
            }
            _ => format!("%{byte:02X}"),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(conflict: &str, text: &str) -> Record {
        Record {
            conflict: conflict.to_string(),
            by: String::from("claude-code 2.1"),
            pick: Some(CandidateKind::Theirs),
            text: text.to_string(),
            reason: String::from("Keeps main's rename and the feature's timeout."),
            dropped: None,
            at: 1_763_300_000,
        }
    }

    /// Records and reviews read back as written, per file, and a new record
    /// for a conflict replaces the old one: the review shows the agent's
    /// last word.
    #[test]
    fn records_and_reviews_round_trip_per_file() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::at(dir.path(), "stop");
        assert!(store.records("a.txt").is_empty());
        store.add_record("a.txt", record("c1", "one\n")).unwrap();
        store.add_record("a.txt", record("c2", "two\n")).unwrap();
        store
            .add_record("a.txt", record("c1", "one again\n"))
            .unwrap();
        store
            .add_record("b/a.txt", record("c3", "three\n"))
            .unwrap();
        let records = store.records("a.txt");
        assert_eq!(records.len(), 2);
        assert_eq!(records["c1"].text, "one again\n");
        assert_eq!(store.records("b/a.txt")["c3"], record("c3", "three\n"));

        let reviewed = BTreeMap::from([(String::from("c1"), String::from("one again\n"))]);
        store.set_reviewed("a.txt", &reviewed).unwrap();
        assert_eq!(store.reviewed("a.txt"), reviewed);
        assert!(store.reviewed("b/a.txt").is_empty());
    }

    /// A file damaged by hand reads as nothing recorded, rather than
    /// stopping the review.
    #[test]
    fn a_damaged_file_reads_as_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::at(dir.path(), "stop");
        store.add_record("a.txt", record("c1", "one\n")).unwrap();
        fs::write(store.file("records", "a.txt"), "{ not json").unwrap();
        assert!(store.records("a.txt").is_empty());
    }

    /// Paths that differ only by characters a file name cannot hold, or by
    /// how they would be escaped, never share a file.
    #[test]
    fn paths_get_files_of_their_own() {
        let names = ["a/b.txt", "a%2Fb.txt", "a b.txt", "été.txt", "a_b.txt"];
        let encoded: std::collections::HashSet<String> =
            names.iter().map(|name| encoded(name)).collect();
        assert_eq!(encoded.len(), names.len());
        assert_eq!(super::encoded("a/b c.txt"), "a%2Fb%20c.txt");
    }

    fn ask(conflict: &str, at: u64) -> Ask {
        Ask {
            conflict: conflict.to_string(),
            kind: AskKind::Explain,
            at,
        }
    }

    fn answer(conflict: &str, text: &str, at: u64) -> Answer {
        Answer {
            conflict: conflict.to_string(),
            kind: AskKind::Explain,
            by: String::from("claude-code 2.1"),
            text: text.to_string(),
            at,
        }
    }

    /// An ask stays open until an answer comes at or after it; asking again
    /// opens it again, and a second answer replaces the first. Asks of
    /// every file of the stop are found, oldest first, with their path.
    #[test]
    fn asks_stay_open_until_answered() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::at(dir.path(), "stop");
        assert!(store.open_asks().is_empty());
        store.ask("src/a b.swift", ask("c1", 2_000)).unwrap();
        store.ask("b.txt", ask("c2", 1_000)).unwrap();
        assert!(store.is_open("src/a b.swift", "c1"));
        assert!(!store.is_open("src/a b.swift", "c9"));
        let open = store.open_asks();
        assert_eq!(
            open.iter()
                .map(|open| (open.path.as_str(), open.ask.conflict.as_str()))
                .collect::<Vec<_>>(),
            [("b.txt", "c2"), ("src/a b.swift", "c1")]
        );

        store
            .answer("src/a b.swift", answer("c1", "First.", 2_000))
            .unwrap();
        assert!(!store.is_open("src/a b.swift", "c1"));
        assert_eq!(store.open_asks().len(), 1);
        store
            .answer("src/a b.swift", answer("c1", "Second.", 2_500))
            .unwrap();
        assert_eq!(store.answers("src/a b.swift")["c1"].text, "Second.");
        assert_eq!(store.answers("src/a b.swift").len(), 1);

        store.ask("src/a b.swift", ask("c1", 3_000)).unwrap();
        assert!(store.is_open("src/a b.swift", "c1"));
        store.cancel_ask("src/a b.swift", "c1").unwrap();
        assert!(!store.is_open("src/a b.swift", "c1"));
        store.cancel_ask("src/a b.swift", "c1").unwrap();
        assert_eq!(store.open_asks().len(), 1);
    }

    /// Every encoded path decodes back; a name it cannot have given does
    /// not decode.
    #[test]
    fn file_names_decode_to_their_path() {
        for path in ["a/b.txt", "a%2Fb.txt", "a b.txt", "été.txt", "a_b.txt", ""] {
            assert_eq!(decoded(&encoded(path)).as_deref(), Some(path));
        }
        assert_eq!(decoded("a%2"), None);
        assert_eq!(decoded("a%zz"), None);
        assert_eq!(decoded("%FF"), None);
    }

    /// Pruning keeps the current stop and removes the others: the review of
    /// the commit a rebase stopped on before is gone once it moved on. A
    /// folder it cannot read is reported.
    #[test]
    fn pruning_keeps_only_the_current_stop() {
        let dir = tempfile::tempdir().unwrap();
        let old = Store::at(dir.path(), "old");
        old.add_record("a.txt", record("c1", "one\n")).unwrap();
        let current = Store::at(dir.path(), "current");
        current.add_record("a.txt", record("c2", "two\n")).unwrap();
        current.prune().unwrap();
        assert!(old.records("a.txt").is_empty());
        assert_eq!(current.records("a.txt").len(), 1);
        // Nothing kept yet is fine too.
        Store::at(&dir.path().join("none"), "stop").prune().unwrap();
        // A folder that cannot be read is an error, not "nothing kept".
        fs::write(dir.path().join("file"), "").unwrap();
        assert!(Store::at(&dir.path().join("file"), "stop").prune().is_err());
    }
}
