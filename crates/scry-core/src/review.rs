//! What was done to a conflicted file resolved outside Scry: by the user's
//! agent, another editor or a person.
//!
//! git keeps the three versions of an unmerged file in the index (stages 1,
//! 2 and 3) until it is staged, so the file can be written again as git
//! wrote it, with its markers. The file as it is now is then laid over that
//! one: the text around the conflicts must still be there, in order, and
//! what lies between is each conflict's resolution, named by the candidate
//! it equals when it equals one.

use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::Path;

use serde::Serialize;

use crate::conflict::{self, ParseError, Segment};
use crate::git::Git;
use crate::records::{Record, Store};
use crate::resolution::{self, Analysis, CandidateKind};
use crate::session::{self, LoadError, MergeSession, MergetoolFiles, Origin, Outside};
use crate::syntax::Language;
use crate::syntax::ParseCheck;

/// How one conflict was resolved.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", content = "value", rename_all = "kebab-case")]
pub enum Resolution {
    /// Its markers are still there.
    Open,
    /// The text of a candidate the editor offers for it.
    Candidate(CandidateKind),
    /// Other text, line breaks included.
    Text(String),
    /// The text around the conflicts changed, so where this one's
    /// resolution starts and ends cannot be told.
    Unknown,
}

/// One conflict of the file and how it was resolved.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ConflictReview {
    /// The id Scry showed while the markers were there.
    pub id: String,
    /// Position in the file, from 1.
    pub number: usize,
    /// The line of the file, from 1, where the resolution starts; `None`
    /// when it is unknown or empty.
    pub line: Option<usize>,
    pub resolution: Resolution,
    /// The resolution's text as git's would hold it, with the final line
    /// break git adds, and the line where it starts, even when empty: what
    /// a review session is built from.
    #[serde(skip)]
    pub(crate) region: Option<(String, usize)>,
}

/// What was done to a file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FileReview {
    pub conflicts: Vec<ConflictReview>,
    /// The text around the conflicts is no longer what git wrote: a review
    /// must show it, since nobody asked for it.
    pub changed_outside: bool,
    /// Whether the file ends with a line break (or is empty).
    pub ends_with_break: bool,
}

/// How far the review of a file resolved outside Scry is, for the
/// overview.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Progress {
    /// Conflicts whose resolution the user reviewed, or made in Scry.
    pub reviewed: usize,
    pub total: usize,
    /// The text around the conflicts changed: which conflict holds what
    /// cannot be told, so the file is reviewed as a whole.
    pub changed_outside: bool,
}

impl Progress {
    /// Nothing left to review: the file can be staged without asking.
    pub fn is_done(&self) -> bool {
        !self.changed_outside && self.reviewed == self.total
    }
}

/// How far the review of `found` is, given the conflicts `reviewed` (from
/// `Store::reviewed`) with the text each held then.
pub fn progress(found: &FileReview, reviewed: &BTreeMap<String, String>) -> Progress {
    let done = found
        .conflicts
        .iter()
        .filter(|conflict| match (&conflict.resolution, &conflict.region) {
            (Resolution::Candidate(_) | Resolution::Text(_), Some((text, _))) => {
                reviewed.get(&conflict.id) == Some(text)
            }
            _ => false,
        })
        .count();
    Progress {
        reviewed: done,
        total: found.conflicts.len(),
        changed_outside: found.changed_outside,
    }
}

/// Opens a conflicted file for the editor: as a review when at least one
/// of its conflicts was resolved outside Scry, else from its markers as
/// `session::open_file` reads them. The review is kept either way, so a
/// conflict the user resolves in Scry is not asked again. `git` runs at
/// `root`, `beside` in the file's folder (what `open_file` needs); `path`
/// is from the root.
pub fn opened(
    git: &dyn Git,
    beside: &dyn Git,
    root: &Path,
    path: &str,
    (ours, theirs): (&str, &str),
) -> Result<MergeSession, LoadError> {
    if let Ok(Some(session)) = open(git, root, path, ours, theirs)
        && session.unreviewed() + session.reviewed().len() > 0
    {
        return Ok(session);
    }
    let session = session::open_file(beside, &root.join(path))?;
    Ok(match Store::open(git) {
        Ok(store) => session.keeping_reviews(store, path.to_string()),
        Err(_) => session,
    })
}

/// What was done to `path` (from the repository root), with `git` running
/// at the root. `None` when the index lacks one of the sides (deleted on
/// one side, already staged): there is no text conflict to rebuild.
pub fn find(git: &dyn Git, root: &Path, path: &str) -> Result<Option<FileReview>, LoadError> {
    let file = root.join(path);
    let bytes = match fs::read(&file) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(LoadError::Io(file, error)),
    };
    let current = String::from_utf8(bytes).map_err(|_| LoadError::NotText(file.clone()))?;
    let marker_size = conflict::marker_size(git, Path::new(path));
    let Some(written) = as_git_wrote(git, path, &file, marker_size, ("ours", "theirs"))? else {
        return Ok(None);
    };
    let known = Store::open(git)
        .map(|store| known(&store.records(path), &store.reviewed(path)))
        .unwrap_or_default();
    resolutions(&written.text, &current, marker_size, &known)
        .map(Some)
        .map_err(LoadError::Parse)
}

/// `path` (from the repository root) opened for review: a session on its
/// conflicts rebuilt from the index, with the sides named `ours` and
/// `theirs`, each conflict's choice being what the file holds
/// (`Choice::Outside`), with the agent's record and the review kept in
/// the git folder. `None` when there is no text conflict to rebuild, or
/// the text around the conflicts changed: there is then no conflict by
/// conflict review.
pub fn open(
    git: &dyn Git,
    root: &Path,
    path: &str,
    ours: &str,
    theirs: &str,
) -> Result<Option<MergeSession>, LoadError> {
    let file = root.join(path);
    let current = fs::read(&file).map_err(|error| LoadError::Io(file.clone(), error))?;
    let current = String::from_utf8(current).map_err(|_| LoadError::NotText(file.clone()))?;
    let marker_size = conflict::marker_size(git, Path::new(path));
    let Some(written) = as_git_wrote(git, path, &file, marker_size, (ours, theirs))? else {
        return Ok(None);
    };
    let store = Store::open(git).ok();
    let (records, reviewed) = store
        .as_ref()
        .map(|store| (store.records(path), store.reviewed(path)))
        .unwrap_or_default();
    let found = session(
        &written.text,
        &current,
        marker_size,
        Language::from_path(&file),
        written.breaks,
        &records,
        &reviewed,
    )
    .map_err(LoadError::Parse)?;
    Ok(found.map(|session| {
        let session = session.with_file(file);
        match store {
            Some(store) => session.keeping_reviews(store, path.to_string()),
            None => session,
        }
    }))
}

/// A review session on `current`, resolved from `written` (git's text with
/// markers): the conflicts of `written`, each with what `current` holds as
/// its choice, its record when it still matches, and reviewed when the
/// review kept that same text. `breaks` say whether ours and theirs end
/// with a line break. `None` when the text around the conflicts changed.
pub fn session(
    written: &str,
    current: &str,
    marker_size: usize,
    language: Option<Language>,
    breaks: (bool, bool),
    records: &BTreeMap<String, Record>,
    reviewed: &BTreeMap<String, String>,
) -> Result<Option<MergeSession>, ParseError> {
    let found = resolutions(written, current, marker_size, &known(records, reviewed))?;
    // Markers in a resolution: the file was written in another conflict
    // style than git's now (`git checkout --conflict=diff3`), and those
    // conflicts are open, which only the markers themselves can show.
    let holds_markers = |text: &str| holds_markers(text, marker_size);
    if found.changed_outside
        || found
            .conflicts
            .iter()
            .any(|conflict| matches!(&conflict.resolution, Resolution::Text(text) if holds_markers(text)))
    {
        return Ok(None);
    }
    let mut segments = conflict::parse_with_marker_size(written, marker_size)?;
    let mut places = found
        .conflicts
        .iter()
        .map(|conflict| conflict.region.as_ref());
    for segment in &mut segments {
        if let Segment::Conflict(conflict) = segment
            && let Some(Some((_, line))) = places.next()
        {
            conflict.start_line = *line;
        }
    }
    let session =
        MergeSession::from_segments(segments, Origin::Resolved, current.to_string(), language)
            .with_final_breaks(breaks.0, breaks.1);
    let outside = found
        .conflicts
        .into_iter()
        .enumerate()
        .filter_map(|(index, conflict)| {
            let (text, _) = conflict.region?;
            let matches = match conflict.resolution {
                Resolution::Candidate(kind) => Some(kind),
                Resolution::Text(_) => None,
                Resolution::Open | Resolution::Unknown => return None,
            };
            let record = records
                .get(&conflict.id)
                .filter(|record| record.text == text)
                .cloned();
            let reviewed = reviewed.get(&conflict.id) == Some(&text);
            let outside = Outside {
                text,
                matches,
                record,
                parse: ParseCheck::NotChecked,
                reviewed,
            };
            Some((index, outside))
        })
        .collect();
    Ok(Some(
        session.resolved_outside(outside, found.ends_with_break),
    ))
}

/// Why `write` wrote nothing.
#[derive(Debug)]
pub enum WriteError {
    Load(LoadError),
    /// The index lacks a side of the file: no text conflict to resolve.
    NotConflicted,
    /// The text around the conflicts changed: where each conflict starts
    /// and ends cannot be told.
    ChangedOutside,
    /// No conflict with that id; the file's ids now.
    NoSuchConflict(Vec<String>),
    /// The text holds conflict markers.
    Markers,
    /// The conflict holds a resolution that is not an agent's own to
    /// change: who made it.
    Resolved(Resolver),
    Io(std::path::PathBuf, std::io::Error),
}

impl std::fmt::Display for WriteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WriteError::Load(error) => write!(f, "{error}"),
            WriteError::NotConflicted => write!(f, "git holds no text conflict for this file"),
            WriteError::ChangedOutside => {
                write!(
                    f,
                    "the text around the conflicts changed since git wrote it"
                )
            }
            WriteError::NoSuchConflict(ids) => {
                write!(f, "no such conflict; now: {}", ids.join(", "))
            }
            WriteError::Markers => write!(f, "the text holds conflict markers"),
            WriteError::Resolved(by) => write!(f, "already resolved {by}"),
            WriteError::Io(path, error) => write!(f, "{}: {error}", path.display()),
        }
    }
}

impl From<LoadError> for WriteError {
    fn from(error: LoadError) -> WriteError {
        WriteError::Load(error)
    }
}

/// What resolves a conflict written with `write`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fill {
    /// A candidate's text, written byte for byte: the file then holds that
    /// candidate, final line break included.
    Pick { kind: CandidateKind, text: String },
    /// Text of its own, written with the file's line breaks and a final
    /// one; empty removes the conflict's lines.
    Text(String),
}

/// Who made a resolution `write` will not replace unless asked to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolver {
    /// No record of an agent: by hand, another editor or tool.
    Outside,
    /// An agent, then changed by someone else since.
    ChangedSince(String),
    /// Reviewed by the user in Scry, or chosen there.
    Reviewed,
}

impl std::fmt::Display for Resolver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Resolver::Outside => write!(
                f,
                "outside Scry, without an agent's record (by hand or another tool)"
            ),
            Resolver::ChangedSince(by) => write!(f, "by {by}, then changed by someone else"),
            Resolver::Reviewed => write!(f, "and reviewed by the user in Scry"),
        }
    }
}

/// What `write` may replace: a conflict still open, or one an agent
/// resolved and nobody changed or reviewed since (the agent correcting its
/// own resolution). Anything else only when `replace` is set.
#[derive(Debug, Clone, Default)]
pub struct Guard {
    /// The records of the file, by conflict id (`Store::records`).
    pub records: BTreeMap<String, Record>,
    /// The reviewed conflicts of the file (`Store::reviewed`).
    pub reviewed: BTreeMap<String, String>,
    pub replace: bool,
}

/// Resolves conflict `id` of `path` (from the repository root, `git`
/// running there) with `fill`, writing that conflict's region of the file
/// only: the text around it and the other conflicts, open or resolved,
/// stay as they are. A resolution that is not an agent's own is kept
/// unless `replace` (see `Guard`). Returns the region's text as written,
/// what a `Record` keeps.
pub fn write(
    git: &dyn Git,
    root: &Path,
    path: &str,
    id: &str,
    fill: &Fill,
    replace: bool,
) -> Result<String, WriteError> {
    let file = root.join(path);
    let current = fs::read(&file).map_err(|error| LoadError::Io(file.clone(), error))?;
    let current = String::from_utf8(current).map_err(|_| LoadError::NotText(file.clone()))?;
    let marker_size = conflict::marker_size(git, Path::new(path));
    let written = as_git_wrote(git, path, &file, marker_size, ("ours", "theirs"))?
        .ok_or(WriteError::NotConflicted)?;
    let (records, reviewed) = Store::open(git)
        .map(|store| (store.records(path), store.reviewed(path)))
        .unwrap_or_default();
    let guard = Guard {
        records,
        reviewed,
        replace,
    };
    let (result, text) = match rewrite(
        &written.text,
        written.breaks,
        &current,
        marker_size,
        id,
        fill,
        &guard,
    ) {
        // Markers in another conflict style than git's now (`git checkout
        // --conflict=diff3`): the file's own markers are the conflicts the
        // agent was shown, laid over the file itself.
        Err(error @ (WriteError::ChangedOutside | WriteError::NoSuchConflict(_)))
            if has_conflicts(&current, marker_size) =>
        {
            let own = rewrite(
                &current,
                written.breaks,
                &current,
                marker_size,
                id,
                fill,
                &guard,
            );
            match own {
                // Not among the markers either: the text around it changed.
                Err(WriteError::NoSuchConflict(_))
                    if matches!(error, WriteError::ChangedOutside) =>
                {
                    return Err(error);
                }
                other => other?,
            }
        }
        other => other?,
    };
    session::write_atomically(&file, &result).map_err(|error| WriteError::Io(file, error))?;
    Ok(text)
}

/// `current` with conflict `id` resolved by `fill`, `written` being the
/// file as git wrote it with markers of `marker_size`, and `breaks` whether
/// ours and theirs end with a line break: what `write` writes, and the
/// region's text as written (with the final line break git adds).
pub fn rewrite(
    written: &str,
    breaks: (bool, bool),
    current: &str,
    marker_size: usize,
    id: &str,
    fill: &Fill,
    guard: &Guard,
) -> Result<(String, String), WriteError> {
    let known = known(&guard.records, &guard.reviewed);
    let laid = Laid::new(written, current, marker_size, &known).map_err(LoadError::Parse)?;
    let Some(index) = laid.analyses.iter().position(|analysis| analysis.id == id) else {
        return Err(WriteError::NoSuchConflict(
            laid.analyses
                .iter()
                .map(|analysis| analysis.id.clone())
                .collect(),
        ));
    };
    let text = match fill {
        Fill::Pick { text, .. } => text.clone(),
        Fill::Text(text) => {
            if holds_markers(text, marker_size) {
                return Err(WriteError::Markers);
            }
            let mut text = text.replace("\r\n", "\n");
            if !text.is_empty() && !text.ends_with('\n') {
                text.push('\n');
            }
            if session::line_break(current) == "\r\n" {
                text = text.replace('\n', "\r\n");
            }
            text
        }
    };
    let regions = laid.regions().ok_or(WriteError::ChangedOutside)?;
    let (start, end) = regions[index];
    let region = &laid.text[start..end];
    if !guard.replace && !laid.layout().is_open(index, region) {
        let record = guard.records.get(id);
        let reviewed = guard.reviewed.get(id).is_some_and(|text| text == region);
        let resolver = match record {
            _ if reviewed => Some(Resolver::Reviewed),
            Some(record) if record.text == region => None,
            Some(record) => Some(Resolver::ChangedSince(record.by.clone())),
            None => Some(Resolver::Outside),
        };
        if let Some(resolver) = resolver {
            return Err(WriteError::Resolved(resolver));
        }
    }
    let mut result = format!("{}{text}{}", &laid.text[..start], &laid.text[end..]);
    let ends_the_file = end == laid.text.len();
    let final_break = match fill {
        // The file ends as the side the candidate ends with does.
        Fill::Pick { kind, text } if ends_the_file && !text.is_empty() => match kind {
            CandidateKind::Theirs
            | CandidateKind::OursThenTheirs
            | CandidateKind::OursThenTheirsClosed => breaks.1,
            _ => breaks.0,
        },
        // As the file did.
        _ => !laid.added_break,
    };
    if !final_break && result.ends_with('\n') {
        result.pop();
        // A candidate's own CRLF break goes whole; the break laid over a
        // file without one is a bare line feed.
        if matches!(fill, Fill::Pick { .. }) && result.ends_with('\r') {
            result.pop();
        }
    }
    Ok((result, text))
}

/// `text` holds whole conflicts with markers of `marker_size`.
fn has_conflicts(text: &str, marker_size: usize) -> bool {
    conflict::parse_with_marker_size(text, marker_size)
        .is_ok_and(|segments| conflict::conflicts(&segments).next().is_some())
}

/// `text` holds conflict markers of `marker_size`, or ones that do not
/// parse.
fn holds_markers(text: &str, marker_size: usize) -> bool {
    conflict::parse_with_marker_size(text, marker_size).map_or(true, |segments| {
        conflict::conflicts(&segments).next().is_some()
    })
}

/// A file as git writes it, and whether ours and theirs end with a line
/// break.
struct Written {
    text: String,
    breaks: (bool, bool),
}

/// The file with its markers as git writes it from the index's versions,
/// in the conflict style of the user's configuration, as the merge wrote
/// it. The base is empty when only the sides have the file (both added).
fn as_git_wrote(
    git: &dyn Git,
    path: &str,
    file: &Path,
    marker_size: usize,
    (ours, theirs): (&str, &str),
) -> Result<Option<Written>, LoadError> {
    // Calls at once in one process (the app's threads, parallel tests) can
    // read the same clock: the count keeps their folders apart.
    static FOLDERS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let count = FOLDERS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |time| time.as_nanos());
    let folder = std::env::temp_dir().join(format!(
        "scry-review-{}-{nanos}-{count}",
        std::process::id()
    ));
    fs::create_dir(&folder).map_err(|error| LoadError::Io(folder.clone(), error))?;
    let files = MergetoolFiles {
        base: folder.join("base"),
        local: folder.join("ours"),
        remote: folder.join("theirs"),
        merged: file.to_path_buf(),
    };
    let written = (|| {
        for (stage, version) in [(1, &files.base), (2, &files.local), (3, &files.remote)] {
            let output = git.output(&["show", &format!(":{stage}:{path}")])?;
            let bytes = match output.status {
                Some(0) => output.stdout,
                _ if stage == 1 => Vec::new(),
                _ => return Ok(None),
            };
            fs::write(version, bytes).map_err(|error| LoadError::Io(version.clone(), error))?;
        }
        let ends_with_break = |path: &Path| {
            fs::read(path).is_ok_and(|bytes| bytes.last().is_none_or(|byte| *byte == b'\n'))
        };
        let breaks = (
            ends_with_break(&files.local),
            ends_with_break(&files.remote),
        );
        session::merge_file(git, &files, ours, theirs, marker_size, None)
            .map(|text| Some(Written { text, breaks }))
    })();
    let _ = fs::remove_dir_all(&folder);
    written
}

/// Texts the file is known to have held for each conflict, by id: what an
/// agent recorded writing, what the user reviewed. Where a resolution could
/// end in several places, these are tried first.
pub type Known = BTreeMap<String, Vec<String>>;

/// What the records and the review of a file say it held.
pub fn known(records: &BTreeMap<String, Record>, reviewed: &BTreeMap<String, String>) -> Known {
    let mut known = Known::new();
    for (id, record) in records {
        known
            .entry(id.clone())
            .or_default()
            .push(record.text.clone());
    }
    for (id, text) in reviewed {
        known.entry(id.clone()).or_default().push(text.clone());
    }
    known
}

/// What was done to each conflict of `written` (the file as git wrote it,
/// with markers of `marker_size`) to give `current`, the texts in `known`
/// tried first where a resolution could end in several places.
pub fn resolutions(
    written: &str,
    current: &str,
    marker_size: usize,
    known: &Known,
) -> Result<FileReview, ParseError> {
    let laid = Laid::new(written, current, marker_size, known)?;
    let layout = laid.layout();
    let found = laid.regions();
    let text = &laid.text;
    let reviews = laid
        .analyses
        .iter()
        .enumerate()
        .map(|(index, analysis)| {
            let (line, resolution, region) = match &found {
                Some(regions) => {
                    let (start, end) = regions[index];
                    let shown = if laid.added_break && end == text.len() {
                        &current[start..]
                    } else {
                        &text[start..end]
                    };
                    let at = text[..start].matches('\n').count() + 1;
                    let line = (start < end).then_some(at);
                    let region = (text[start..end].to_string(), at);
                    (
                        line,
                        layout.name(index, &text[start..end], shown),
                        Some(region),
                    )
                }
                None => (None, Resolution::Unknown, None),
            };
            ConflictReview {
                id: analysis.id.clone(),
                number: analysis.number,
                line,
                resolution,
                region,
            }
        })
        .collect();
    Ok(FileReview {
        conflicts: reviews,
        changed_outside: found.is_none(),
        ends_with_break: current.is_empty() || current.ends_with('\n'),
    })
}

/// The file as it is, ready to be laid over git's text with markers.
struct Laid {
    analyses: Vec<Analysis>,
    conflicts: Vec<conflict::Conflict>,
    /// The common text before, between and after the conflicts; empty
    /// when the file starts or ends with one.
    commons: Vec<String>,
    /// The file, with the line break git adds when it lacks a final one.
    text: String,
    added_break: bool,
    marker_size: usize,
    /// Each conflict's known texts, in the order of `analyses`.
    known: Vec<Vec<String>>,
}

impl Laid {
    fn new(
        written: &str,
        current: &str,
        marker_size: usize,
        known: &Known,
    ) -> Result<Laid, ParseError> {
        let segments = conflict::parse_with_marker_size(written, marker_size)?;
        let analyses = resolution::analyze(&segments);
        let mut commons = vec![String::new()];
        for segment in &segments {
            match segment {
                Segment::Common(text) => commons.last_mut().unwrap().push_str(text),
                Segment::Conflict(_) => commons.push(String::new()),
            }
        }
        // git adds a line break to sides that had none at the end of the
        // file; the file then ends without one, which is laid over git's
        // text with it.
        let added_break =
            !current.is_empty() && !current.ends_with('\n') && written.ends_with('\n');
        let text = if added_break {
            format!("{current}\n")
        } else {
            current.to_string()
        };
        let conflicts = conflict::conflicts(&segments).cloned().collect();
        let known = analyses
            .iter()
            .map(|analysis| known.get(&analysis.id).cloned().unwrap_or_default())
            .collect();
        Ok(Laid {
            known,
            analyses,
            conflicts,
            commons,
            text,
            added_break,
            marker_size,
        })
    }

    fn layout(&self) -> Layout<'_> {
        Layout {
            text: &self.text,
            commons: &self.commons,
            analyses: &self.analyses,
            conflicts: &self.conflicts,
            marker_size: self.marker_size,
            known: &self.known,
        }
    }

    /// Each conflict's region in `text`, as byte ranges; `None` when the
    /// text around the conflicts changed.
    fn regions(&self) -> Option<Vec<(usize, usize)>> {
        self.layout().place(0, 0, &mut HashSet::new())
    }
}

/// The file laid over git's: `commons[k]` must start where conflict `k`'s
/// resolution ends.
struct Layout<'a> {
    text: &'a str,
    commons: &'a [String],
    analyses: &'a [Analysis],
    conflicts: &'a [conflict::Conflict],
    marker_size: usize,
    known: &'a [Vec<String>],
}

impl Layout<'_> {
    /// Each conflict's region in the text, as byte ranges, with
    /// `commons[k]` at `at`; `None` when the text around the conflicts does
    /// not fit. Where a resolution could end in several places (it repeats
    /// the lines after it), a text the file is known to hold, a candidate's
    /// text or the conflict left open is tried first, then the shortest
    /// region that lets the rest fit.
    fn place(
        &self,
        k: usize,
        at: usize,
        failed: &mut HashSet<(usize, usize)>,
    ) -> Option<Vec<(usize, usize)>> {
        if failed.contains(&(k, at)) {
            return None;
        }
        let common = &self.commons[k];
        let found = self.text[at..]
            .starts_with(common.as_str())
            .then_some(())
            .and_then(|()| {
                let start = at + common.len();
                if k == self.analyses.len() {
                    return (start == self.text.len()).then(Vec::new);
                }
                let mut ends: Vec<usize> = self.preferred_ends(k, start);
                ends.extend(line_starts(self.text, start));
                // Markers belong to a conflict left open: a region holds
                // them only when it is its own conflict, with them.
                let opening = "<".repeat(self.marker_size);
                let markers = line_starts(self.text, start)
                    .find(|line| self.text[*line..].starts_with(&opening))
                    .unwrap_or(usize::MAX);
                let open = (markers == start)
                    .then(|| self.open_end(k, start))
                    .flatten();
                let mut tried = HashSet::new();
                ends.into_iter()
                    .filter(|end| *end <= markers || Some(*end) == open)
                    .filter(|end| tried.insert(*end))
                    .find_map(|end| {
                        let mut rest = self.place(k + 1, end, failed)?;
                        rest.insert(0, (start, end));
                        Some(rest)
                    })
            });
        if found.is_none() {
            failed.insert((k, at));
        }
        found
    }

    /// Where conflict `k`'s region ends if it holds a candidate's text, or
    /// the conflict with its markers.
    fn preferred_ends(&self, k: usize, start: usize) -> Vec<usize> {
        let rest = &self.text[start..];
        let fits = |text: &&str| rest.starts_with(text);
        let end = |text: &str| start + text.len();
        // What the file is known to hold first: an agent's own text can
        // repeat the lines between two conflicts, where the shortest region
        // would cut it short, and two conflicts alike can only be told apart
        // by it. A record no longer true leaves the rest unplaced, and the
        // next end is tried.
        let mut ends: Vec<usize> = self.known[k]
            .iter()
            .map(String::as_str)
            .filter(fits)
            .map(end)
            .collect();
        let candidates = self.analyses[k]
            .candidates
            .iter()
            .map(|candidate| candidate.text.as_str())
            .filter(fits)
            .map(end);
        if let Some(open) = self.open_end(k, start) {
            // Markers opening right here: the conflict is still open, even
            // where an empty candidate would fit too.
            if rest.starts_with(&"<".repeat(self.marker_size)) {
                ends.push(open);
                ends.extend(candidates);
            } else {
                ends.extend(candidates);
                ends.push(open);
            }
        } else {
            ends.extend(candidates);
        }
        ends
    }

    /// Where conflict `k` ends when it is left open at `start`: after the
    /// first closing marker, when what lies before is that conflict.
    fn open_end(&self, k: usize, start: usize) -> Option<usize> {
        let closing = ">".repeat(self.marker_size);
        line_starts(self.text, start)
            .zip(line_starts(self.text, start).skip(1))
            .find(|(line, _)| self.text[*line..].starts_with(&closing))
            .map(|(_, next)| next)
            .filter(|end| self.is_open(k, &self.text[start..*end]))
    }

    /// `region` is conflict `k` with its markers.
    fn is_open(&self, k: usize, region: &str) -> bool {
        conflict::parse_with_marker_size(region, self.marker_size).is_ok_and(|segments| {
            let conflict = &self.conflicts[k];
            matches!(segments.as_slice(), [Segment::Conflict(found)]
                if found.ours.text == conflict.ours.text && found.theirs.text == conflict.theirs.text)
        })
    }

    /// How `region` (with the final line break git adds, when the file
    /// lacks it) resolves conflict `k`; `shown` is the region as the file
    /// has it.
    fn name(&self, k: usize, region: &str, shown: &str) -> Resolution {
        if let Some(candidate) = self.analyses[k]
            .candidates
            .iter()
            .find(|candidate| candidate.text == region)
        {
            return Resolution::Candidate(candidate.kind);
        }
        if self.is_open(k, region) {
            Resolution::Open
        } else {
            Resolution::Text(shown.to_string())
        }
    }
}

/// The offsets in `text` from `start` on where a line starts. A region
/// never needs to end inside a last line without a line break: git ends
/// its markers with one, and the file is given one where git's text has it.
fn line_starts(text: &str, start: usize) -> impl Iterator<Item = usize> + '_ {
    std::iter::once(start).chain(
        text[start..]
            .match_indices('\n')
            .map(move |(index, _)| start + index + 1),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What `write` refuses reads as a sentence, with the ids it has now.
    #[test]
    fn write_errors_say_what_happened() {
        assert_eq!(
            WriteError::NoSuchConflict(vec![String::from("c1"), String::from("c2")]).to_string(),
            "no such conflict; now: c1, c2"
        );
        assert_eq!(
            WriteError::ChangedOutside.to_string(),
            "the text around the conflicts changed since git wrote it"
        );
        assert_eq!(
            WriteError::Markers.to_string(),
            "the text holds conflict markers"
        );
        assert_eq!(
            WriteError::Resolved(Resolver::Outside).to_string(),
            "already resolved outside Scry, without an agent's record (by hand or another tool)"
        );
        assert_eq!(
            Resolver::ChangedSince(String::from("codex 1")).to_string(),
            "by codex 1, then changed by someone else"
        );
        assert_eq!(
            Resolver::Reviewed.to_string(),
            "and reviewed by the user in Scry"
        );
    }

    /// When a resolution could end in two places, both leaving the rest of
    /// the file in order, the one holding a candidate's text wins: the
    /// review then says "theirs" rather than two pieces of own text. Here
    /// theirs repeats the line between the conflicts.
    #[test]
    fn a_candidate_is_preferred_where_the_text_could_split_two_ways() {
        let written = "<<<<<<< ours\no\n=======\na\nm\nb\n>>>>>>> theirs\nm\n\
            <<<<<<< ours\np\n=======\nq\n>>>>>>> theirs\n";
        let review = resolutions(written, "a\nm\nb\nm\nc\n", 7, &Known::new()).unwrap();
        let found: Vec<Resolution> = review.conflicts.into_iter().map(|c| c.resolution).collect();
        assert_eq!(
            found,
            [
                Resolution::Candidate(CandidateKind::Theirs),
                Resolution::Text(String::from("c\n")),
            ]
        );
    }

    /// A resolution made outside Scry is checked like a candidate, and the
    /// check shows even in the list, where candidates are not checked yet:
    /// own text that breaks the file says so, and a side that parses says
    /// so too.
    #[test]
    fn outside_resolutions_are_checked() {
        let written = "func a() {\n<<<<<<< ours\n  let x = 1\n=======\n  let x = 2\n>>>>>>> theirs\n}\n\
            func b() {\n  let m = 0\n  let n = 0\n  let o = 0\n}\n\
            func c() {\n<<<<<<< ours\n  let y = 1\n=======\n  let y = 2\n>>>>>>> theirs\n}\n";
        let current = "func a() {\n  let x = (\n}\n\
            func b() {\n  let m = 0\n  let n = 0\n  let o = 0\n}\n\
            func c() {\n  let y = 2\n}\n";
        let none = BTreeMap::new();
        let session = session(
            written,
            current,
            7,
            Some(Language::Swift),
            (true, true),
            &BTreeMap::new(),
            &none,
        )
        .unwrap()
        .unwrap();
        let broken = session
            .conflict_view_so_far(0, "ours", "theirs")
            .chosen
            .unwrap();
        assert!(
            matches!(broken.parse, ParseCheck::DoesNotParse { .. }),
            "{broken:?}"
        );
        let theirs = session
            .conflict_view_so_far(1, "ours", "theirs")
            .chosen
            .unwrap();
        assert_eq!(theirs.id, "theirs");
        assert_eq!(theirs.parse, ParseCheck::Parses);
    }

    /// A conflict left open whose side holds the line after it is still
    /// found open, markers and all, rather than cut at that line.
    #[test]
    fn an_open_conflict_holding_the_next_line_is_found_open() {
        let written = "<<<<<<< ours\nm\n=======\nx\n>>>>>>> theirs\nm\nend\n";
        let review = resolutions(written, written, 7, &Known::new()).unwrap();
        assert_eq!(review.conflicts[0].resolution, Resolution::Open);
    }

    /// Markers belong to the conflict they enclose: where conflicts have
    /// empty sides, an empty candidate fits anywhere, yet a conflict left
    /// open is never read as another's text, markers included, nor as an
    /// earlier conflict's resolution. Such a reading would show an open
    /// conflict as "own text".
    #[test]
    fn markers_are_never_part_of_another_conflicts_resolution() {
        let open = |theirs: &str| format!("<<<<<<< ours\n=======\n{theirs}>>>>>>> theirs\n");
        let written = [open(""), open(""), open("a\n"), open("")].concat();
        let current = format!("a\n{}{}{}", open(""), open("a\n"), open(""));
        let found: Vec<Resolution> = resolutions(&written, &current, 7, &Known::new())
            .unwrap()
            .conflicts
            .into_iter()
            .map(|conflict| conflict.resolution)
            .collect();
        assert_eq!(
            found,
            [
                Resolution::Text(String::from("a\n")),
                Resolution::Open,
                Resolution::Open,
                Resolution::Open,
            ]
        );
        let current = [open(""), String::new(), open("a\n"), String::new()].concat();
        let found: Vec<Resolution> = resolutions(&written, &current, 7, &Known::new())
            .unwrap()
            .conflicts
            .into_iter()
            .map(|conflict| conflict.resolution)
            .collect();
        assert_eq!(
            found,
            [
                Resolution::Open,
                Resolution::Candidate(CandidateKind::Ours),
                Resolution::Open,
                Resolution::Candidate(CandidateKind::Ours),
            ]
        );
    }
}
