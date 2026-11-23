//! The tools, as plain functions from their arguments to text: compact
//! answers for an agent, counts before lists, and a hint at what to call
//! next. An answer the agent should correct its call from (a path that is
//! not a conflicted file, a stale id) is an `Err`, still a sentence.
//!
//! Only `resolve` writes: one conflict's region of a conflicted text file,
//! and its record under the git folder. Nothing touches the index.

use std::collections::HashMap;
use std::path::PathBuf;

use scry_core::blame::{self, LineCommit};
use scry_core::git::GitCli;
use scry_core::operation::Side;
use scry_core::overview::{self, FileView, Overview, Snapshot};
use scry_core::presentation;
use scry_core::records::{Record, Store};
use scry_core::resolution::CandidateKind;
use scry_core::resolution::{Analysis, Candidate};
use scry_core::review::{self, Fill, Resolver, WriteError};
use scry_core::session::{Choice, MergeSession};
use scry_core::syntax::ParseCheck;
use scry_core::theme::State;

/// Lines of context `conflict` shows unless asked otherwise.
pub const DEFAULT_CONTEXT: usize = 3;
/// The most lines of context `conflict` shows.
pub const MAX_CONTEXT: usize = 100;
/// A reason's length, in characters: enough to name what each side keeps,
/// short enough to read in the inspector.
pub const REASON_LENGTH: std::ops::RangeInclusive<usize> = 20..=600;
/// The most `dropped` says, in characters.
pub const MAX_DROPPED: usize = 600;
/// The largest text `resolve` writes for one conflict, in characters, as
/// the schema's `maxLength` counts them.
pub const MAX_TEXT: usize = 100_000;

/// What resolves a conflict, as the agent asks for it.
pub enum Resolve {
    /// A candidate Scry offers for it.
    Pick(CandidateKind),
    /// Text of its own; empty removes the conflict's lines.
    Text(String),
}

/// A conflict's resolution, as the agent asks for it.
pub struct Resolution {
    pub resolve: Resolve,
    /// Replace a resolution that is not the agent's own.
    pub replace: bool,
    pub reason: String,
    /// What own text leaves out of either side.
    pub dropped: Option<String>,
}

/// The tools, with git run from the folder the server was started in.
pub struct Tools {
    git: GitCli,
    /// Kept across calls, as the app keeps it: the store of reviews is
    /// pruned only when the operation moves to another step. Made at the
    /// first call inside a repository.
    overview: Option<Overview>,
}

/// The repository as one call sees it.
struct Repo {
    /// git run at the root.
    git: GitCli,
    root: PathBuf,
    snapshot: Snapshot,
}

impl Tools {
    pub fn new(git: GitCli) -> Tools {
        Tools {
            git,
            overview: None,
        }
    }

    /// The operation in progress, its sides and its files.
    pub fn overview(&mut self) -> Result<String, String> {
        let repo = self.repo()?;
        let snapshot = &repo.snapshot;
        let mut text = String::new();
        match &snapshot.operation {
            Some(operation) => {
                text += &format!("{}\n", operation.summary());
                text += &format!("ours: {}\n", side(&operation.ours));
                match &operation.theirs {
                    Some(theirs) => text += &format!("theirs: {}\n", side(theirs)),
                    None => text += "theirs: none, no commit is being applied\n",
                }
            }
            None if snapshot.files.is_empty() => {
                return Ok(String::from(
                    "No merge, rebase, cherry-pick or revert in progress, and no file with conflicts.",
                ));
            }
            None => {
                text += "No merge, rebase, cherry-pick or revert in progress, but files have conflicts (left by git stash pop or git checkout --merge).\n"
            }
        }

        let mut open = Vec::new();
        let mut resolved = Vec::new();
        let mut special = Vec::new();
        let mut staged = Vec::new();
        let mut open_conflicts = 0;
        let mut to_review = 0;
        for file in &snapshot.files {
            match file {
                FileView {
                    state: State::Staged,
                    ..
                } => staged.push(file.path.clone()),
                FileView {
                    special: Some(_), ..
                } => special.push(format!("  {}  {}", file.path, file.kind)),
                FileView {
                    state: State::Unresolved,
                    openable: true,
                    conflicts,
                    ..
                } => {
                    let count = conflicts.unwrap_or(0);
                    open_conflicts += count;
                    open.push(format!(
                        "  {}  {}",
                        file.path,
                        plural(count, "open conflict")
                    ));
                }
                FileView {
                    state: State::Unresolved,
                    ..
                } => special.push(format!(
                    "  {}  {}, not readable as text",
                    file.path, file.kind
                )),
                FileView {
                    state: State::NeedsReview,
                    review,
                    ..
                } => {
                    to_review += 1;
                    let progress = match review {
                        Some(progress) if progress.changed_outside => {
                            String::from("changed outside its conflicts")
                        }
                        Some(progress) => {
                            format!("{} of {} reviewed", progress.reviewed, progress.total)
                        }
                        None => String::from("not reviewed"),
                    };
                    resolved.push(format!("  {}  to review in Scry: {progress}", file.path));
                }
                FileView {
                    state: State::Resolved,
                    ..
                } => resolved.push(format!("  {}  resolved", file.path)),
            }
        }

        let mut counts = vec![match open.len() {
            0 => String::from("No open conflict"),
            files => format!(
                "{} with {}",
                plural(files, "file"),
                plural(open_conflicts, "open conflict")
            ),
        }];
        if !resolved.is_empty() {
            counts.push(format!(
                "{} resolved, not staged ({to_review} to review)",
                plural(resolved.len(), "file")
            ));
        }
        if !special.is_empty() {
            counts.push(format!(
                "{} to settle in Scry",
                plural(special.len(), "file")
            ));
        }
        if !staged.is_empty() {
            counts.push(format!("{} staged", plural(staged.len(), "file")));
        }
        text += &format!("{}.\n", counts.join(", "));
        section(&mut text, "To resolve", &open);
        section(&mut text, "Resolved, not staged", &resolved);
        section(
            &mut text,
            "Settled in Scry's own views, not with these tools",
            &special,
        );
        if !staged.is_empty() {
            text += &format!("Staged: {}\n", staged.join(", "));
        }
        if !open.is_empty() || !resolved.is_empty() {
            text += "Next: conflicts(path) lists a file's conflicts.";
        }
        Ok(text.trim_end().to_string())
    }

    /// One line per conflict of `path`.
    pub fn conflicts(&mut self, path: &str) -> Result<String, String> {
        let repo = self.repo()?;
        let path = repo.text_file(path)?;
        let opened = repo.open(&path)?;
        let session = &opened.session;
        let count = session.conflict_count();
        if count == 0 {
            return Ok(repo.nothing_to_tell_apart(&path));
        }
        let language = session.language();
        let mut text = format!(
            "{path}{}: {}, {} open.\n",
            language.map_or(String::new(), |language| format!(" ({})", language.name())),
            plural(count, "conflict"),
            session.unresolved()
        );
        for index in 0..count {
            let analysis = session.analysis(index);
            let line = opened
                .line(analysis)
                .map_or(String::new(), |line| format!("  line {line}"));
            let suggestion = analysis.suggestion.map_or(String::new(), |kind| {
                format!(" Suggested by rule: {}.", kind.id())
            });
            text += &format!(
                "{}{line}  {}. {}. Candidates: {}.{suggestion}\n",
                analysis.id,
                state(session.choice(index)),
                analysis.kind.describe(&opened.ours, &opened.theirs),
                analysis
                    .candidates
                    .iter()
                    .map(candidate_name)
                    .collect::<Vec<_>>()
                    .join(", "),
            );
        }
        if language.is_some() {
            text += "✓ the file parses with that candidate, ✗ it does not.\n";
        }
        text += "Next: conflict(path, id) shows a conflict's sides, base, candidates and commits.";
        Ok(text)
    }

    /// One conflict of `path` in full, with `context` lines around it.
    pub fn conflict(&mut self, path: &str, id: &str, context: usize) -> Result<String, String> {
        let repo = self.repo()?;
        let path = repo.text_file(path)?;
        let opened = repo.open(&path)?;
        let session = &opened.session;
        let count = session.conflict_count();
        let Some(index) = index_of(session, id) else {
            return Err(stale(&repo, &path, id, session));
        };
        let analysis = session.analysis(index);
        let conflict = session.conflict(index);
        let commits = match &repo.snapshot.operation {
            Some(operation) => blame::conflict_commits(&repo.git, operation, &path, session)
                .into_iter()
                .nth(index)
                .unwrap_or_default(),
            None => Default::default(),
        };

        let line = opened
            .line(analysis)
            .map_or(String::new(), |line| format!(", line {line}"));
        let mut text = format!(
            "Conflict {id}, {} of {count} in {path}{line}: {}.\n{}.\n",
            index + 1,
            state(session.choice(index)),
            analysis.kind.describe(&opened.ours, &opened.theirs),
        );
        if let Some(kind) = analysis.suggestion {
            text += &format!("Suggested by rule: {}.\n", kind.id());
        }
        match session.choice(index) {
            Some(Choice::Outside(outside)) => {
                if let Some(record) = &outside.record {
                    text += &format!("Reason given by {}: {}\n", record.by, record.reason);
                    if let Some(dropped) = &record.dropped {
                        text += &format!("Left out: {dropped}\n");
                    }
                }
                if outside.matches.is_none() {
                    text += &format!("\nResolved with its own text:\n{}", fenced(&outside.text));
                }
            }
            Some(Choice::Edited(edit)) => {
                text += &format!("\nResolved with its own text:\n{}", fenced(&edit.text))
            }
            Some(Choice::Candidate(_)) | None => {}
        }

        let (before, after) = session.context(index, context);
        if context > 0 {
            text += &format!("\nBefore:\n{}", fenced(&before));
        }
        text += &format!(
            "\nOurs, {}{}:\n{}",
            opened.ours,
            last_change(commits.ours.as_ref()),
            fenced(&conflict.ours.text)
        );
        match &conflict.base {
            Some(base) => text += &format!("\nBase:\n{}", fenced(&base.text)),
            None => {
                text += "\nBase: not known (git wrote no base and Scry could not rebuild it).\n"
            }
        }
        text += &format!(
            "\nTheirs, {}{}:\n{}",
            opened.theirs,
            last_change(commits.theirs.as_ref()),
            fenced(&conflict.theirs.text)
        );
        if context > 0 {
            text += &format!("\nAfter:\n{}", fenced(&after));
        }

        text += "\nCandidates";
        if session.language().is_some() {
            text += " (✓ the file parses with it, ✗ it does not)";
        }
        text += ":\n";
        for candidate in &analysis.candidates {
            text += &format!("- {}", candidate_name(candidate));
            if let ParseCheck::DoesNotParse { line } = candidate.parse {
                text += &format!(", error on line {line}");
            }
            let same_as = [
                ("ours", Some(&conflict.ours.text)),
                ("base", conflict.base.as_ref().map(|base| &base.text)),
                ("theirs", Some(&conflict.theirs.text)),
            ]
            .into_iter()
            .find(|(_, side)| *side == Some(&candidate.text));
            text += &match same_as {
                _ if candidate.text.is_empty() => String::from(": no text, the lines removed\n"),
                Some((name, _)) => format!(": same text as {name}\n"),
                None => format!(":\n{}", fenced(&candidate.text)),
            };
        }
        Ok(text.trim_end().to_string())
    }

    /// Writes conflict `id` of `path` as `resolution` says, records who did
    /// it (`by`) and why, and says whether the file parses with it.
    pub fn resolve(
        &mut self,
        path: &str,
        id: &str,
        resolution: Resolution,
        by: &str,
    ) -> Result<String, String> {
        let repo = self.repo()?;
        let path = repo.text_file(path)?;
        let fill = match resolution.resolve {
            Resolve::Text(text) => Fill::Text(text),
            Resolve::Pick(kind) => repo.picked(&path, id, kind)?,
        };
        let pick = match &fill {
            Fill::Pick { kind, .. } => Some(*kind),
            Fill::Text(_) => None,
        };
        let text = review::write(&repo.git, &repo.root, &path, id, &fill, resolution.replace)
            .map_err(|error| repo.refusal(&path, id, error))?;
        let record = Record {
            conflict: id.to_string(),
            by: by.to_string(),
            pick,
            text,
            reason: resolution.reason,
            dropped: resolution.dropped,
            at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |time| time.as_secs()),
        };
        let recorded = Store::open(&repo.git)
            .map_err(|error| error.to_string())
            .and_then(|store| {
                store
                    .add_record(&path, record)
                    .map_err(|error| error.to_string())
            });

        let what = pick.map_or("its own text", |kind| kind.id());
        let mut answer = format!("Wrote {what} for conflict {id} in {path}.");
        if let Err(error) = recorded {
            answer += &format!(
                " Its reason could not be kept ({error}): the user sees no reason for it."
            );
        }
        answer += &format!("\n{}", repo.read_back(&path, id, pick));
        answer += "\nNothing is staged: the user reviews each resolution in Scry (git scry).";
        Ok(answer)
    }

    /// The repository the server was started in, read again.
    fn repo(&mut self) -> Result<Repo, String> {
        let root = overview::root(&self.git).map_err(|_| {
            String::from(
                "Not in a git repository: the server runs git from the folder it was started in. Start the agent in the repository.",
            )
        })?;
        let git = self.git.in_dir(root.clone());
        // The server's folder never changes, nor does its repository.
        let snapshot = self
            .overview
            .get_or_insert_with(|| Overview::new(root.clone()))
            .refresh(&git)
            .map_err(|error| format!("git could not read the repository: {error}"))?;
        Ok(Repo {
            git,
            root,
            snapshot,
        })
    }
}

/// A conflicted file opened as the app opens it.
struct Opened {
    session: MergeSession,
    /// The line of each conflict in the file as it is, by id.
    lines: HashMap<String, usize>,
    /// The sides as the app names them.
    ours: String,
    theirs: String,
}

impl Opened {
    fn line(&self, analysis: &Analysis) -> Option<usize> {
        self.lines.get(&analysis.id).copied()
    }
}

impl Repo {
    /// `path` (from the root) when it is a text file git left with
    /// conflicts at this step, as the overview lists it; otherwise what to
    /// do instead.
    fn text_file(&self, path: &str) -> Result<String, String> {
        let path = path.strip_prefix("./").unwrap_or(path);
        let Some(file) = self.snapshot.files.iter().find(|file| file.path == path) else {
            let paths: Vec<&str> = self
                .snapshot
                .files
                .iter()
                .filter(|file| text_file(file))
                .map(|file| file.path.as_str())
                .collect();
            return Err(match paths.is_empty() {
                true => format!("{path} is not a file git left with conflicts: none has any."),
                false => format!(
                    "{path} is not a file git left with conflicts. Paths are from the repository root; files with conflicts: {}.",
                    paths.join(", ")
                ),
            });
        };
        if file.state == State::Staged {
            return Err(format!(
                "{path} is staged: git has it. To change it, unstage it in Scry first."
            ));
        }
        if file.special.is_some() || !file.openable {
            return Err(format!(
                "{path} is a conflict of another kind ({}): settle it in Scry, not with these tools.",
                file.kind
            ));
        }
        Ok(path.to_string())
    }

    /// The file's session, as `open` makes it, without the lines of its
    /// conflicts.
    fn session(&self, path: &str) -> Result<MergeSession, String> {
        let (ours, theirs) = presentation::side_labels(self.snapshot.operation.as_ref());
        let file = self.root.join(path);
        let folder = file.parent().unwrap_or(&self.root).to_path_buf();
        review::opened(
            &self.git,
            &self.git.in_dir(folder),
            &self.root,
            path,
            (&ours, &theirs),
        )
        .map_err(|error| format!("{path}: {error}"))
    }

    /// The file opened as the app opens it: from its markers, or, when
    /// conflicts were resolved outside Scry, rebuilt from git's versions
    /// with what the file holds.
    fn open(&self, path: &str) -> Result<Opened, String> {
        let (ours, theirs) = presentation::side_labels(self.snapshot.operation.as_ref());
        let session = self.session(path)?;
        let lines = review::find(&self.git, &self.root, path)
            .ok()
            .flatten()
            .map(|found| {
                found
                    .conflicts
                    .into_iter()
                    .filter_map(|conflict| Some((conflict.id, conflict.line?)))
                    .collect()
            })
            .unwrap_or_default();
        Ok(Opened {
            session,
            lines,
            ours,
            theirs,
        })
    }

    /// The candidate `kind` of conflict `id`, as `conflicts` showed it: the
    /// file opened as the app opens it, its candidates checked.
    fn picked(&self, path: &str, id: &str, kind: CandidateKind) -> Result<Fill, String> {
        let session = self.session(path)?;
        let index = index_of(&session, id).ok_or_else(|| stale(self, path, id, &session))?;
        let candidates = &session.analysis(index).candidates;
        let candidate = candidates
            .iter()
            .find(|candidate| candidate.answers(kind))
            .ok_or_else(|| {
                format!(
                    "Conflict {id} offers no candidate {}. Its candidates: {}. Or write text of your own.",
                    kind.id(),
                    candidates
                        .iter()
                        .map(|candidate| candidate.kind.id())
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            })?;
        Ok(Fill::Pick {
            kind,
            text: candidate.text.clone(),
        })
    }

    /// Why `resolve` wrote nothing, for the agent to act on.
    fn refusal(&self, path: &str, id: &str, error: WriteError) -> String {
        match error {
            WriteError::Resolved(resolver) => {
                let who = match resolver {
                    Resolver::Outside => String::from(
                        "outside Scry without an agent's record: by hand, or with another tool",
                    ),
                    Resolver::ChangedSince(by) => format!("by {by}, then changed by someone else"),
                    Resolver::Reviewed => {
                        String::from("and the user reviewed that resolution in Scry")
                    }
                };
                format!(
                    "Nothing written: conflict {id} is already resolved {who}. Keep it, or ask the user before replacing it with replace: true."
                )
            }
            WriteError::NoSuchConflict(ids) => stale_ids(self, path, id, &ids),
            WriteError::Markers => String::from(
                "Nothing written: the text holds conflict markers. Write the conflict's resolved lines only.",
            ),
            WriteError::ChangedOutside => format!(
                "Nothing written: {path} changed outside its conflicts since git wrote it, so where each conflict starts and ends cannot be told. The user resolves it in Scry."
            ),
            WriteError::NotConflicted => {
                format!("Nothing written: git holds no text conflict for {path}.")
            }
            error => format!("Nothing written: {error}"),
        }
    }

    /// What the file says once conflict `id` is written: whether it parses
    /// when no conflict is left open, else the pick's own check and how
    /// many are left.
    fn read_back(&self, path: &str, id: &str, pick: Option<CandidateKind>) -> String {
        let session = match self.session(path) {
            Ok(session) => session,
            Err(error) => return format!("It could not be read back: {error}"),
        };
        let open = session.unresolved();
        if open == 0 {
            return String::from(match session.merged_check() {
                ParseCheck::Parses => "The file parses.",
                ParseCheck::DoesNotParse { line } => {
                    return format!(
                        "The file does not parse: error on line {line}. Fix it with resolve again."
                    );
                }
                ParseCheck::NotChecked => {
                    "Not checked by parsing: Scry has no grammar it trusts for this file."
                }
            });
        }
        let parse = pick.zip(index_of(&session, id)).and_then(|(kind, index)| {
            session
                .analysis(index)
                .candidates
                .iter()
                .find(|candidate| candidate.answers(kind))
                .map(|candidate| candidate.parse)
        });
        let check = match parse {
            Some(ParseCheck::Parses) => "The file parses with it, the open conflicts taken as ours",
            Some(ParseCheck::DoesNotParse { .. }) => {
                "The file does not parse with it, the open conflicts taken as ours"
            }
            _ => "Not checked by parsing yet",
        };
        format!(
            "{check}; {} left in this file.",
            plural(open, "open conflict")
        )
    }

    /// Why a conflicted file shows no conflict.
    fn nothing_to_tell_apart(&self, path: &str) -> String {
        let changed_outside = self
            .snapshot
            .files
            .iter()
            .find(|file| file.path == path)
            .and_then(|file| file.review)
            .is_some_and(|progress| progress.changed_outside);
        match changed_outside {
            true => format!(
                "{path} has no markers left, and its text changed outside its conflicts too: Scry cannot tell its conflicts apart. The user reviews it in Scry."
            ),
            false => format!("{path} has no conflict left."),
        }
    }
}

/// The index of conflict `id` in `session`.
fn index_of(session: &MergeSession, id: &str) -> Option<usize> {
    (0..session.conflict_count()).find(|index| session.analysis_so_far(*index).id == id)
}

/// Why conflict `id` is not in `path`, with the ids it has now.
fn stale(repo: &Repo, path: &str, id: &str, session: &MergeSession) -> String {
    let ids: Vec<String> = (0..session.conflict_count())
        .map(|index| session.analysis_so_far(index).id.clone())
        .collect();
    stale_ids(repo, path, id, &ids)
}

fn stale_ids(repo: &Repo, path: &str, id: &str, ids: &[String]) -> String {
    match ids.is_empty() {
        true => format!(
            "No conflict {id} in {path}: {}",
            repo.nothing_to_tell_apart(path)
        ),
        false => format!(
            "No conflict {id} in {path}: it changed, or never was there. Its conflicts now: {}.",
            ids.join(", ")
        ),
    }
}

/// A file these tools work on.
fn text_file(file: &FileView) -> bool {
    file.state != State::Staged && file.special.is_none() && file.openable
}

/// A side with the commit it is at, when its label does not say it.
fn side(side: &Side) -> String {
    match side.detail() {
        Some(detail) => format!("{}, {detail}", side.label()),
        None => side.label(),
    }
}

/// Where a conflict stands.
fn state(choice: Option<&Choice>) -> String {
    match choice {
        None => String::from("open"),
        Some(Choice::Candidate(kind)) => format!("resolved in Scry: {}", kind.id()),
        Some(Choice::Edited(_)) => String::from("resolved in Scry: own text"),
        Some(Choice::Outside(outside)) => {
            let by = outside
                .record
                .as_ref()
                .map_or(String::new(), |record| format!(" by {}", record.by));
            let what = outside.matches.map_or("own text", |kind| kind.id());
            let reviewed = match outside.reviewed {
                true => "reviewed",
                false => "not reviewed yet",
            };
            format!("resolved{by}: {what}, {reviewed}")
        }
    }
}

/// A candidate's kind, the kinds giving the same text, and its parse mark.
fn candidate_name(candidate: &Candidate) -> String {
    let mut name = candidate.kind.id().to_string();
    for alias in &candidate.aliases {
        name += &format!(" (same as {})", alias.id());
    }
    name += match candidate.parse {
        ParseCheck::Parses => " ✓",
        ParseCheck::DoesNotParse { .. } => " ✗",
        ParseCheck::NotChecked => "",
    };
    name
}

/// The last commit to change a side's lines, as a clause.
fn last_change(commit: Option<&LineCommit>) -> String {
    match commit {
        Some(commit) => format!(
            ", last changed by {} \"{}\" ({}, {})",
            commit.short_id,
            commit.subject,
            commit.author,
            blame::age_from_now(commit.time)
        ),
        None => String::new(),
    }
}

/// `text` between fences longer than any run of backticks in it, so it
/// reads back exactly; a missing final line break is said.
fn fenced(text: &str) -> String {
    if text.is_empty() {
        return String::from("(empty)\n");
    }
    let longest = text
        .split(|character| character != '`')
        .map(str::len)
        .max()
        .unwrap_or(0);
    let fence = "`".repeat(longest.max(2) + 1);
    match text.ends_with('\n') {
        true => format!("{fence}\n{text}{fence}\n"),
        false => format!("{fence}\n{text}\n{fence}\n(no line break at the end)\n"),
    }
}

/// A list under its title and count, when it has anything.
fn section(text: &mut String, title: &str, lines: &[String]) {
    if !lines.is_empty() {
        *text += &format!("{title} ({}):\n{}\n", lines.len(), lines.join("\n"));
    }
}

fn plural(count: usize, noun: &str) -> String {
    match count {
        1 => format!("1 {noun}"),
        _ => format!("{count} {noun}s"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A conflict's text holding a fence still reads back exactly: the
    /// fence around it is longer than any run of backticks inside.
    #[test]
    fn fences_are_longer_than_the_text_s_backticks() {
        assert_eq!(fenced("a\n"), "```\na\n```\n");
        assert_eq!(fenced("````\n"), "`````\n````\n`````\n");
        assert_eq!(fenced(""), "(empty)\n");
        assert_eq!(fenced("a"), "```\na\n```\n(no line break at the end)\n");
    }
}
