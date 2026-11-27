//! Asks waiting for an agent, watched from outside the app: `git scry
//! wait` prints each new ask of the repository, and Scry's Claude Code
//! plugin runs it as a monitor, so the user's session answers it (vault
//! spec "Explain through a watching agent").
//!
//! The asks and answers themselves are kept with the review
//! ([`crate::records::Store`]) for conflicts and, for lines selected in
//! the Changes window, outside any operation
//! ([`crate::line_asks::LineStore`]); here is what watching them needs: reading
//! the stop only when they changed, telling a new ask from one already
//! printed, the line printed, and the presence files that tell the app a
//! watcher is there.

use std::collections::BTreeSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use crate::git::{Git, GitError};
use crate::line_asks::LineStore;
use crate::records::{Ask, AskKind, OpenAsk, Store};

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
    /// The asks about lines of the Changes window.
    lines: LineStore,
    seen: Option<Signature>,
    /// The asks already printed, by stop folder, file, kind, conflict and
    /// time: the same ask is printed once, an ask made again is printed
    /// again.
    printed: BTreeSet<(PathBuf, String, &'static str, String, u64)>,
}

impl Watch {
    /// Watches the asks of the repository whose git folder is `git_dir`.
    pub fn new(git_dir: &Path) -> Watch {
        Watch {
            reviews: git_dir.join("scry").join("review"),
            lines: LineStore::new(git_dir),
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
        // Whether asks were made about lines (`true`) or about conflicts.
        let any_ask = |of_lines: bool| {
            signature.iter().any(|(file, _, _)| {
                (file.parent() == Some(self.lines.folder())) == of_lines
                    && file
                        .file_name()
                        .and_then(|name| name.to_str())
                        .is_some_and(|name| name.starts_with("asks-"))
            })
        };
        let mut open = Vec::new();
        // The stop is read (three `git` runs) only when a conflict was
        // asked about.
        if any_ask(false) {
            let store = Store::open(git)?;
            let folder = store.folder().to_path_buf();
            open.extend(
                store
                    .open_asks()
                    .into_iter()
                    .map(|open| (folder.clone(), open)),
            );
        }
        if any_ask(true) {
            // Lines of the Changes window: an Explain like the others,
            // told apart by its id.
            let folder = self.lines.folder().to_path_buf();
            open.extend(self.lines.open_asks().into_iter().map(|(path, ask)| {
                let ask = Ask {
                    conflict: ask.id,
                    kind: AskKind::Explain,
                    at: ask.at,
                    edited: None,
                };
                (folder.clone(), OpenAsk { path, ask })
            }));
        }
        self.seen = Some(signature);
        open.sort_by(|a, b| (a.1.ask.at, &a.1.path).cmp(&(b.1.ask.at, &b.1.path)));
        Ok(open
            .into_iter()
            .filter(|(folder, open)| {
                self.printed.insert((
                    folder.clone(),
                    open.path.clone(),
                    open.ask.kind.word(),
                    open.ask.conflict.clone(),
                    open.ask.at,
                ))
            })
            .map(|(_, open)| open)
            .collect())
    }

    /// Every asks and answers file of every stop and of the lines, with
    /// its modification time and size.
    fn signature(&self) -> Signature {
        let stops = fs::read_dir(&self.reviews)
            .into_iter()
            .flatten()
            .filter_map(|stop| Some(stop.ok()?.path()));
        stops
            .chain([self.lines.folder().to_path_buf()])
            .filter_map(|folder| fs::read_dir(folder).ok())
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

/// How long the file of a watcher that stopped beating is kept when its
/// lock cannot be tried; a file still locked is never removed, a watcher
/// waking from sleep being still there.
pub const FORGOTTEN_AFTER: Duration = Duration::from_secs(3600);

/// A watcher's presence file, `<git dir>/scry/watchers/<id>`, which it
/// holds locked while it runs: the system releases the lock when the
/// process ends, even killed with its session, so a watcher gone is seen
/// at once rather than after [`GONE_AFTER`]. Removed when dropped.
pub struct Presence {
    file: fs::File,
    path: PathBuf,
}

impl Presence {
    /// Says the watcher `id` (its process id) is there, now.
    pub fn start(git_dir: &Path, id: &str) -> io::Result<Presence> {
        let folder = watchers(git_dir);
        fs::create_dir_all(&folder)?;
        let path = folder.join(id);
        // Truncated, so the file of an earlier process with the same id
        // reads as new until it is locked.
        let mut file = fs::File::options()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&path)?;
        file.lock()?;
        // When it started, in seconds since 1970, for Settings › Agent.
        let started = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map_or(0, |since| since.as_secs());
        io::Write::write_all(&mut file, started.to_string().as_bytes())?;
        let presence = Presence { file, path };
        presence.beat()?;
        Ok(presence)
    }

    /// Says the watcher is still there, every [`BEAT`] while it runs.
    /// Removes the files of watchers that ended without removing theirs,
    /// never one a running watcher holds.
    pub fn beat(&self) -> io::Result<()> {
        let now = SystemTime::now();
        self.file.set_modified(now)?;
        let Some(folder) = self.path.parent() else {
            return Ok(());
        };
        for entry in fs::read_dir(folder)?.flatten() {
            let path = entry.path();
            if path == self.path {
                continue;
            }
            let silent = |after: Duration| {
                entry
                    .metadata()
                    .and_then(|metadata| metadata.modified())
                    .is_ok_and(|beat| now.duration_since(beat).is_ok_and(|since| since > after))
            };
            let remove = match held(&path) {
                Some(true) => false,
                Some(false) => silent(GONE_AFTER),
                None => silent(FORGOTTEN_AFTER),
            };
            if remove {
                let _ = fs::remove_file(path);
            }
        }
        Ok(())
    }
}

impl Drop for Presence {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

/// Whether a running watcher holds `path` locked; `None` when the lock
/// cannot be tried, the beat alone then saying.
fn held(path: &Path) -> Option<bool> {
    let file = fs::File::open(path).ok()?;
    match file.try_lock_shared() {
        Ok(()) => Some(false),
        Err(fs::TryLockError::WouldBlock) => Some(true),
        Err(fs::TryLockError::Error(_)) => None,
    }
}

/// Whether a watcher that beat within [`GONE_AFTER`] of `now` still runs.
pub fn watching(git_dir: &Path, now: SystemTime) -> bool {
    !sessions(git_dir, now).is_empty()
}

/// When each watcher still running started, earliest first: one per
/// Claude Code session watching the repository. A file that does not say
/// counts from its last beat.
pub fn sessions(git_dir: &Path, now: SystemTime) -> Vec<SystemTime> {
    let Ok(entries) = fs::read_dir(watchers(git_dir)) else {
        return Vec::new();
    };
    let mut started: Vec<SystemTime> = entries
        .flatten()
        .filter_map(|entry| {
            let beat = entry
                .metadata()
                .and_then(|metadata| metadata.modified())
                .ok()?;
            let recent = now
                .duration_since(beat)
                .map_or(true, |since| since <= GONE_AFTER);
            if !recent || held(&entry.path()) == Some(false) {
                return None;
            }
            let written = fs::read_to_string(entry.path()).ok();
            Some(
                written
                    .and_then(|text| text.trim().parse::<u64>().ok())
                    .map_or(beat, |seconds| {
                        SystemTime::UNIX_EPOCH + Duration::from_secs(seconds)
                    }),
            )
        })
        .collect();
    started.sort();
    started
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open(path: &str, conflict: &str) -> OpenAsk {
        OpenAsk {
            path: path.to_string(),
            ask: Ask {
                conflict: conflict.to_string(),
                kind: AskKind::Explain,
                at: 1,
                edited: None,
            },
        }
    }

    /// An Explain and a Resolve asked at once about one conflict are two
    /// lines, each naming its kind.
    #[test]
    fn each_kind_of_ask_is_printed() {
        let repo = tempfile::tempdir().unwrap();
        let git = crate::git::GitCli::new(repo.path().to_path_buf());
        git.run(&["init", "--quiet"]).unwrap();
        let store = Store::open(&git).unwrap();
        for kind in [AskKind::Explain, AskKind::Resolve] {
            let ask = Ask {
                kind,
                ..open("a.txt", "c1").ask
            };
            store.ask("a.txt", ask).unwrap();
        }
        let mut watch = Watch::new(&repo.path().join(".git"));
        let lines: Vec<String> = watch.poll(&git).unwrap().iter().map(line).collect();
        assert_eq!(
            lines,
            ["Scry asks: explain a.txt c1", "Scry asks: resolve a.txt c1"]
        );
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

    /// A watcher counts while it runs and beat within three beats; none,
    /// or only old ones, is nobody watching; one that left, or whose
    /// process ended without removing its file, is gone at once.
    #[test]
    fn a_watcher_counts_while_it_runs_and_beats() {
        let dir = tempfile::tempdir().unwrap();
        let now = SystemTime::now();
        assert!(!watching(dir.path(), now));
        let presence = Presence::start(dir.path(), "42").unwrap();
        assert!(watching(dir.path(), now));
        assert!(watching(
            dir.path(),
            now + GONE_AFTER - Duration::from_secs(1)
        ));
        assert!(!watching(
            dir.path(),
            now + GONE_AFTER + Duration::from_secs(2)
        ));
        let second = Presence::start(dir.path(), "44").unwrap();
        let started = sessions(dir.path(), now);
        assert_eq!(started.len(), 2);
        let seconds = |time: SystemTime| {
            time.duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_secs()
        };
        assert!(seconds(started[0]).abs_diff(seconds(now)) <= 1);
        // A file that does not say when it started counts from its beat.
        fs::write(watchers(dir.path()).join("44"), "").unwrap();
        let earlier = SystemTime::now() - Duration::from_secs(20);
        fs::File::options()
            .write(true)
            .open(watchers(dir.path()).join("44"))
            .unwrap()
            .set_modified(earlier)
            .unwrap();
        assert_eq!(sessions(dir.path(), now)[0], earlier);
        drop(second);
        drop(presence);
        assert!(!watching(dir.path(), now));
        assert!(sessions(dir.path(), now).is_empty());

        // Ended without removing its file: the lock went with it.
        fs::write(watchers(dir.path()).join("43"), "").unwrap();
        assert!(!watching(dir.path(), now));
    }

    /// A beat removes the file of a watcher that ended without removing it
    /// once it is silent; it keeps those that beat recently, and those that
    /// run even silent for an hour (the Mac slept).
    #[test]
    fn a_beat_forgets_watchers_gone() {
        let dir = tempfile::tempdir().unwrap();
        let folder = watchers(dir.path());
        let running = Presence::start(dir.path(), "1").unwrap();
        let old_running = Presence::start(dir.path(), "2").unwrap();
        fs::write(folder.join("3"), "").unwrap();
        fs::write(folder.join("4"), "").unwrap();
        fs::write(folder.join("5"), "").unwrap();
        let set = |id: &str, age: Duration| {
            fs::File::options()
                .write(true)
                .open(folder.join(id))
                .unwrap()
                .set_modified(SystemTime::now() - age)
                .unwrap();
        };
        set("2", FORGOTTEN_AFTER + Duration::from_secs(60));
        set("3", GONE_AFTER + Duration::from_secs(5));
        set("4", GONE_AFTER - Duration::from_secs(5));
        set("5", FORGOTTEN_AFTER - Duration::from_secs(60));
        running.beat().unwrap();
        assert!(folder.join("1").exists());
        assert!(folder.join("2").exists());
        assert!(!folder.join("3").exists());
        assert!(folder.join("4").exists());
        assert!(!folder.join("5").exists());
        drop(old_running);
        drop(running);
        assert!(!folder.join("1").exists() && !folder.join("2").exists());
    }

    /// Lines asked about in the Changes window are printed as an Explain,
    /// with no operation in progress, once.
    #[test]
    fn asks_about_lines_are_printed_outside_an_operation() {
        let repo = tempfile::tempdir().unwrap();
        let git = crate::git::GitCli::new(repo.path().to_path_buf());
        git.run(&["init", "--quiet"]).unwrap();
        let git_dir = repo.path().join(".git");
        let mut watch = Watch::new(&git_dir);
        assert!(watch.poll(&git).unwrap().is_empty());
        let ask = crate::line_asks::LineAsk {
            id: String::from("L0000abcd"),
            first: 1,
            last: 2,
            lines: vec![String::from("a"), String::from("b")],
            what: String::new(),
            excerpt: String::new(),
            target: crate::file_blame::Target::WorkingCopy,
            at: 5,
        };
        LineStore::new(&git_dir)
            .ask("src/a b.rs", ask, &[])
            .unwrap();
        let lines: Vec<String> = watch.poll(&git).unwrap().iter().map(line).collect();
        assert_eq!(lines, ["Scry asks: explain \"src/a b.rs\" L0000abcd"]);
        assert!(watch.poll(&git).unwrap().is_empty());
    }
}
