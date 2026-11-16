//! Properties of a merge session that must hold for any file and any
//! sequence of commands, checked on generated ones (proptest): the cases
//! written by hand cover the paths someone thought of, these cover the
//! orders nobody tries, such as an undo after a refused choice or a redo
//! after suggestions.
//!
//! What git defines (the conflicts in a file, what taking a side gives) is
//! checked against git itself in `merge_file_oracle.rs`; what no tool
//! defines, the session's history and its events, is checked here against
//! a model.

use proptest::prelude::*;

use scry_core::command::{Command, Event};
use scry_core::resolution::CandidateKind;
use scry_core::session::{Choice, MergeSession};

/// Lines from a small alphabet, so that sides share lines and the rules
/// find every kind of conflict (same change, one side empty, both added).
fn line() -> impl Strategy<Value = String> {
    prop::sample::select(vec!["a", "b", "c", "", "x y", "  a"]).prop_map(|line| format!("{line}\n"))
}

fn lines(max: usize) -> impl Strategy<Value = String> {
    prop::collection::vec(line(), 0..=max).prop_map(|lines| lines.concat())
}

/// One part of a generated file: common text, or a conflict with its sides
/// and, as `diff3` writes it, maybe its base.
#[derive(Debug, Clone)]
enum Part {
    Common(String),
    Conflict {
        ours: String,
        base: Option<String>,
        theirs: String,
    },
}

fn part() -> impl Strategy<Value = Part> {
    prop_oneof![
        lines(3).prop_map(Part::Common),
        (lines(3), prop::option::of(lines(2)), lines(3))
            .prop_map(|(ours, base, theirs)| Part::Conflict { ours, base, theirs }),
    ]
}

/// A file with git's markers, as `git merge-file` writes them.
fn written(parts: &[Part]) -> String {
    parts
        .iter()
        .map(|part| match part {
            Part::Common(text) => text.clone(),
            Part::Conflict { ours, base, theirs } => {
                let base = base
                    .as_ref()
                    .map(|base| format!("||||||| base\n{base}"))
                    .unwrap_or_default();
                format!("<<<<<<< ours\n{ours}{base}=======\n{theirs}>>>>>>> theirs\n")
            }
        })
        .collect()
}

const KINDS: [CandidateKind; 8] = [
    CandidateKind::Ours,
    CandidateKind::Theirs,
    CandidateKind::OursThenTheirs,
    CandidateKind::TheirsThenOurs,
    CandidateKind::Neither,
    CandidateKind::Base,
    CandidateKind::OursThenTheirsClosed,
    CandidateKind::TheirsThenOursClosed,
];

/// Any command but Save (it writes a file, and changes no choice), aimed
/// at conflicts that exist and one past the last, with candidates a
/// conflict may not offer, so refusals are part of the sequences.
fn command() -> impl Strategy<Value = Command> {
    let conflict = 0..5usize;
    prop_oneof![
        3 => (conflict.clone(), prop::sample::select(KINDS.to_vec()))
            .prop_map(|(conflict, candidate)| Command::Choose { conflict, candidate }),
        1 => (conflict, lines(2)).prop_map(|(conflict, text)| Command::Edit { conflict, text }),
        1 => Just(Command::ApplySuggestions),
        2 => Just(Command::Undo),
        2 => Just(Command::Redo),
    ]
}

fn choices(session: &MergeSession) -> Vec<Option<Choice>> {
    (0..session.conflict_count())
        .map(|index| session.choice(index).cloned())
        .collect()
}

/// The conflicts whose choice differs between two states.
fn changed(before: &[Option<Choice>], after: &[Option<Choice>]) -> Vec<usize> {
    (0..before.len())
        .filter(|&index| before[index] != after[index])
        .collect()
}

/// The conflicts the events say changed, in order, and the last history
/// state they give.
fn reported(events: &[Event]) -> (Vec<usize>, Option<(bool, bool)>) {
    let mut conflicts = Vec::new();
    let mut history = None;
    for event in events {
        match event {
            Event::ChoicesChanged { conflicts: these } => conflicts.extend(these),
            Event::HistoryChanged { can_undo, can_redo } => history = Some((*can_undo, *can_redo)),
            _ => {}
        }
    }
    conflicts.sort_unstable();
    (conflicts, history)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// Markers as git writes them always read back as the conflicts they
    /// hold, sides and base byte for byte, and the file's common text is
    /// what an all-resolved merge keeps around the choices.
    #[test]
    fn written_markers_read_back_as_their_conflicts(parts in prop::collection::vec(part(), 0..6)) {
        let text = written(&parts);
        let session = MergeSession::from_text(&text, None).unwrap();
        let conflicts: Vec<_> = parts
            .iter()
            .filter_map(|part| match part {
                Part::Conflict { ours, base, theirs } => Some((ours, base, theirs)),
                Part::Common(_) => None,
            })
            .collect();
        prop_assert_eq!(session.conflict_count(), conflicts.len());
        for (index, (ours, base, theirs)) in conflicts.into_iter().enumerate() {
            let conflict = session.conflict(index);
            prop_assert_eq!(&conflict.ours.text, ours);
            prop_assert_eq!(conflict.base.as_ref().map(|side| &side.text), base.as_ref());
            prop_assert_eq!(&conflict.theirs.text, theirs);
        }
    }

    /// Whatever the commands, the session behaves as an editor's history:
    /// a step that changes something can be undone to exactly the choices
    /// before it and redone to exactly the choices after it; a new step
    /// drops what could be redone; a command that changes nothing, or is
    /// refused, adds no step and no event. The events name exactly the
    /// conflicts whose choice changed and the history as it now is, since
    /// the app and the MCP server refresh only what they are told.
    #[test]
    fn the_history_and_its_events_follow_every_change(
        parts in prop::collection::vec(part(), 1..6),
        commands in prop::collection::vec(command(), 0..40),
    ) {
        let mut session = MergeSession::from_text(&written(&parts), None).unwrap();
        // The model: the states undo and redo must give back.
        let mut undo: Vec<Vec<Option<Choice>>> = Vec::new();
        let mut redo: Vec<Vec<Option<Choice>>> = Vec::new();
        for command in commands {
            let before = choices(&session);
            let result = session.apply(command.clone());
            let after = choices(&session);
            let Ok(events) = result else {
                prop_assert_eq!(&after, &before, "refused {:?} changed the choices", command);
                prop_assert_eq!(
                    (session.can_undo(), session.can_redo()),
                    (!undo.is_empty(), !redo.is_empty())
                );
                continue;
            };
            match command {
                Command::Undo => {
                    if let Some(state) = undo.pop() {
                        prop_assert_eq!(&after, &state, "undo");
                        redo.push(before.clone());
                    }
                }
                Command::Redo => {
                    if let Some(state) = redo.pop() {
                        prop_assert_eq!(&after, &state, "redo");
                        undo.push(before.clone());
                    }
                }
                _ => {
                    if after != before {
                        undo.push(before.clone());
                        redo.clear();
                    }
                }
            }
            let history = (!undo.is_empty(), !redo.is_empty());
            prop_assert_eq!((session.can_undo(), session.can_redo()), history);
            let (conflicts, told) = reported(&events);
            if after == before {
                prop_assert!(events.is_empty(), "{:?} changed nothing: {:?}", command, events);
            } else {
                prop_assert_eq!(told, Some(history), "{:?}", command);
            }
            prop_assert_eq!(&conflicts, &changed(&before, &after), "{:?}", command);
            // The status and the merged text follow the choices.
            let resolved = after.iter().filter(|choice| choice.is_some()).count();
            prop_assert_eq!(session.status().resolved, resolved);
            prop_assert_eq!(session.merged_text().is_some(), resolved == after.len());
        }
        // Undoing everything gives the file back unresolved.
        while session.can_undo() {
            session.apply(Command::Undo).unwrap();
        }
        prop_assert!(choices(&session).iter().all(Option::is_none));
    }

    /// A CRLF file stays CRLF whatever is chosen and whatever is typed by
    /// hand, with `\n` or `\r\n` (a paste, another front end): a file
    /// mixing both shows as changed on every line in some tools and breaks
    /// `.gitattributes` checks.
    #[test]
    fn a_crlf_file_never_gets_a_bare_line_feed(
        parts in prop::collection::vec(part(), 1..6),
        commands in prop::collection::vec(command(), 0..20),
        last in prop::collection::vec(lines(2), 5),
        crlf_typed in any::<bool>(),
    ) {
        let text = written(&parts).replace('\n', "\r\n");
        let mut session = MergeSession::from_text(&text, None).unwrap();
        for command in commands {
            let command = match command {
                Command::Edit { conflict, text } if crlf_typed => Command::Edit {
                    conflict,
                    text: text.replace('\n', "\r\n"),
                },
                command => command,
            };
            let _ = session.apply(command);
        }
        // Whatever is left open is resolved by hand, so the file is whole.
        for (index, text) in last.into_iter().enumerate().take(session.conflict_count()) {
            if session.choice(index).is_none() {
                session.apply(Command::Edit { conflict: index, text }).unwrap();
            }
        }
        let merged = session.merged_text().unwrap();
        prop_assert_eq!(merged.matches('\n').count(), merged.matches("\r\n").count(), "{:?}", merged);
    }
}
