//! Xcode project files (`project.pbxproj`): most conflicts are both sides
//! adding or removing entries in the same list or section (a file each, a
//! build phase each), where the answer is to keep every addition and every
//! removal. This module says, conflict by conflict, what each side adds and
//! removes, settles the conflicts where that is safe, leaves the others to
//! the merge editor with the reason, and checks the result: every object
//! the file refers to must still be defined.

use crate::conflict::{self, Conflict, ParseError, Segment};

/// What one conflict of the project file does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectConflict {
    /// Entries by name ("OfflineQueue.swift in Sources", or a setting's
    /// line).
    pub ours_adds: Vec<String>,
    pub theirs_adds: Vec<String>,
    /// Entries of the base a side removed; empty when the conflict has no
    /// base to compare with.
    pub ours_removes: Vec<String>,
    pub theirs_removes: Vec<String>,
    /// Entries a side changed: "MARKETING_VERSION 1.3 → 1.4".
    pub ours_changes: Vec<String>,
    pub theirs_changes: Vec<String>,
    /// Whether keeping both sides' changes is safe here.
    pub keeps_both: bool,
    /// Why not, when it is not.
    pub reason: Option<String>,
}

/// The project file's conflicts, and what settling the safe ones gives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectReview {
    pub conflicts: Vec<ProjectConflict>,
    /// How many conflicts keeping both settles.
    pub resolvable: usize,
    /// Objects the result refers to without defining them, outside the
    /// conflicts left ("A2000002 ForecastView.swift"): git merged those
    /// lines without a conflict, yet the project would be broken.
    pub missing: Vec<String>,
    /// Every missing object is only listed (`<id> /* name */,` in a group
    /// or a build phase), so removing those lines repairs the project.
    pub missing_removable: bool,
}

impl ProjectReview {
    pub fn can_keep_both(&self) -> bool {
        self.resolvable > 0
    }
}

/// Reviews a project file with conflict markers.
pub fn review(text: &str, marker_size: usize) -> Result<ProjectReview, ParseError> {
    let segments = conflict::parse_with_marker_size(text, marker_size)?;
    let plan = settle(&segments, marker_size);
    let conflicts: Vec<ProjectConflict> = conflict::conflicts(&segments)
        .zip(&plan.reasons)
        .map(|(conflict, reason)| {
            let changes = Changes::of(conflict);
            let (ours_adds, ours_removes, ours_changes) =
                described(&changes.ours_adds, &changes.ours_removes);
            let (theirs_adds, theirs_removes, theirs_changes) =
                described(&changes.theirs_adds, &changes.theirs_removes);
            ProjectConflict {
                ours_adds,
                theirs_adds,
                ours_removes,
                theirs_removes,
                ours_changes,
                theirs_changes,
                keeps_both: reason.is_none(),
                reason: reason.clone(),
            }
        })
        .collect();
    let ids: Vec<&str> = plan.missing_ids.iter().map(String::as_str).collect();
    let missing_removable = !ids.is_empty()
        && plan
            .text
            .lines()
            .filter(|line| self::ids(line).iter().any(|id| ids.contains(id)))
            .all(|line| listed(line).is_some_and(|id| ids.contains(&id)));
    Ok(ProjectReview {
        missing_removable,
        resolvable: conflicts
            .iter()
            .filter(|conflict| conflict.keeps_both)
            .count(),
        conflicts,
        missing: plan.missing,
    })
}

/// The file with every safe conflict settled: ours, without what theirs
/// removed, then what theirs added. The other conflicts keep their
/// markers, for the merge editor. With `remove_missing`, the list entries
/// of objects no longer defined are removed too.
pub fn keep_both(
    text: &str,
    marker_size: usize,
    remove_missing: bool,
) -> Result<String, KeepBothError> {
    let segments =
        conflict::parse_with_marker_size(text, marker_size).map_err(KeepBothError::Parse)?;
    let plan = settle(&segments, marker_size);
    let result: String = if !remove_missing || plan.missing_ids.is_empty() {
        plan.text
    } else {
        plan.text
            .split_inclusive('\n')
            .filter(|line| {
                listed(line.trim_end_matches(['\n', '\r']))
                    .is_none_or(|id| !plan.missing_ids.iter().any(|missing| missing == id))
            })
            .collect()
    };
    // The last guard: a project with every conflict settled must close
    // each `{` and `(` it opens, or Xcode cannot open it.
    let settled_all = plan.reasons.iter().all(Option::is_none);
    if settled_all && !balanced(&result) {
        return Err(KeepBothError::Unbalanced);
    }
    Ok(result)
}

/// Why keeping both was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeepBothError {
    Parse(ParseError),
    /// The result would not close every bracket it opens.
    Unbalanced,
}

/// The id of a line that only lists an object (`<id> /* name */,`).
fn listed(line: &str) -> Option<&str> {
    let trimmed = line.trim();
    let id = key(trimmed);
    let rest = trimmed[id.len()..].trim_start();
    (ids(trimmed) == [id] && rest.starts_with("/*") && rest.ends_with("*/,")).then_some(id)
}

/// Each side's changes in a conflict, against the base when there is one.
struct Changes<'a> {
    ours: Vec<&'a str>,
    ours_adds: Vec<&'a str>,
    theirs_adds: Vec<&'a str>,
    ours_removes: Vec<&'a str>,
    theirs_removes: Vec<&'a str>,
}

impl<'a> Changes<'a> {
    fn of(conflict: &'a Conflict) -> Changes<'a> {
        let ours = lines(&conflict.ours.text);
        let theirs = lines(&conflict.theirs.text);
        let entries = |lines: &[&'a str]| -> Vec<&'a str> {
            lines
                .iter()
                .copied()
                .filter(|line| !is_structure(line))
                .collect()
        };
        match &conflict.base {
            Some(base) => {
                let base = lines(&base.text);
                let only = |these: &[&'a str], those: &[&'a str]| -> Vec<&'a str> {
                    entries(these)
                        .into_iter()
                        .filter(|line| !those.contains(line))
                        .collect()
                };
                Changes {
                    ours_adds: only(&ours, &base),
                    theirs_adds: only(&theirs, &base),
                    ours_removes: only(&base, &ours),
                    theirs_removes: only(&base, &theirs),
                    ours,
                }
            }
            // Without a base, a line only one side has is taken for an
            // addition: removals cannot be told apart.
            None => {
                let only = |these: &[&'a str], those: &[&'a str]| -> Vec<&'a str> {
                    entries(these)
                        .into_iter()
                        .filter(|line| !those.contains(line))
                        .collect()
                };
                Changes {
                    ours_adds: only(&ours, &theirs),
                    theirs_adds: only(&theirs, &ours),
                    ours_removes: Vec::new(),
                    theirs_removes: Vec::new(),
                    ours,
                }
            }
        }
    }

    /// Why keeping both would contradict itself: an entry with a different
    /// line on each side, or changed on one side and removed on the other.
    fn contradiction(&self) -> Option<String> {
        for ours in &self.ours_adds {
            for theirs in &self.theirs_adds {
                if key(ours) == key(theirs) && ours != theirs {
                    return Some(format!(
                        "{}: {} on one side, {} on the other",
                        label(ours),
                        value(ours),
                        value(theirs)
                    ));
                }
            }
        }
        for (added, removed) in [
            (&self.ours_adds, &self.theirs_removes),
            (&self.theirs_adds, &self.ours_removes),
        ] {
            for line in added {
                if removed.iter().any(|gone| key(gone) == key(line)) {
                    return Some(format!(
                        "{} is changed on one side and removed on the other",
                        label(line)
                    ));
                }
            }
        }
        None
    }

    /// Ours without what theirs removed, then what theirs added.
    fn merged(&self) -> Vec<&'a str> {
        let mut merged: Vec<&str> = self
            .ours
            .iter()
            .copied()
            .filter(|line| !self.theirs_removes.contains(line))
            .collect();
        for line in &self.theirs_adds {
            if !merged.contains(line) {
                merged.push(line);
            }
        }
        merged
    }
}

/// The result of settling the safe conflicts.
struct Settled {
    text: String,
    /// Per conflict, why it is left; `None` when settled.
    reasons: Vec<Option<String>>,
    missing: Vec<String>,
    missing_ids: Vec<String>,
}

fn settle(segments: &[Segment], marker_size: usize) -> Settled {
    let names = object_names(segments);
    let mut reasons: Vec<Option<String>> = conflict::conflicts(segments)
        .map(|conflict| shape(conflict).or_else(|| Changes::of(conflict).contradiction()))
        .collect();
    loop {
        let text = write(segments, &reasons, marker_size);
        let defined = defined(&text);
        let mut changed = false;
        // A settled conflict whose lines refer to an object the result no
        // longer defines is left to the user.
        for (index, conflict) in conflict::conflicts(segments).enumerate() {
            if reasons[index].is_some() {
                continue;
            }
            let changes = Changes::of(conflict);
            let undefined = changes
                .merged()
                .iter()
                .flat_map(|line| ids(line))
                .find(|id| !defined.contains(id));
            if let Some(id) = undefined {
                reasons[index] = Some(format!(
                    "refers to {}, which the result no longer defines",
                    named(id, &names)
                ));
                changed = true;
            }
        }
        if !changed {
            let left: Vec<&str> = conflict::conflicts(segments)
                .zip(&reasons)
                .filter(|(_, reason)| reason.is_some())
                .flat_map(|(conflict, _)| {
                    lines(&conflict.ours.text)
                        .into_iter()
                        .chain(lines(&conflict.theirs.text))
                        .flat_map(ids)
                        .collect::<Vec<_>>()
                })
                .collect();
            let mut missing: Vec<String> = Vec::new();
            let mut missing_ids: Vec<String> = Vec::new();
            for id in text.lines().flat_map(ids) {
                if !defined.contains(&id)
                    && !left.contains(&id)
                    && !missing_ids.iter().any(|known| known == id)
                {
                    missing.push(named(id, &names));
                    missing_ids.push(id.to_string());
                }
            }
            return Settled {
                text,
                reasons,
                missing,
                missing_ids,
            };
        }
    }
}

/// The file with the conflicts without a reason settled, the others as
/// git wrote them.
fn write(segments: &[Segment], reasons: &[Option<String>], marker_size: usize) -> String {
    let mut text = String::new();
    let mut index = 0;
    for segment in segments {
        match segment {
            Segment::Common(common) => text.push_str(common),
            Segment::Conflict(conflict) => {
                if reasons[index].is_none() {
                    let ending = if conflict.ours.text.contains("\r\n") {
                        "\r\n"
                    } else {
                        "\n"
                    };
                    for line in Changes::of(conflict).merged() {
                        text.push_str(line);
                        text.push_str(ending);
                    }
                } else {
                    text.push_str(&markers(conflict, marker_size));
                }
                index += 1;
            }
        }
    }
    text
}

/// A conflict as git writes it.
fn markers(conflict: &Conflict, size: usize) -> String {
    let marker = |character: char, label: &str| {
        let mut line = character.to_string().repeat(size);
        if !label.is_empty() {
            line.push(' ');
            line.push_str(label);
        }
        line.push('\n');
        line
    };
    let mut text = marker('<', &conflict.ours.label);
    text.push_str(&conflict.ours.text);
    if let Some(base) = &conflict.base {
        text.push_str(&marker('|', &base.label));
        text.push_str(&base.text);
    }
    text.push_str(&marker('=', ""));
    text.push_str(&conflict.theirs.text);
    text.push_str(&marker('>', &conflict.theirs.label));
    text
}

fn lines(text: &str) -> Vec<&str> {
    text.split_inclusive('\n')
        .map(|line| line.trim_end_matches(['\n', '\r']))
        .collect()
}

/// Why a conflict is not made of single-line entries, which is all that
/// keeping both handles: a side that opens or closes an object (`{ … };`)
/// or a list (`( … );`) inside the conflict is left to the merge editor,
/// since merging its lines one by one would split the object.
fn shape(conflict: &Conflict) -> Option<String> {
    let base = conflict.base.as_ref().map(|base| base.text.as_str());
    let spans = [
        Some(conflict.ours.text.as_str()),
        base,
        Some(conflict.theirs.text.as_str()),
    ]
    .into_iter()
    .flatten()
    .flat_map(lines)
    .filter(|line| !line.trim().is_empty())
    .any(|line| is_structure(line) || !balanced(line));
    spans.then(|| {
        String::from("opens or closes an object or a list across several lines: resolve it by hand")
    })
}

/// Every `{` and `(` closed, never closed before it opens, outside quoted
/// strings and `/* … */` comments (a build script's text can hold any
/// bracket).
fn balanced(text: &str) -> bool {
    let (mut braces, mut parens) = (0i64, 0i64);
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                while let Some(c) = chars.next() {
                    match c {
                        '\\' => {
                            chars.next();
                        }
                        '"' => break,
                        _ => {}
                    }
                }
            }
            '/' if chars.peek() == Some(&'*') => {
                chars.next();
                let mut previous = ' ';
                for c in chars.by_ref() {
                    if previous == '*' && c == '/' {
                        break;
                    }
                    previous = c;
                }
            }
            '{' => braces += 1,
            '}' => braces -= 1,
            '(' => parens += 1,
            ')' => parens -= 1,
            _ => {}
        }
        if braces < 0 || parens < 0 {
            return false;
        }
    }
    braces == 0 && parens == 0
}

/// Closing brackets and blank lines belong to no entry.
fn is_structure(line: &str) -> bool {
    matches!(line.trim(), "" | ");" | "};" | "}" | ")" | "(" | "{")
}

/// What a line defines or lists: the object's id
/// (`A1B2… /* Name */ = {…};`, `A1B2… /* Name */,`), or the setting's
/// name (`MARKETING_VERSION = 1.2;`).
fn key(line: &str) -> &str {
    let line = line.trim();
    let end = [" /*", " =", ","]
        .iter()
        .filter_map(|separator| line.find(separator))
        .min()
        .unwrap_or(line.len());
    &line[..end]
}

/// An entry as people read it: its comment ("OfflineQueue.swift in
/// Sources"), or else the setting's line.
fn entry_name(line: &str) -> String {
    let line = line.trim();
    if let (Some(start), Some(end)) = (line.find("/* "), line.find(" */"))
        && start < end
    {
        return line[start + 3..end].to_string();
    }
    line.trim_end_matches([';', ',']).to_string()
}

/// A side's additions and removals by name, with the entries it changed
/// (added and removed under the same key) as "KEY old → new".
fn described(adds: &[&str], removes: &[&str]) -> (Vec<String>, Vec<String>, Vec<String>) {
    let shares_key =
        |line: &str, others: &[&str]| others.iter().any(|other| key(other) == key(line));
    let changes = adds
        .iter()
        .filter_map(|added| {
            removes
                .iter()
                .find(|removed| key(removed) == key(added))
                .map(|removed| format!("{} {} → {}", label(added), value(removed), value(added)))
        })
        .collect();
    let only_added = adds
        .iter()
        .filter(|line| !shares_key(line, removes))
        .map(|line| entry_name(line))
        .collect();
    let only_removed = removes
        .iter()
        .filter(|line| !shares_key(line, adds))
        .map(|line| entry_name(line))
        .collect();
    (only_added, only_removed, changes)
}

/// What an entry is called: its comment for an object, its name for a
/// setting.
fn label(line: &str) -> String {
    let trimmed = line.trim();
    if trimmed.contains("/* ") {
        entry_name(trimmed)
    } else {
        key(trimmed).to_string()
    }
}

/// A setting's value ("1.4"), or the entry's name.
fn value(line: &str) -> String {
    let trimmed = line.trim();
    match trimmed.split_once(" = ") {
        Some((_, value)) if !trimmed.contains("/* ") => value.trim_end_matches(';').to_string(),
        _ => entry_name(trimmed),
    }
}

/// Every object id with the name its comments give it.
fn object_names(segments: &[Segment]) -> Vec<(String, String)> {
    let mut names: Vec<(String, String)> = Vec::new();
    let mut add = |text: &str| {
        for line in text.lines() {
            for id in ids(line) {
                if names.iter().any(|(known, _)| known == id) {
                    continue;
                }
                let after = &line[line.find(id).unwrap_or(0) + id.len()..];
                if let (Some(start), Some(end)) = (after.find("/* "), after.find(" */"))
                    && start < end
                {
                    names.push((id.to_string(), after[start + 3..end].to_string()));
                }
            }
        }
    };
    for segment in segments {
        match segment {
            Segment::Common(text) => add(text),
            Segment::Conflict(conflict) => {
                add(&conflict.ours.text);
                add(&conflict.theirs.text);
                if let Some(base) = &conflict.base {
                    add(&base.text);
                }
            }
        }
    }
    names
}

/// "A2000002 ForecastView.swift".
fn named(id: &str, names: &[(String, String)]) -> String {
    match names.iter().find(|(known, _)| known == id) {
        Some((_, name)) => format!("{id} {name}"),
        None => id.to_string(),
    }
}

/// The object ids the text defines (`<id> /* … */ = {`).
fn defined(text: &str) -> Vec<&str> {
    text.lines()
        .filter(|line| line.contains("= {"))
        .map(key)
        .collect()
}

/// The object ids a line mentions: 8 to 24 hexadecimal characters, each
/// followed by its ` /* name */` comment, as Xcode writes them.
fn ids(line: &str) -> Vec<&str> {
    line.split(|character: char| !character.is_ascii_alphanumeric())
        .filter(|word| {
            (8..=24).contains(&word.len())
                && word.chars().all(|character| character.is_ascii_hexdigit())
                && word.chars().any(|character| character.is_ascii_digit())
        })
        .filter(|word| {
            line.find(word)
                .is_some_and(|at| line[at + word.len()..].starts_with(" /*"))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const BOTH_ADD: &str = "\
/* Begin PBXBuildFile section */
\t\tA1000001 /* SyncEngine.swift in Sources */ = {isa = PBXBuildFile; fileRef = A2000001 /* SyncEngine.swift */; };
<<<<<<< ours
\t\tA1000003 /* OfflineQueue.swift in Sources */ = {isa = PBXBuildFile; fileRef = A2000003 /* OfflineQueue.swift */; };
=======
\t\tA1000004 /* ForecastCache.swift in Sources */ = {isa = PBXBuildFile; fileRef = A2000004 /* ForecastCache.swift */; };
>>>>>>> theirs
/* End PBXBuildFile section */
\t\tA2000001 /* SyncEngine.swift */ = {isa = PBXFileReference; path = SyncEngine.swift; };
<<<<<<< ours
\t\tA2000003 /* OfflineQueue.swift */ = {isa = PBXFileReference; path = OfflineQueue.swift; };
=======
\t\tA2000004 /* ForecastCache.swift */ = {isa = PBXFileReference; path = ForecastCache.swift; };
>>>>>>> theirs
";

    /// Each side adding its own file is the usual project conflict: both
    /// additions are named, both conflicts settle, and the result keeps
    /// every line once, without markers.
    #[test]
    fn both_sides_adding_files_keep_both() {
        let review = review(BOTH_ADD, 7).unwrap();
        assert_eq!(review.conflicts.len(), 2);
        assert_eq!(
            review.conflicts[0].ours_adds,
            ["OfflineQueue.swift in Sources"]
        );
        assert_eq!(
            review.conflicts[0].theirs_adds,
            ["ForecastCache.swift in Sources"]
        );
        assert_eq!(review.resolvable, 2);
        assert!(review.missing.is_empty());

        let result = keep_both(BOTH_ADD, 7, true).unwrap();
        assert!(!result.contains("<<<<<<<"));
        assert!(result.contains("A1000003 /* OfflineQueue.swift in Sources */"));
        assert!(result.contains("A1000004 /* ForecastCache.swift in Sources */"));
        assert_eq!(
            result
                .matches("A2000004 /* ForecastCache.swift */ = {")
                .count(),
            1
        );
    }

    /// Each side adding a multi-line object at the same place (a build
    /// phase per target) used to be "kept both" line by line, which split
    /// the objects and broke the project: it is now left by hand.
    #[test]
    fn a_multi_line_object_added_on_each_side_is_left_by_hand() {
        let text = "\
/* Begin PBXFrameworksBuildPhase section */
<<<<<<< ours
\t\tB1000001 /* Frameworks */ = {
\t\t\tisa = PBXFrameworksBuildPhase;
\t\t\tfiles = (
\t\t\t);
\t\t};
||||||| base
=======
\t\tB2000002 /* Frameworks */ = {
\t\t\tisa = PBXFrameworksBuildPhase;
\t\t\tfiles = (
\t\t\t);
\t\t};
>>>>>>> theirs
/* End PBXFrameworksBuildPhase section */
";
        let review = review(text, 7).unwrap();
        assert_eq!(review.resolvable, 0);
        assert!(
            review.conflicts[0]
                .reason
                .as_deref()
                .unwrap()
                .contains("across several lines")
        );
        // Nothing settles, so the file keeps its markers as git wrote it.
        assert!(keep_both(text, 7, true).unwrap().contains("<<<<<<< ours"));
    }

    /// A side whose conflict runs past the end of a list (`);`) used to get
    /// the other side's entries after the bracket, among the group's
    /// attributes: it is now left by hand.
    #[test]
    fn a_conflict_past_the_end_of_a_list_is_left_by_hand() {
        let text = "\
\t\t\tchildren = (
\t\t\t\tA2000001 /* One.swift */,
<<<<<<< ours
\t\t\t\tA2000003 /* Ours.swift */,
\t\t\t);
\t\t\tpath = Sources;
||||||| base
\t\t\t);
\t\t\tpath = Source;
=======
\t\t\t\tA2000004 /* Theirs.swift */,
\t\t\t);
\t\t\tpath = Source;
>>>>>>> theirs
\t\t\tsourceTree = \"<group>\";
";
        let review = review(text, 7).unwrap();
        assert_eq!(review.resolvable, 0);
        assert!(!review.can_keep_both());
    }

    /// Brackets inside strings and comments do not count, and a closing
    /// bracket before its opening one is not balanced.
    #[test]
    fn balance_ignores_strings_and_comments() {
        assert!(balanced(
            "A1 /* main) */ = {isa = X; shellScript = \"if [ x ]; then { echo; \\\"(\\\"\";};"
        ));
        assert!(!balanced("\t\t};"));
        assert!(!balanced("files = ("));
        assert!(!balanced("}{"));
    }

    /// With the base, an entry one side removed stays removed while the
    /// other side's addition is kept: keeping both never brings a deleted
    /// file back.
    #[test]
    fn a_removal_beside_an_addition_stays_removed() {
        let text = "\
\t\t\tchildren = (
<<<<<<< ours
\t\t\t\tA2000002 /* ForecastView.swift */,
\t\t\t\tA2000003 /* OfflineQueue.swift */,
||||||| base
\t\t\t\tA2000002 /* ForecastView.swift */,
=======
\t\t\t\tA2000004 /* ForecastCache.swift */,
>>>>>>> theirs
\t\t\t);
\t\tA2000003 /* OfflineQueue.swift */ = {isa = PBXFileReference; };
\t\tA2000004 /* ForecastCache.swift */ = {isa = PBXFileReference; };
";
        let review = review(text, 7).unwrap();
        assert_eq!(review.conflicts[0].theirs_removes, ["ForecastView.swift"]);
        assert_eq!(review.conflicts[0].ours_adds, ["OfflineQueue.swift"]);
        assert_eq!(review.resolvable, 1);
        let result = keep_both(text, 7, true).unwrap();
        assert!(!result.contains("ForecastView"));
        assert!(
            result
                .contains("OfflineQueue.swift */,\n\t\t\t\tA2000004 /* ForecastCache.swift */,\n")
        );
    }

    /// A setting changed differently on each side is left to the editor,
    /// with markers and the reason, while the other conflicts settle.
    #[test]
    fn a_setting_changed_on_both_sides_is_left_with_its_reason() {
        let text = "\
<<<<<<< ours
\t\t\t\tMARKETING_VERSION = 1.4;
||||||| base
\t\t\t\tMARKETING_VERSION = 1.3;
=======
\t\t\t\tMARKETING_VERSION = 2.0;
>>>>>>> theirs
<<<<<<< ours
\t\tA2000003 /* OfflineQueue.swift */ = {isa = PBXFileReference; };
=======
\t\tA2000004 /* ForecastCache.swift */ = {isa = PBXFileReference; };
>>>>>>> theirs
";
        let review = review(text, 7).unwrap();
        assert_eq!(
            review.conflicts[0].reason.as_deref(),
            Some("MARKETING_VERSION: 1.4 on one side, 2.0 on the other")
        );
        assert_eq!(
            review.conflicts[0].ours_changes,
            ["MARKETING_VERSION 1.3 → 1.4"]
        );
        assert!(review.conflicts[0].ours_adds.is_empty());
        assert!(review.conflicts[0].theirs_removes.is_empty());
        assert_eq!(review.resolvable, 1);
        let result = keep_both(text, 7, true).unwrap();
        assert!(
            result.starts_with("<<<<<<< ours\n\t\t\t\tMARKETING_VERSION = 1.4;\n||||||| base\n")
        );
        assert!(
            result.ends_with("A2000004 /* ForecastCache.swift */ = {isa = PBXFileReference; };\n")
        );
    }

    /// An entry one side lists while the other removed its definition is
    /// left with the reason; one git merged without a conflict is reported
    /// as missing, named.
    #[test]
    fn references_to_removed_objects_are_caught() {
        let text = "\
<<<<<<< ours
\t\tA2000002 /* ForecastView.swift */ = {isa = PBXFileReference; };
\t\tA2000003 /* OfflineQueue.swift */ = {isa = PBXFileReference; };
||||||| base
\t\tA2000002 /* ForecastView.swift */ = {isa = PBXFileReference; };
=======
>>>>>>> theirs
\t\tA3000002 /* Widgets */ = {
\t\t\tchildren = (
\t\t\t\tA2000002 /* ForecastView.swift */,
\t\t\t);
\t\t};
";
        let review = review(text, 7).unwrap();
        assert_eq!(review.resolvable, 1);
        assert_eq!(review.missing, ["A2000002 ForecastView.swift"]);
        assert!(review.missing_removable);

        let kept = keep_both(text, 7, false).unwrap();
        assert!(kept.contains("\t\t\t\tA2000002 /* ForecastView.swift */,\n"));
        let repaired = keep_both(text, 7, true).unwrap();
        assert!(!repaired.contains("ForecastView"));
        assert!(repaired.contains("children = (\n\t\t\t);"));
    }
}
