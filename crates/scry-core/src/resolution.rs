//! What can be done with each conflict: a stable ID, a classification in
//! plain words, the candidate resolutions that need no judgement, and the one
//! a rule can pick on its own.
//!
//! Everything here is deterministic. An agent later chooses among these
//! candidate IDs, or adds its own candidate, instead of rewriting the file.

use crate::conflict::{Conflict, Segment, conflicts};
use crate::syntax::ParseCheck;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictKind {
    /// Both sides hold the same text.
    SameChange,
    /// Both sides hold the same text except for whitespace.
    SameChangeButWhitespace,
    /// Theirs still holds the base text.
    OnlyOursChanged,
    /// Ours still holds the base text.
    OnlyTheirsChanged,
    /// The base is empty: each side added different lines at the same place.
    BothAdded,
    /// Ours removed the lines that theirs changed.
    OursDeleted,
    /// Theirs removed the lines that ours changed.
    TheirsDeleted,
    /// Both sides changed the same lines differently.
    BothChanged,
    /// The file was written without the base (`merge.conflictStyle=merge`),
    /// so what each side did cannot be told apart.
    NoBase,
}

impl ConflictKind {
    /// The classification in plain words, naming the sides with `ours` and
    /// `theirs` (labels such as "main · rebase target").
    pub fn describe(self, ours: &str, theirs: &str) -> String {
        match self {
            ConflictKind::SameChange => String::from("Same change on both sides"),
            ConflictKind::SameChangeButWhitespace => {
                String::from("Same change on both sides, except for whitespace")
            }
            ConflictKind::OnlyOursChanged => format!("Only {ours} changed these lines"),
            ConflictKind::OnlyTheirsChanged => format!("Only {theirs} changed these lines"),
            ConflictKind::BothAdded => String::from("Both sides added lines here"),
            ConflictKind::OursDeleted => {
                format!("{ours} deleted these lines, {theirs} changed them")
            }
            ConflictKind::TheirsDeleted => {
                format!("{theirs} deleted these lines, {ours} changed them")
            }
            ConflictKind::BothChanged => String::from("Both sides changed these lines"),
            ConflictKind::NoBase => String::from(
                "The sides differ; no base to compare with (set merge.conflictStyle to zdiff3)",
            ),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CandidateKind {
    Ours,
    Theirs,
    OursThenTheirs,
    TheirsThenOurs,
    Neither,
    Base,
    /// Both sides with the lines that follow the conflict repeated after the
    /// first one. With `diff3` and `zdiff3`, git moves the lines both sides
    /// end with (often a closing brace) below the conflict, so plain "both"
    /// leaves the first side unclosed. Offered only when plain "both" does
    /// not parse and this does.
    OursThenTheirsClosed,
    TheirsThenOursClosed,
}

impl CandidateKind {
    /// The candidate's ID within its conflict, as used by the CLI and MCP.
    pub fn id(self) -> &'static str {
        match self {
            CandidateKind::Ours => "ours",
            CandidateKind::Theirs => "theirs",
            CandidateKind::OursThenTheirs => "ours-then-theirs",
            CandidateKind::TheirsThenOurs => "theirs-then-ours",
            CandidateKind::Neither => "neither",
            CandidateKind::Base => "base",
            CandidateKind::OursThenTheirsClosed => "ours-then-theirs-closed",
            CandidateKind::TheirsThenOursClosed => "theirs-then-ours-closed",
        }
    }

    /// The candidate kind with this ID, as typed in the CLI or sent over MCP.
    pub fn from_id(id: &str) -> Option<CandidateKind> {
        [
            CandidateKind::Ours,
            CandidateKind::Theirs,
            CandidateKind::OursThenTheirs,
            CandidateKind::TheirsThenOurs,
            CandidateKind::Neither,
            CandidateKind::Base,
            CandidateKind::OursThenTheirsClosed,
            CandidateKind::TheirsThenOursClosed,
        ]
        .into_iter()
        .find(|kind| kind.id() == id)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub kind: CandidateKind,
    /// The text that replaces the whole conflict, markers included.
    pub text: String,
    /// Whether the file parses with this candidate in place; filled by the
    /// merge session when it knows the language.
    pub parse: ParseCheck,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Analysis {
    /// Derived from the conflict's content, not its position, so it survives
    /// the resolution of other conflicts in the file and goes stale when the
    /// conflict itself changes.
    pub id: String,
    /// Position in the file, from 1.
    pub number: usize,
    pub start_line: usize,
    pub kind: ConflictKind,
    /// Distinct texts only, in a fixed order.
    pub candidates: Vec<Candidate>,
    /// The candidate a rule picks when no judgement is needed. A suggestion
    /// only: applying it is still the user's action.
    pub suggestion: Option<CandidateKind>,
}

/// Analyzes every conflict of a parsed file, in order.
pub fn analyze(segments: &[Segment]) -> Vec<Analysis> {
    let mut analyses: Vec<Analysis> = Vec::new();
    for (index, conflict) in conflicts(segments).enumerate() {
        let content_id = content_id(conflict);
        // Identical conflicts in one file share a content ID: number the
        // repeats.
        let mut id = content_id.clone();
        let mut repeat = 2;
        while analyses.iter().any(|analysis| analysis.id == id) {
            id = format!("{content_id}-{repeat}");
            repeat += 1;
        }
        let kind = classify(conflict);
        analyses.push(Analysis {
            id,
            number: index + 1,
            start_line: conflict.start_line,
            kind,
            candidates: candidates(conflict),
            suggestion: suggestion(kind),
        });
    }
    analyses
}

fn classify(conflict: &Conflict) -> ConflictKind {
    let ours = &conflict.ours.text;
    let theirs = &conflict.theirs.text;
    if ours == theirs {
        return ConflictKind::SameChange;
    }
    let base = conflict.base.as_ref().map(|base| &base.text);
    if base == Some(theirs) {
        return ConflictKind::OnlyOursChanged;
    }
    if base == Some(ours) {
        return ConflictKind::OnlyTheirsChanged;
    }
    if without_whitespace(ours) == without_whitespace(theirs) {
        return ConflictKind::SameChangeButWhitespace;
    }
    let Some(base) = base else {
        return ConflictKind::NoBase;
    };
    if base.is_empty() {
        ConflictKind::BothAdded
    } else if ours.is_empty() {
        ConflictKind::OursDeleted
    } else if theirs.is_empty() {
        ConflictKind::TheirsDeleted
    } else {
        ConflictKind::BothChanged
    }
}

fn candidates(conflict: &Conflict) -> Vec<Candidate> {
    let ours = &conflict.ours.text;
    let theirs = &conflict.theirs.text;
    let mut texts = vec![
        (CandidateKind::Ours, ours.clone()),
        (CandidateKind::Theirs, theirs.clone()),
        (CandidateKind::OursThenTheirs, format!("{ours}{theirs}")),
        (CandidateKind::TheirsThenOurs, format!("{theirs}{ours}")),
        (CandidateKind::Neither, String::new()),
    ];
    // An empty base would only repeat "neither".
    if let Some(base) = &conflict.base
        && !base.text.is_empty()
    {
        texts.push((CandidateKind::Base, base.text.clone()));
    }

    let mut candidates: Vec<Candidate> = Vec::new();
    for (kind, text) in texts {
        if !candidates.iter().any(|candidate| candidate.text == text) {
            candidates.push(Candidate {
                kind,
                text,
                parse: ParseCheck::NotChecked,
            });
        }
    }
    candidates
}

fn suggestion(kind: ConflictKind) -> Option<CandidateKind> {
    match kind {
        ConflictKind::SameChange | ConflictKind::OnlyOursChanged => Some(CandidateKind::Ours),
        ConflictKind::OnlyTheirsChanged => Some(CandidateKind::Theirs),
        _ => None,
    }
}

/// The lines without blank lines, each with its whitespace runs collapsed.
fn without_whitespace(text: &str) -> Vec<String> {
    text.lines()
        .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|line| !line.is_empty())
        .collect()
}

/// "c" and six hex digits of a 64-bit FNV-1a hash of the three sides. Written
/// out rather than taken from the standard library, whose hash may change
/// between Rust versions: IDs must stay the same across builds.
fn content_id(conflict: &Conflict) -> String {
    let base = conflict.base.as_ref().map_or("", |base| base.text.as_str());
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for part in [
        conflict.ours.text.as_str(),
        base,
        conflict.theirs.text.as_str(),
    ] {
        // 0xff never occurs in UTF-8, so it separates the parts unambiguously.
        for byte in part.bytes().chain([0xff]) {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0100_0000_01b3);
        }
    }
    format!("c{:06x}", hash >> 40)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conflict::parse;

    fn conflict(ours: &str, base: Option<&str>, theirs: &str) -> String {
        let base = match base {
            Some(text) => format!("||||||| base\n{text}"),
            None => String::new(),
        };
        format!("<<<<<<< ours\n{ours}{base}=======\n{theirs}>>>>>>> theirs\n")
    }

    fn analyze_one(ours: &str, base: Option<&str>, theirs: &str) -> Analysis {
        let segments = parse(&conflict(ours, base, theirs)).unwrap();
        analyze(&segments).remove(0)
    }

    fn candidate_ids(analysis: &Analysis) -> Vec<&'static str> {
        analysis.candidates.iter().map(|c| c.kind.id()).collect()
    }

    /// Every classification is decided from the three texts alone, with no judgement.
    #[test]
    fn classifications() {
        let cases = [
            ("a\n", Some("x\n"), "a\n", ConflictKind::SameChange),
            (
                "a\n",
                Some("x\n"),
                "  a\n",
                ConflictKind::SameChangeButWhitespace,
            ),
            ("a\n", Some("x\n"), "x\n", ConflictKind::OnlyOursChanged),
            ("x\n", Some("x\n"), "b\n", ConflictKind::OnlyTheirsChanged),
            ("a\n", Some(""), "b\n", ConflictKind::BothAdded),
            ("", Some("x\n"), "b\n", ConflictKind::OursDeleted),
            ("a\n", Some("x\n"), "", ConflictKind::TheirsDeleted),
            ("a\n", Some("x\n"), "b\n", ConflictKind::BothChanged),
            ("a\n", None, "b\n", ConflictKind::NoBase),
        ];
        for (ours, base, theirs, kind) in cases {
            assert_eq!(
                analyze_one(ours, base, theirs).kind,
                kind,
                "{ours:?} {base:?} {theirs:?}"
            );
        }
    }

    /// A rule suggests a candidate only when the answer is certain (same change, or one side
    /// unchanged); a whitespace-only difference is left to the user.
    #[test]
    fn rules_suggest_only_when_no_judgement_is_needed() {
        assert_eq!(
            analyze_one("a\n", Some("x\n"), "x\n").suggestion,
            Some(CandidateKind::Ours)
        );
        assert_eq!(
            analyze_one("x\n", Some("x\n"), "b\n").suggestion,
            Some(CandidateKind::Theirs)
        );
        assert_eq!(
            analyze_one("a\n", Some("x\n"), "a\n").suggestion,
            Some(CandidateKind::Ours)
        );
        assert_eq!(analyze_one("a\n", Some("x\n"), "b\n").suggestion, None);
        assert_eq!(analyze_one("a\n", Some("x\n"), "  a\n").suggestion, None);
    }

    /// Candidates come in a fixed order, with their exact text, so every UI lists the same
    /// thing.
    #[test]
    fn candidates_in_order_with_their_text() {
        let analysis = analyze_one("a\n", Some("x\n"), "b\n");
        assert_eq!(
            candidate_ids(&analysis),
            [
                "ours",
                "theirs",
                "ours-then-theirs",
                "theirs-then-ours",
                "neither",
                "base"
            ]
        );
        let texts: Vec<&str> = analysis
            .candidates
            .iter()
            .map(|c| c.text.as_str())
            .collect();
        assert_eq!(texts, ["a\n", "b\n", "a\nb\n", "b\na\n", "", "x\n"]);
    }

    /// Candidates with the same text are offered once, and no base candidate exists without a
    /// base.
    #[test]
    fn duplicate_candidates_are_dropped() {
        // Ours is empty: "ours-then-theirs" equals "theirs", "neither" equals "ours".
        let analysis = analyze_one("", Some("x\n"), "b\n");
        assert_eq!(candidate_ids(&analysis), ["ours", "theirs", "base"]);
        // Empty base and no base: no "base" candidate.
        assert!(!candidate_ids(&analyze_one("a\n", Some(""), "b\n")).contains(&"base"));
        assert!(!candidate_ids(&analyze_one("a\n", None, "b\n")).contains(&"base"));
    }

    /// A conflict's ID comes from its content, so it stays the same when other conflicts are
    /// resolved and the numbers shift.
    #[test]
    fn ids_follow_content_not_position() {
        let first = conflict("a\n", Some("x\n"), "b\n");
        let second = conflict("c\n", Some("y\n"), "d\n");
        let both = parse(&format!("{first}text\n{second}")).unwrap();
        let only_second = parse(&format!("resolved\ntext\n{second}")).unwrap();

        let both = analyze(&both);
        let only_second = analyze(&only_second);
        assert_ne!(both[0].id, both[1].id);
        assert_eq!(both[1].id, only_second[0].id);
        assert_eq!(both[1].number, 2);
        assert_eq!(only_second[0].number, 1);
        assert!(both[0].id.starts_with('c') && both[0].id.len() == 7);
    }

    /// IDs ignore the marker labels, which change when a file is rebuilt, and must never
    /// change between builds or Rust versions.
    #[test]
    fn ids_ignore_labels_and_survive_rebuilds() {
        let text = "<<<<<<< HEAD\na\n||||||| base\nx\n=======\nb\n>>>>>>> 1234567 (Other label)\n";
        let analysis = analyze(&parse(text).unwrap()).remove(0);
        assert_eq!(analysis.id, analyze_one("a\n", Some("x\n"), "b\n").id);
        // A fixed value, computed independently: IDs must not change between
        // builds or Rust versions.
        assert_eq!(analysis.id, "c660efd");
    }

    /// Two identical conflicts in a file still get distinct IDs.
    #[test]
    fn identical_conflicts_get_numbered_ids() {
        let one = conflict("a\n", Some("x\n"), "b\n");
        let analyses = analyze(&parse(&format!("{one}text\n{one}")).unwrap());
        assert_eq!(analyses[1].id, format!("{}-2", analyses[0].id));
    }

    /// Classification sentences use the human side labels, not git's ours and theirs.
    #[test]
    fn descriptions_name_the_sides() {
        assert_eq!(
            ConflictKind::OursDeleted.describe("main · rebase target", "Your commit a1b2c3d"),
            "main · rebase target deleted these lines, Your commit a1b2c3d changed them"
        );
    }
}
