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

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::git::{Git, GitError};
use crate::resolution::CandidateKind;
use crate::session::write_atomically;

/// An agent's name as the user reads it: its MCP client's name, without
/// the version that follows it ("claude-code 2.1.331" is "Claude Code").
/// The version stays in the records, for whoever reads the files.
pub fn agent_name(by: &str) -> String {
    let name = match by.trim().rsplit_once(' ') {
        Some((name, version)) if version.starts_with(|c: char| c.is_ascii_digit()) => name,
        _ => by.trim(),
    };
    match name {
        "claude-code" => String::from("Claude Code"),
        name => name.to_string(),
    }
}

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
    /// A resolution to propose, which the user accepts or not in Scry.
    Resolve,
}

impl AskKind {
    /// The word the asks are printed with, as serialized.
    pub fn word(self) -> &'static str {
        match self {
            AskKind::Explain => "explain",
            AskKind::Resolve => "resolve",
        }
    }

    /// Where an ask or answer of this kind about `conflict` is kept in its
    /// file: an Explain under the conflict's id alone, as before Resolve
    /// existed, so earlier files still read.
    fn key(self, conflict: &str) -> String {
        match self {
            AskKind::Explain => conflict.to_string(),
            AskKind::Resolve => format!("resolve:{conflict}"),
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
    /// The text the user wrote by hand for the conflict when asking, if
    /// any: an answer is about it, and stale once another is written.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edited: Option<String>,
}

/// An agent's answer to an ask, written by the MCP server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Answer {
    /// The conflict's id.
    pub conflict: String,
    pub kind: AskKind,
    /// The agent's name and version, as its MCP client gave them.
    pub by: String,
    /// The explanation, or with `failed` why there is none.
    pub text: String,
    /// Milliseconds since 1970.
    pub at: u64,
    /// The text written by hand the ask was made with, copied from it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edited: Option<String>,
    /// The agent could not answer: `text` says why.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub failed: bool,
    /// For a Resolve, the resolution proposed; `text` is then its reason.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proposal: Option<Proposal>,
}

/// A resolution an agent proposes for a conflict, written to no file until
/// the user accepts it in Scry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Proposal {
    /// The candidate proposed; `None` for text of the agent's own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pick: Option<CandidateKind>,
    /// The resolution's text: the candidate's, or the agent's own.
    pub text: String,
    /// What of either side the text leaves out, as the agent said.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dropped: Option<String>,
}

/// A conflict the user resolved in Scry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Chosen {
    /// The candidate chosen; `None` for text written by hand.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub candidate: Option<CandidateKind>,
    /// The resolution's text: a file holding other text since was changed
    /// outside Scry.
    pub text: String,
}

/// An ask still waiting for its answer, with the file it is about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenAsk {
    /// The file, from the repository root.
    pub path: String,
    pub ask: Ask,
}

/// How far Explain is on one file's conflicts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Explanations {
    /// Conflicts with an explanation (not a failure) and no newer ask.
    pub explained: usize,
    /// Conflicts asked about and not answered since.
    pub waiting: usize,
    /// Conflicts whose resolution the user asked for, not proposed since.
    pub suggesting: usize,
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

    /// What the user chose in Scry for the conflicts of `path`, by
    /// conflict id: what opening the file again gives back.
    pub fn chosen(&self, path: &str) -> BTreeMap<String, Chosen> {
        read(&self.file("chosen", path)).unwrap_or_default()
    }

    /// Replaces what the user chose for the conflicts of `path`.
    pub fn set_chosen(&self, path: &str, chosen: &BTreeMap<String, Chosen>) -> io::Result<()> {
        self.write(&self.file("chosen", path), chosen)
    }

    /// What the user asked about the conflicts of `path`, of `kind`, by
    /// conflict id: the last ask of each conflict.
    pub fn asks(&self, path: &str, kind: AskKind) -> BTreeMap<String, Ask> {
        of_kind(self.all_asks(path), kind, |ask| (ask.kind, &ask.conflict))
    }

    /// Asks about a conflict of `path`, replacing an earlier ask of the
    /// same kind about it.
    pub fn ask(&self, path: &str, ask: Ask) -> io::Result<()> {
        let mut asks = self.all_asks(path);
        asks.insert(ask.kind.key(&ask.conflict), ask);
        self.write(&self.file("asks", path), &asks)
    }

    /// Takes back the ask of `kind` about a conflict of `path`, answered
    /// or not.
    pub fn cancel_ask(&self, path: &str, kind: AskKind, conflict: &str) -> io::Result<()> {
        let mut asks = self.all_asks(path);
        if asks.remove(&kind.key(conflict)).is_some() {
            self.write(&self.file("asks", path), &asks)?;
        }
        Ok(())
    }

    /// The agents' answers of `kind` about the conflicts of `path`, by
    /// conflict id.
    pub fn answers(&self, path: &str, kind: AskKind) -> BTreeMap<String, Answer> {
        of_kind(self.all_answers(path), kind, |answer| {
            (answer.kind, &answer.conflict)
        })
    }

    /// Keeps an agent's answer about a conflict of `path`, replacing an
    /// earlier one of the same kind: an agent correcting itself leaves one
    /// answer.
    pub fn answer(&self, path: &str, answer: Answer) -> io::Result<()> {
        let mut answers = self.all_answers(path);
        answers.insert(answer.kind.key(&answer.conflict), answer);
        self.write(&self.file("answers", path), &answers)
    }

    /// Forgets the answer of `kind` about a conflict of `path`: a proposal
    /// the user rejected.
    pub fn forget_answer(&self, path: &str, kind: AskKind, conflict: &str) -> io::Result<()> {
        let mut answers = self.all_answers(path);
        if answers.remove(&kind.key(conflict)).is_some() {
            self.write(&self.file("answers", path), &answers)?;
        }
        Ok(())
    }

    /// Whether the ask of `kind` about `conflict` of `path` waits for its
    /// answer.
    pub fn is_open(&self, path: &str, kind: AskKind, conflict: &str) -> bool {
        self.asks(path, kind)
            .get(conflict)
            .is_some_and(|ask| open(ask, self.answers(path, kind).get(conflict)))
    }

    /// Every ask of `path`, of every kind, as its file keeps them.
    fn all_asks(&self, path: &str) -> BTreeMap<String, Ask> {
        read(&self.file("asks", path)).unwrap_or_default()
    }

    /// Every answer about `path`, of every kind, as its file keeps them.
    fn all_answers(&self, path: &str) -> BTreeMap<String, Answer> {
        read(&self.file("answers", path)).unwrap_or_default()
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
                let answers = self.all_answers(&path);
                self.all_asks(&path)
                    .into_values()
                    .filter(|ask| open(ask, answers.get(&ask.kind.key(&ask.conflict))))
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

    /// How many conflicts of each file were explained and how many wait
    /// for their explanation, for the overview; files with neither are
    /// left out. A conflict asked again waits: its last answer is not
    /// counted.
    pub fn explanations(&self) -> BTreeMap<String, Explanations> {
        let Ok(entries) = fs::read_dir(&self.folder) else {
            return BTreeMap::new();
        };
        let paths: BTreeSet<String> = entries
            .filter_map(|entry| {
                let name = entry.ok()?.file_name().into_string().ok()?;
                let name = name.strip_suffix(".json")?;
                decoded(
                    name.strip_prefix("asks-")
                        .or_else(|| name.strip_prefix("answers-"))?,
                )
            })
            .collect();
        paths
            .into_iter()
            .filter_map(|path| {
                let asks = self.asks(&path, AskKind::Explain);
                let answers = self.answers(&path, AskKind::Explain);
                let waiting = asks
                    .values()
                    .filter(|ask| open(ask, answers.get(&ask.conflict)))
                    .count();
                let explained = answers
                    .values()
                    .filter(|answer| {
                        !answer.failed
                            && !asks
                                .get(&answer.conflict)
                                .is_some_and(|ask| open(ask, Some(answer)))
                    })
                    .count();
                let proposals = self.answers(&path, AskKind::Resolve);
                let suggesting = self
                    .asks(&path, AskKind::Resolve)
                    .values()
                    .filter(|ask| open(ask, proposals.get(&ask.conflict)))
                    .count();
                let counts = Explanations {
                    explained,
                    waiting,
                    suggesting,
                };
                (explained + waiting + suggesting > 0).then_some((path, counts))
            })
            .collect()
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

/// The entries of `kind` of a file of asks or answers, by conflict id.
fn of_kind<T>(
    entries: BTreeMap<String, T>,
    kind: AskKind,
    key: impl Fn(&T) -> (AskKind, &String),
) -> BTreeMap<String, T> {
    entries
        .into_values()
        .filter_map(|entry| {
            let (its_kind, conflict) = key(&entry);
            let conflict = conflict.clone();
            (its_kind == kind).then_some((conflict, entry))
        })
        .collect()
}

fn read<T: for<'de> Deserialize<'de>>(file: &Path) -> Option<T> {
    serde_json::from_slice(&fs::read(file).ok()?).ok()
}

/// An ask is open until an answer of its kind comes at or after it.
fn open(ask: &Ask, answer: Option<&Answer>) -> bool {
    !answer.is_some_and(|answer| answer.kind == ask.kind && answer.at >= ask.at)
}

/// The path [`encoded`] gave, `None` for a name it cannot have given.
pub(crate) fn decoded(name: &str) -> Option<String> {
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
pub(crate) fn encoded(path: &str) -> String {
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
            edited: None,
        }
    }

    fn answer(conflict: &str, text: &str, at: u64) -> Answer {
        Answer {
            conflict: conflict.to_string(),
            kind: AskKind::Explain,
            by: String::from("claude-code 2.1"),
            text: text.to_string(),
            at,
            edited: None,
            failed: false,
            proposal: None,
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
        assert!(store.is_open("src/a b.swift", AskKind::Explain, "c1"));
        assert!(!store.is_open("src/a b.swift", AskKind::Explain, "c9"));
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
        assert!(!store.is_open("src/a b.swift", AskKind::Explain, "c1"));
        assert_eq!(store.open_asks().len(), 1);
        store
            .answer("src/a b.swift", answer("c1", "Second.", 2_500))
            .unwrap();
        assert_eq!(
            store.answers("src/a b.swift", AskKind::Explain)["c1"].text,
            "Second."
        );
        assert_eq!(store.answers("src/a b.swift", AskKind::Explain).len(), 1);

        store.ask("src/a b.swift", ask("c1", 3_000)).unwrap();
        assert!(store.is_open("src/a b.swift", AskKind::Explain, "c1"));
        store
            .cancel_ask("src/a b.swift", AskKind::Explain, "c1")
            .unwrap();
        assert!(!store.is_open("src/a b.swift", AskKind::Explain, "c1"));
        store
            .cancel_ask("src/a b.swift", AskKind::Explain, "c1")
            .unwrap();
        assert_eq!(store.open_asks().len(), 1);
    }

    /// An Explain and a Resolve on the same conflict are kept apart: each
    /// opens, closes and is taken back on its own, and every open one is
    /// found; a Resolve's answer keeps its proposal.
    #[test]
    fn explain_and_resolve_asks_are_kept_apart() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::at(dir.path(), "stop");
        let resolve = |at| Ask {
            kind: AskKind::Resolve,
            ..ask("c1", at)
        };
        store.ask("a.txt", ask("c1", 1_000)).unwrap();
        store.ask("a.txt", resolve(1_000)).unwrap();
        assert_eq!(store.asks("a.txt", AskKind::Resolve).len(), 1);
        assert_eq!(store.open_asks().len(), 2);

        let proposal = Proposal {
            pick: Some(CandidateKind::Theirs),
            text: String::from("theirs\n"),
            dropped: None,
        };
        store
            .answer(
                "a.txt",
                Answer {
                    kind: AskKind::Resolve,
                    proposal: Some(proposal.clone()),
                    ..answer("c1", "Theirs keeps the rename.", 1_500)
                },
            )
            .unwrap();
        assert!(!store.is_open("a.txt", AskKind::Resolve, "c1"));
        assert!(store.is_open("a.txt", AskKind::Explain, "c1"));
        assert!(store.answers("a.txt", AskKind::Explain).is_empty());
        assert_eq!(
            store.answers("a.txt", AskKind::Resolve)["c1"].proposal,
            Some(proposal)
        );
        let open = store.open_asks();
        assert_eq!(open.len(), 1);
        assert_eq!(open[0].ask.kind, AskKind::Explain);

        store.cancel_ask("a.txt", AskKind::Resolve, "c1").unwrap();
        assert!(store.asks("a.txt", AskKind::Resolve).is_empty());
        assert_eq!(store.asks("a.txt", AskKind::Explain).len(), 1);
        assert_eq!(
            store.explanations()["a.txt"],
            Explanations {
                explained: 0,
                waiting: 1,
                suggesting: 0
            }
        );
        store.ask("a.txt", resolve(2_000)).unwrap();
        assert_eq!(store.explanations()["a.txt"].suggesting, 1);
    }

    /// The overview counts, per file, the conflicts explained and those
    /// waiting; a conflict asked again waits and its last answer is not
    /// counted; a file with an answer alone is listed, one with nothing
    /// left (asks taken back) is not.
    #[test]
    fn explanations_count_per_file() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::at(dir.path(), "stop");
        assert!(store.explanations().is_empty());
        store.ask("a b.txt", ask("c1", 1_000)).unwrap();
        store.ask("a b.txt", ask("c2", 1_000)).unwrap();
        store
            .answer("a b.txt", answer("c1", "One.", 1_500))
            .unwrap();
        store
            .answer("a b.txt", answer("c2", "Two.", 1_500))
            .unwrap();
        store.ask("a b.txt", ask("c2", 2_000)).unwrap();
        store.ask("a b.txt", ask("c3", 2_000)).unwrap();
        store
            .answer("only.txt", answer("c1", "One.", 1_000))
            .unwrap();
        store
            .answer(
                "only.txt",
                Answer {
                    failed: true,
                    ..answer("c2", "The conflict is gone.", 1_000)
                },
            )
            .unwrap();
        store.ask("gone.txt", ask("c1", 1_000)).unwrap();
        store
            .cancel_ask("gone.txt", AskKind::Explain, "c1")
            .unwrap();
        fs::write(store.folder().join("records-x.txt.json"), "{}").unwrap();
        fs::write(store.folder().join("asks-%zz.json"), "{}").unwrap();
        assert_eq!(
            store.explanations(),
            BTreeMap::from([
                (
                    String::from("a b.txt"),
                    Explanations {
                        explained: 1,
                        waiting: 2,
                        suggesting: 0
                    }
                ),
                (
                    String::from("only.txt"),
                    Explanations {
                        explained: 1,
                        waiting: 0,
                        suggesting: 0
                    }
                ),
            ])
        );
        assert!(Store::at(dir.path(), "other").explanations().is_empty());
    }

    /// Asks and answers written before `edited` and `failed` read as without
    /// them, and those fields are written only when set.
    #[test]
    fn asks_and_answers_read_without_the_newer_fields() {
        let ask: Ask =
            serde_json::from_str(r#"{"conflict":"c1","kind":"explain","at":1}"#).unwrap();
        assert_eq!(ask.edited, None);
        assert!(!serde_json::to_string(&ask).unwrap().contains("edited"));
        let answer: Answer = serde_json::from_str(
            r#"{"conflict":"c1","kind":"explain","by":"a","text":"t","at":1}"#,
        )
        .unwrap();
        assert!(!answer.failed && answer.edited.is_none() && answer.proposal.is_none());
        let written = serde_json::to_string(&answer).unwrap();
        assert!(
            !written.contains("failed")
                && !written.contains("edited")
                && !written.contains("proposal")
        );
        let failed = Answer {
            failed: true,
            edited: Some(String::from("x")),
            ..answer
        };
        let written = serde_json::to_string(&failed).unwrap();
        assert!(written.contains(r#""failed":true"#) && written.contains(r#""edited":"x""#));
    }

    /// The version goes, and Claude Code's client gets its product name;
    /// other names stay as they are.
    #[test]
    fn agent_names_lose_their_version() {
        assert_eq!(agent_name("claude-code 2.1.331"), "Claude Code");
        assert_eq!(agent_name("claude-code"), "Claude Code");
        assert_eq!(agent_name("cursor-vscode 1.0"), "cursor-vscode");
        assert_eq!(agent_name("My Agent v2"), "My Agent v2");
        assert_eq!(agent_name(" codex 0.9 "), "codex");
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
