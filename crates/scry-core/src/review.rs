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
    resolutions(&written.text, &current, marker_size)
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
    let found = resolutions(written, current, marker_size)?;
    if found.changed_outside {
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
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |time| time.as_nanos());
    let folder = std::env::temp_dir().join(format!("scry-review-{}-{nanos}", std::process::id()));
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

/// What was done to each conflict of `written` (the file as git wrote it,
/// with markers of `marker_size`) to give `current`.
pub fn resolutions(
    written: &str,
    current: &str,
    marker_size: usize,
) -> Result<FileReview, ParseError> {
    let segments = conflict::parse_with_marker_size(written, marker_size)?;
    let analyses = resolution::analyze(&segments);
    // The common text before, between and after the conflicts; empty when
    // the file starts or ends with one.
    let mut commons = vec![String::new()];
    for segment in &segments {
        match segment {
            Segment::Common(text) => commons.last_mut().unwrap().push_str(text),
            Segment::Conflict(_) => commons.push(String::new()),
        }
    }
    // git adds a line break to sides that had none at the end of the file;
    // the file then ends without one, which is laid over git's text with it.
    let added_break = !current.is_empty() && !current.ends_with('\n') && written.ends_with('\n');
    let text = if added_break {
        format!("{current}\n")
    } else {
        current.to_string()
    };
    let conflicts: Vec<_> = conflict::conflicts(&segments).cloned().collect();
    let layout = Layout {
        text: &text,
        commons: &commons,
        analyses: &analyses,
        conflicts: &conflicts,
        marker_size,
    };
    let mut failed = HashSet::new();
    let found = layout.place(0, 0, &mut failed);
    let reviews = analyses
        .iter()
        .enumerate()
        .map(|(index, analysis)| {
            let (line, resolution, region) = match &found {
                Some(regions) => {
                    let (start, end) = regions[index];
                    let shown = if added_break && end == text.len() {
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

/// The file laid over git's: `commons[k]` must start where conflict `k`'s
/// resolution ends.
struct Layout<'a> {
    text: &'a str,
    commons: &'a [String],
    analyses: &'a [Analysis],
    conflicts: &'a [conflict::Conflict],
    marker_size: usize,
}

impl Layout<'_> {
    /// Each conflict's region in the text, as byte ranges, with
    /// `commons[k]` at `at`; `None` when the text around the conflicts does
    /// not fit. Where a resolution could end in several places (it repeats
    /// the lines after it), a candidate's text or the conflict left open is
    /// tried first, then the shortest region that lets the rest fit.
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
                let mut tried = HashSet::new();
                ends.into_iter()
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
        let mut ends: Vec<usize> = self.analyses[k]
            .candidates
            .iter()
            .filter(|candidate| rest.starts_with(candidate.text.as_str()))
            .map(|candidate| start + candidate.text.len())
            .collect();
        let closing = ">".repeat(self.marker_size);
        if let Some(end) = line_starts(self.text, start)
            .zip(line_starts(self.text, start).skip(1))
            .find(|(line, _)| self.text[*line..].starts_with(&closing))
            .map(|(_, next)| next)
        {
            ends.push(end);
        }
        ends
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
        let open = conflict::parse_with_marker_size(region, self.marker_size).is_ok_and(|segments| {
            let conflict = &self.conflicts[k];
            matches!(segments.as_slice(), [Segment::Conflict(found)]
                if found.ours.text == conflict.ours.text && found.theirs.text == conflict.theirs.text)
        });
        if open {
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

    /// When a resolution could end in two places, both leaving the rest of
    /// the file in order, the one holding a candidate's text wins: the
    /// review then says "theirs" rather than two pieces of own text. Here
    /// theirs repeats the line between the conflicts.
    #[test]
    fn a_candidate_is_preferred_where_the_text_could_split_two_ways() {
        let written = "<<<<<<< ours\no\n=======\na\nm\nb\n>>>>>>> theirs\nm\n\
            <<<<<<< ours\np\n=======\nq\n>>>>>>> theirs\n";
        let review = resolutions(written, "a\nm\nb\nm\nc\n", 7).unwrap();
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
        let review = resolutions(written, written, 7).unwrap();
        assert_eq!(review.conflicts[0].resolution, Resolution::Open);
    }
}
