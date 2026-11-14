//! What a merge looks like in words: candidate titles and subtitles, where
//! each resolution comes from, previews, and the panes' headers.
//!
//! One source for every UI (portability rule 1), so the CLI, the Mac app
//! and the MCP server say the same thing; they only lay it out.

use serde::Serialize;
use std::path::Path;

use crate::operation::{Operation, Side, SideRole};
use crate::resolution::{Analysis, CandidateKind};
use crate::session::{Choice, MergeSession};
use crate::syntax::ParseCheck;
use crate::theme::{Badge, Provenance, Role, Severity, State};

/// The ID of a conflict's text written by hand, beside the candidates'.
pub const EDITED_ID: &str = "edited";

/// How many lines of a candidate a preview shows.
pub const PREVIEW_LINES: usize = 3;

impl CandidateKind {
    /// The candidate's name, as buttons, cards and badges show it.
    pub fn title(self) -> &'static str {
        match self {
            CandidateKind::Ours => "Ours",
            CandidateKind::Theirs => "Theirs",
            CandidateKind::OursThenTheirs => "Both · ours first",
            CandidateKind::TheirsThenOurs => "Both · theirs first",
            CandidateKind::Neither => "None",
            CandidateKind::Base => "Base",
            CandidateKind::OursThenTheirsClosed => "Both · ours first, each closed",
            CandidateKind::TheirsThenOursClosed => "Both · theirs first, each closed",
        }
    }

    /// One line under the title; a side shows its label ("main + 1
    /// rebased commit").
    pub fn subtitle(self, ours: &str, theirs: &str) -> String {
        match self {
            CandidateKind::Ours => ours.to_string(),
            CandidateKind::Theirs => theirs.to_string(),
            CandidateKind::OursThenTheirs => String::from("Ours, then theirs"),
            CandidateKind::TheirsThenOurs => String::from("Theirs, then ours"),
            CandidateKind::Neither => String::from("Remove the block"),
            CandidateKind::Base => String::from("As before both changes"),
            CandidateKind::OursThenTheirsClosed | CandidateKind::TheirsThenOursClosed => {
                String::from("Repeats the lines git moved below the conflict")
            }
        }
    }

    /// The "closed" variants are made by a rule; the others are git's
    /// sides as they are.
    pub fn provenance(self) -> Provenance {
        match self {
            CandidateKind::OursThenTheirsClosed | CandidateKind::TheirsThenOursClosed => {
                Provenance::Rule
            }
            _ => Provenance::Git,
        }
    }
}

/// A candidate as a card or a menu shows it. Text written by hand is shown
/// the same way, with `EDITED_ID`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CandidateView {
    pub id: String,
    pub title: String,
    pub subtitle: String,
    /// At most `PREVIEW_LINES` lines, without their line breaks.
    pub preview: Vec<String>,
    /// Whether the text has more lines than the preview.
    pub truncated: bool,
    pub parse: ParseCheck,
    pub provenance: Provenance,
    /// Picked by a rule because no judgement is needed.
    pub suggested: bool,
    /// IDs of the kinds that give this same text and choose this candidate
    /// ("theirs" when both sides made the same change).
    pub aliases: Vec<String>,
}

/// A conflict as the inspector or the CLI shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ConflictView {
    pub id: String,
    /// From 1.
    pub number: usize,
    pub start_line: usize,
    pub classification: String,
    pub candidates: Vec<CandidateView>,
    /// The resolution: one of the candidates, or the text written by hand.
    pub chosen: Option<CandidateView>,
    pub state: State,
    /// What the lens row says about the resolution: its title in its
    /// provenance's role, and an error when the file does not parse with
    /// it.
    pub badges: Vec<Badge>,
    /// The text editing by hand starts from: the current resolution, or
    /// ours while unresolved.
    pub edit_text: String,
    /// Whether the candidates are checked by parsing (or the file cannot
    /// be): until then they have no parse result and no closed "both".
    pub checked: bool,
}

/// The first lines of a text, and whether there are more. Any line ending
/// counts: `lines` drops a `\r` before `\n`.
fn preview(text: &str) -> (Vec<String>, bool) {
    let lines: Vec<&str> = text.lines().collect();
    let preview = lines
        .iter()
        .take(PREVIEW_LINES)
        .map(|line| line.to_string())
        .collect();
    (preview, lines.len() > PREVIEW_LINES)
}

impl MergeSession {
    /// Conflict `index` in words, with `ours` and `theirs` the sides'
    /// labels; checks its candidates first if they are not yet.
    pub fn conflict_view(&self, index: usize, ours: &str, theirs: &str) -> ConflictView {
        self.view(self.analysis(index), index, ours, theirs)
    }

    /// Conflict `index` as far as it is checked (`analysis_so_far`): cheap
    /// for a list of every conflict, where the focused one is asked for
    /// with `conflict_view`.
    pub fn conflict_view_so_far(&self, index: usize, ours: &str, theirs: &str) -> ConflictView {
        self.view(self.analysis_so_far(index), index, ours, theirs)
    }

    fn view(&self, analysis: &Analysis, index: usize, ours: &str, theirs: &str) -> ConflictView {
        let candidates: Vec<CandidateView> = analysis
            .candidates
            .iter()
            .map(|candidate| {
                let (preview, truncated) = preview(&candidate.text);
                CandidateView {
                    id: candidate.kind.id().to_string(),
                    title: candidate.kind.title().to_string(),
                    subtitle: candidate.kind.subtitle(ours, theirs),
                    preview,
                    truncated,
                    parse: candidate.parse,
                    provenance: candidate.kind.provenance(),
                    suggested: analysis.suggestion == Some(candidate.kind),
                    aliases: candidate
                        .aliases
                        .iter()
                        .map(|kind| kind.id().to_string())
                        .collect(),
                }
            })
            .collect();
        let chosen = match self.choice(index) {
            None => None,
            Some(Choice::Candidate(kind)) => candidates
                .iter()
                .find(|candidate| candidate.id == kind.id())
                .cloned(),
            Some(Choice::Edited(edit)) => {
                let (preview, truncated) = preview(&edit.text);
                Some(CandidateView {
                    id: EDITED_ID.to_string(),
                    title: String::from("Edited by you"),
                    subtitle: String::from("Written by hand"),
                    preview,
                    truncated,
                    parse: edit.parse,
                    provenance: Provenance::Human,
                    suggested: false,
                    aliases: Vec::new(),
                })
            }
        };
        let (state, badges) = match &chosen {
            None => (
                State::Unresolved,
                vec![Badge::new("Unresolved", Role::State(State::Unresolved))],
            ),
            Some(chosen) => {
                let mut badges = vec![Badge::new(
                    chosen.title.clone(),
                    Role::Provenance(chosen.provenance),
                )];
                if let ParseCheck::DoesNotParse { .. } = chosen.parse {
                    badges.push(Badge::new(
                        "Does not parse",
                        Role::Severity(Severity::Error),
                    ));
                }
                (State::Resolved, badges)
            }
        };
        ConflictView {
            id: analysis.id.clone(),
            number: analysis.number,
            start_line: analysis.start_line,
            classification: analysis.kind.describe(ours, theirs),
            candidates,
            chosen,
            state,
            badges,
            edit_text: self
                .chosen_text(index)
                .unwrap_or(&self.conflict(index).ours.text)
                .to_string(),
            checked: self.is_checked(index),
        }
    }
}

/// One side's pane heading.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PaneSide {
    /// "main + 1 rebased commit · rebase target".
    pub label: String,
    /// One line of context under the label.
    pub detail: String,
}

/// What the editor shows above the panes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Header {
    pub file_name: String,
    pub path: String,
    /// The operation in plain words, when git has one in progress.
    pub summary: Option<String>,
    pub ours: PaneSide,
    pub theirs: PaneSide,
    pub language: Option<String>,
}

/// The sides' labels, for the markers of a rebuilt file and the
/// candidates' subtitles: the operation's ("main + 1 rebased commit"), or
/// git's own words without one.
pub fn side_labels(operation: Option<&Operation>) -> (String, String) {
    match operation {
        Some(operation) => (
            operation.ours.label(),
            operation
                .theirs
                .as_ref()
                .map_or_else(|| String::from("theirs"), |side| side.label()),
        ),
        None => (String::from("ours"), String::from("theirs")),
    }
}

/// The header of the file at `path`, with the sides of the operation git
/// has in progress, or the labels on the markers without one.
pub fn header(path: &Path, session: &MergeSession, operation: Option<&Operation>) -> Header {
    let (summary, ours, theirs) = match operation {
        Some(operation) => {
            let ours = pane_side(&operation.ours, "ours");
            let theirs = match &operation.theirs {
                Some(side) => pane_side(side, "theirs"),
                None => marker_side(session, false),
            };
            (Some(operation.summary()), ours, theirs)
        }
        None => (
            None,
            marker_side(session, true),
            marker_side(session, false),
        ),
    };
    Header {
        file_name: path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default(),
        path: path.display().to_string(),
        summary,
        ours,
        theirs,
        language: session
            .language()
            .map(|language| language.name().to_string()),
    }
}

fn pane_side(side: &Side, git_name: &str) -> PaneSide {
    let git_term = format!("git calls this side \u{201c}{git_name}\u{201d}");
    let detail = match side.role {
        SideRole::CommitBeingRebased | SideRole::CherryPickedCommit | SideRole::RevertedCommit => {
            git_term
        }
        _ => format!("{} · {git_term}", side.commit.short_id),
    };
    PaneSide {
        label: side.label(),
        detail,
    }
}

/// Without an operation: the labels git wrote on the first conflict's
/// markers.
fn marker_side(session: &MergeSession, ours: bool) -> PaneSide {
    let git_name = if ours { "ours" } else { "theirs" };
    let label = match session.conflict_count() {
        0 => String::new(),
        _ if ours => session.conflict(0).ours.label.clone(),
        _ => session.conflict(0).theirs.label.clone(),
    };
    PaneSide {
        label: if label.is_empty() {
            git_name.to_string()
        } else {
            label
        },
        detail: format!("git calls this side \u{201c}{git_name}\u{201d}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::Command;
    use crate::syntax::Language;

    const TEXT: &str =
        "<<<<<<< HEAD\nlet a = 1\n||||||| base\nlet a = 0\n=======\nlet a = 2\n>>>>>>> topic\n";

    /// Every UI shows a candidate with the same title, subtitle, preview
    /// and provenance, because they all come from here.
    #[test]
    fn candidates_are_described_once_for_every_ui() {
        let session = MergeSession::from_text(TEXT, Some(Language::Swift)).unwrap();
        let view = session.conflict_view(0, "main", "topic");

        let theirs = view
            .candidates
            .iter()
            .find(|candidate| candidate.id == "theirs")
            .unwrap();
        assert_eq!(theirs.title, "Theirs");
        assert_eq!(theirs.subtitle, "topic");
        assert_eq!(theirs.preview, vec!["let a = 2"]);
        assert_eq!(theirs.provenance, Provenance::Git);
        assert_eq!(view.chosen, None);
        assert_eq!(view.state, State::Unresolved);
        assert_eq!(
            view.badges,
            vec![Badge::new("Unresolved", Role::State(State::Unresolved))]
        );
        assert_eq!(view.edit_text, "let a = 1\n");
        assert_eq!(
            CandidateKind::TheirsThenOursClosed.provenance(),
            Provenance::Rule
        );
    }

    /// The resolution is shown like a candidate, including text written
    /// by hand, which is the user's, with its own check and a preview
    /// that ignores Windows line endings; its badges say where it comes
    /// from and whether the file still parses.
    #[test]
    fn the_chosen_resolution_says_where_it_comes_from() {
        let mut session = MergeSession::from_text(TEXT, Some(Language::Swift)).unwrap();
        session
            .apply(Command::Choose {
                conflict: 0,
                candidate: CandidateKind::Theirs,
            })
            .unwrap();
        let chosen = session.conflict_view(0, "main", "topic").chosen.unwrap();
        assert_eq!(
            (chosen.id.as_str(), chosen.provenance),
            ("theirs", Provenance::Git)
        );

        session
            .apply(Command::Edit {
                conflict: 0,
                text: String::from("let a = 3\r\nlet b = 4\r\nlet c = 5\r\nlet d = 6\r\n"),
            })
            .unwrap();
        let chosen = session.conflict_view(0, "main", "topic").chosen.unwrap();
        assert_eq!(chosen.id, EDITED_ID);
        assert_eq!(chosen.provenance, Provenance::Human);
        assert_eq!(chosen.parse, ParseCheck::Parses);

        session
            .apply(Command::Edit {
                conflict: 0,
                text: String::from("func f( {\n"),
            })
            .unwrap();
        let view = session.conflict_view(0, "main", "topic");
        assert_eq!(view.state, State::Resolved);
        assert_eq!(
            view.badges,
            vec![
                Badge::new("Edited by you", Role::Provenance(Provenance::Human)),
                Badge::new("Does not parse", Role::Severity(Severity::Error)),
            ]
        );
        session.apply(Command::Undo).unwrap();
        let chosen = session.conflict_view(0, "main", "topic").chosen.unwrap();
        assert_eq!(chosen.preview, vec!["let a = 3", "let b = 4", "let c = 5"]);
        assert!(chosen.truncated);
    }

    /// Without an operation in progress, the panes are named after the
    /// markers' labels, with git's own term under them.
    #[test]
    fn header_without_an_operation_uses_the_markers() {
        let session = MergeSession::from_text(TEXT, Some(Language::Swift)).unwrap();
        let header = header(Path::new("src/A.swift"), &session, None);

        assert_eq!(header.file_name, "A.swift");
        assert_eq!(header.ours.label, "HEAD");
        assert_eq!(header.theirs.label, "topic");
        assert_eq!(
            header.theirs.detail,
            "git calls this side \u{201c}theirs\u{201d}"
        );
        assert_eq!(header.language.as_deref(), Some("Swift"));
        assert_eq!(header.summary, None);
    }
}
