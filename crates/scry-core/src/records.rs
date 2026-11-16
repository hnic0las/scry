//! What Scry keeps about a review in the repository's git folder: who
//! resolved each conflict outside Scry and why (written by the MCP
//! server's `resolve`), and which resolutions the user reviewed (written
//! by the app).
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
