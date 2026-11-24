//! Asks waiting for an agent, watched from outside the app: `git scry
//! wait` prints each new ask of the repository, and Scry's Claude Code
//! plugin runs it as a monitor, so the user's session answers it (vault
//! spec "Explain through a watching agent").
//!
//! The asks and answers themselves are kept with the review
//! ([`crate::records::Store`]); here is what watching them needs: reading
//! the stop only when they changed, telling a new ask from one already
//! printed, the line printed, and the presence files that tell the app a
//! watcher is there.

use std::collections::BTreeSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use crate::git::{Git, GitError};
use crate::records::{OpenAsk, Store};

/// How often a watcher says it is still there.
pub const BEAT: Duration = Duration::from_secs(10);
/// How long after its last beat a watcher still counts: three beats
/// missed, it is gone (quit, killed, its session closed).
pub const GONE_AFTER: Duration = Duration::from_secs(30);

/// The repository's git folder (each linked worktree has its own).
pub fn git_dir(git: &dyn Git) -> Result<PathBuf, GitError> {
    let dir = git.run(&["rev-parse", "--absolute-git-dir"])?;
    Ok(PathBuf::from(dir.trim_end()))
}

/// The line `git scry wait` prints for an ask, which the plugin's agent
/// reads: `Scry asks: explain src/Large.swift c12`, the path quoted as a
/// JSON string when it holds a space, a quote or a control character.
pub fn line(open: &OpenAsk) -> String {
    let plain = !open.path.is_empty()
        && !open.path.chars().any(|character| {
            character.is_whitespace() || character.is_control() || "\"'\\".contains(character)
        });
    let path = match plain {
        true => open.path.clone(),
        false => serde_json::to_string(&open.path).unwrap_or_default(),
    };
    format!(
        "Scry asks: {} {} {}",
        open.ask.kind.word(),
        path,
        open.ask.conflict
    )
}

/// What a watcher saw of the asks and answers files last time, so it reads
/// the stop (three `git` runs) only when one of them changed.
type Signature = BTreeSet<(PathBuf, Option<SystemTime>, u64)>;

/// One watcher's view of a repository's asks.
pub struct Watch {
    reviews: PathBuf,
    seen: Option<Signature>,
    /// The asks already printed, by stop folder, file, conflict and time:
    /// the same ask is printed once, an ask made again is printed again.
    printed: BTreeSet<(PathBuf, String, String, u64)>,
}

impl Watch {
    /// Watches the asks of the repository whose git folder is `git_dir`.
    pub fn new(git_dir: &Path) -> Watch {
        Watch {
            reviews: git_dir.join("scry").join("review"),
            seen: None,
            printed: BTreeSet::new(),
        }
    }

    /// The asks of the stop `git` is at that this watch has not returned
    /// yet, oldest first; the first call returns the ones already waiting.
    pub fn poll(&mut self, git: &dyn Git) -> Result<Vec<OpenAsk>, GitError> {
        let signature = self.signature();
        if self.seen.as_ref() == Some(&signature) {
            return Ok(Vec::new());
        }
        let any_ask = signature.iter().any(|(file, _, _)| {
            file.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("asks-"))
        });
        if !any_ask {
            self.seen = Some(signature);
            return Ok(Vec::new());
        }
        let store = Store::open(git)?;
        self.seen = Some(signature);
        let folder = store.folder().to_path_buf();
        Ok(store
            .open_asks()
            .into_iter()
            .filter(|open| {
                self.printed.insert((
                    folder.clone(),
                    open.path.clone(),
                    open.ask.conflict.clone(),
                    open.ask.at,
                ))
            })
            .collect())
    }

    /// Every asks and answers file of every stop, with its modification
    /// time and size.
    fn signature(&self) -> Signature {
        let Ok(stops) = fs::read_dir(&self.reviews) else {
            return Signature::new();
        };
        stops
            .filter_map(|stop| fs::read_dir(stop.ok()?.path()).ok())
            .flatten()
            .filter_map(|entry| {
                let entry = entry.ok()?;
                let name = entry.file_name().into_string().ok()?;
                if !(name.starts_with("asks-") || name.starts_with("answers-")) {
                    return None;
                }
                let metadata = entry.metadata().ok()?;
                Some((entry.path(), metadata.modified().ok(), metadata.len()))
            })
            .collect()
    }
}

/// The folder of the presence files, one per watcher.
fn watchers(git_dir: &Path) -> PathBuf {
    git_dir.join("scry").join("watchers")
}

/// Says the watcher `id` (its process id) is there, now: written when it
/// starts and every [`BEAT`] while it runs.
pub fn beat(git_dir: &Path, id: &str) -> io::Result<()> {
    let folder = watchers(git_dir);
    fs::create_dir_all(&folder)?;
    fs::write(folder.join(id), "")
}

/// Says the watcher `id` left.
pub fn leave(git_dir: &Path, id: &str) {
    let _ = fs::remove_file(watchers(git_dir).join(id));
}

/// Whether a watcher beat within [`GONE_AFTER`] of `now`.
pub fn watching(git_dir: &Path, now: SystemTime) -> bool {
    let Ok(entries) = fs::read_dir(watchers(git_dir)) else {
        return false;
    };
    entries
        .filter_map(|entry| entry.ok()?.metadata().ok()?.modified().ok())
        .any(|beat| {
            now.duration_since(beat)
                .map_or(true, |since| since <= GONE_AFTER)
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::records::{Ask, AskKind};

    fn open(path: &str, conflict: &str) -> OpenAsk {
        OpenAsk {
            path: path.to_string(),
            ask: Ask {
                conflict: conflict.to_string(),
                kind: AskKind::Explain,
                at: 1,
            },
        }
    }

    /// The line names the kind, the path and the conflict; a path the
    /// agent could split is quoted.
    #[test]
    fn the_line_quotes_a_path_only_when_needed() {
        assert_eq!(
            line(&open("src/Large.swift", "c12")),
            "Scry asks: explain src/Large.swift c12"
        );
        assert_eq!(
            line(&open("My Files/a \"b\".txt", "c1")),
            r#"Scry asks: explain "My Files/a \"b\".txt" c1"#
        );
        assert_eq!(line(&open("", "c1")), r#"Scry asks: explain "" c1"#);
        // Each of these alone is quoted.
        assert_eq!(line(&open("a b", "c1")), r#"Scry asks: explain "a b" c1"#);
        assert_eq!(
            line(&open("a\u{1}b", "c1")),
            r#"Scry asks: explain "a\u0001b" c1"#
        );
        assert_eq!(line(&open("it's", "c1")), r#"Scry asks: explain "it's" c1"#);
        assert_eq!(line(&open("a\\b", "c1")), r#"Scry asks: explain "a\\b" c1"#);
    }

    /// A watcher counts until three beats are missed; none, or only old
    /// ones, is nobody watching; one that left is gone at once.
    #[test]
    fn a_watcher_counts_while_it_beats() {
        let dir = tempfile::tempdir().unwrap();
        let now = SystemTime::now();
        assert!(!watching(dir.path(), now));
        beat(dir.path(), "42").unwrap();
        assert!(watching(dir.path(), now));
        assert!(watching(
            dir.path(),
            now + GONE_AFTER - Duration::from_secs(1)
        ));
        assert!(!watching(
            dir.path(),
            now + GONE_AFTER + Duration::from_secs(2)
        ));
        leave(dir.path(), "42");
        assert!(!watching(dir.path(), now));
        leave(dir.path(), "42");
    }
}
