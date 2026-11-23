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
use scry_core::conflict;
use scry_core::records::Record;
use scry_core::resolution::{self, CandidateKind};
use scry_core::review::{self, Resolution};
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
        1 => (0..5usize).prop_map(|conflict| Command::MarkReviewed { conflict }),
        1 => (0..5usize).prop_map(|conflict| Command::Reopen { conflict }),
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

/// Runs `commands` on `session` against a model of its history (see
/// `the_history_and_its_events_follow_every_change`).
fn follows_history(mut session: MergeSession, commands: Vec<Command>) -> Result<(), TestCaseError> {
    let initial = choices(&session);
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
            prop_assert!(
                events.is_empty(),
                "{:?} changed nothing: {:?}",
                command,
                events
            );
        } else {
            prop_assert_eq!(told, Some(history), "{:?}", command);
        }
        prop_assert_eq!(&conflicts, &changed(&before, &after), "{:?}", command);
        // The status and the merged text follow the choices.
        let resolved = after.iter().filter(|choice| choice.is_some()).count();
        prop_assert_eq!(session.status().resolved, resolved);
        prop_assert_eq!(session.merged_text().is_some(), resolved == after.len());
    }
    // Undoing everything gives the session back as it started.
    while session.can_undo() {
        session.apply(Command::Undo).unwrap();
    }
    prop_assert_eq!(choices(&session), initial);
    Ok(())
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
        follows_history(MergeSession::from_text(&written(&parts), None).unwrap(), commands)?;
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

    /// A file resolved outside Scry, each conflict left open, given a
    /// candidate or given other text, with the text around the conflicts
    /// untouched: each resolution is found as it was made (other text
    /// equal to a candidate is that candidate). The review names what was
    /// done; a wrong name would tell the user "theirs" where it is not.
    #[test]
    fn every_resolution_made_outside_is_found(
        conflicts in prop::collection::vec(
            (lines(3), prop::option::of(lines(2)), lines(3), 0..3usize, any::<prop::sample::Index>(), lines(3)),
            1..5,
        ),
    ) {
        // Lines no side or resolution has, so each conflict's place is
        // certain.
        let mut parts = vec![Part::Common(String::from("first\n"))];
        for (index, (ours, base, theirs, ..)) in conflicts.iter().enumerate() {
            parts.push(Part::Conflict { ours: ours.clone(), base: base.clone(), theirs: theirs.clone() });
            parts.push(Part::Common(format!("between {index}\n")));
        }
        let text = written(&parts);
        let segments = conflict::parse(&text).unwrap();
        let analyses = resolution::analyze(&segments);
        let mut current = String::from("first\n");
        let mut expected = Vec::new();
        for (index, (ours, base, theirs, how, pick, own)) in conflicts.iter().enumerate() {
            let candidates = &analyses[index].candidates;
            let found = match how {
                0 => {
                    current.push_str(&written(&[Part::Conflict {
                        ours: ours.clone(),
                        base: base.clone(),
                        theirs: theirs.clone(),
                    }]));
                    Resolution::Open
                }
                1 => {
                    let candidate = pick.get(candidates);
                    current.push_str(&candidate.text);
                    Resolution::Candidate(candidate.kind)
                }
                _ => {
                    current.push_str(own);
                    candidates
                        .iter()
                        .find(|candidate| candidate.text == *own)
                        .map_or(Resolution::Text(own.clone()), |candidate| Resolution::Candidate(candidate.kind))
                }
            };
            expected.push(found);
            current.push_str(&format!("between {index}\n"));
        }
        let review = review::resolutions(&text, &current, 7, &review::Known::new()).unwrap();
        prop_assert!(!review.changed_outside);
        let found: Vec<Resolution> = review.conflicts.into_iter().map(|c| c.resolution).collect();
        prop_assert_eq!(found, expected);
    }

    /// Resolving conflicts one at a time, in any order, through what the MCP
    /// server's `resolve` writes (each left open, given a candidate's text
    /// or given other text): the file is the one written by hand with the
    /// same choices, each write changing only its conflict's lines, and
    /// finding what was done gives the same choices back.
    #[test]
    fn resolving_conflict_by_conflict_writes_the_choices_found_back(
        conflicts in prop::collection::vec(
            (lines(3), prop::option::of(lines(2)), lines(3), 0..3usize, any::<prop::sample::Index>(), lines(3)),
            1..5,
        ),
        order in any::<prop::sample::Index>(),
    ) {
        let mut parts = vec![Part::Common(String::from("first\n"))];
        for (index, (ours, base, theirs, ..)) in conflicts.iter().enumerate() {
            parts.push(Part::Conflict { ours: ours.clone(), base: base.clone(), theirs: theirs.clone() });
            parts.push(Part::Common(format!("between {index}\n")));
        }
        let text = written(&parts);
        let segments = conflict::parse(&text).unwrap();
        let analyses = resolution::analyze(&segments);
        let mut by_hand = String::from("first\n");
        let mut expected = Vec::new();
        let mut writes = Vec::new();
        for (index, (ours, base, theirs, how, pick, own)) in conflicts.iter().enumerate() {
            let candidates = &analyses[index].candidates;
            let found = match how {
                0 => {
                    by_hand.push_str(&written(&[Part::Conflict {
                        ours: ours.clone(),
                        base: base.clone(),
                        theirs: theirs.clone(),
                    }]));
                    Resolution::Open
                }
                1 => {
                    let candidate = pick.get(candidates);
                    by_hand.push_str(&candidate.text);
                    writes.push((
                        analyses[index].id.clone(),
                        review::Fill::Pick { kind: candidate.kind, text: candidate.text.clone() },
                        candidate.text.clone(),
                    ));
                    Resolution::Candidate(candidate.kind)
                }
                _ => {
                    by_hand.push_str(own);
                    writes.push((analyses[index].id.clone(), review::Fill::Text(own.clone()), own.clone()));
                    candidates
                        .iter()
                        .find(|candidate| candidate.text == *own)
                        .map_or(Resolution::Text(own.clone()), |candidate| Resolution::Candidate(candidate.kind))
                }
            };
            expected.push(found);
            by_hand.push_str(&format!("between {index}\n"));
        }
        // Any rotation of the order the agent resolves them in.
        if !writes.is_empty() {
            let shift = order.index(writes.len());
            writes.rotate_left(shift);
        }
        let mut current = text.clone();
        for (id, fill, expected) in &writes {
            let (file, region) =
                review::rewrite(&text, (true, true), &current, 7, id, fill, &review::Guard::default()).unwrap();
            prop_assert_eq!(&region, expected);
            current = file;
        }
        prop_assert_eq!(&current, &by_hand);
        let review = review::resolutions(&text, &current, 7, &review::Known::new()).unwrap();
        prop_assert!(!review.changed_outside);
        let found: Vec<Resolution> = review.conflicts.into_iter().map(|c| c.resolution).collect();
        prop_assert_eq!(found, expected);
    }

    /// The same, as the MCP server does it, each write recorded as the
    /// agent's, with the lines between the conflicts drawn from the same few
    /// lines as the resolutions (so a resolution can repeat them) and the
    /// file maybe without a final line break: the records place each
    /// resolution where it was written, so the file is the one written by
    /// hand and the choices are found back (review P1#2 and P1#3).
    #[test]
    fn recorded_resolutions_are_found_where_they_were_written(
        conflicts in prop::collection::vec(
            (lines(2), lines(3), prop::option::of(lines(2)), lines(3), 0..3usize, any::<prop::sample::Index>(), lines(3)),
            1..4,
        ),
        last in lines(2),
        order in any::<prop::sample::Index>(),
        unbroken in any::<bool>(),
    ) {
        let mut parts = Vec::new();
        for (before, ours, base, theirs, ..) in &conflicts {
            parts.push(Part::Common(before.clone()));
            parts.push(Part::Conflict { ours: ours.clone(), base: base.clone(), theirs: theirs.clone() });
        }
        parts.push(Part::Common(last.clone()));
        let text = written(&parts);
        let segments = conflict::parse(&text).unwrap();
        let analyses = resolution::analyze(&segments);
        prop_assume!(analyses.len() == conflicts.len());
        let mut by_hand = String::new();
        let mut expected = Vec::new();
        let mut writes = Vec::new();
        for (index, (before, ours, base, theirs, how, pick, own)) in conflicts.iter().enumerate() {
            by_hand.push_str(before);
            let candidates = &analyses[index].candidates;
            let found = match how {
                0 => {
                    by_hand.push_str(&written(&[Part::Conflict {
                        ours: ours.clone(),
                        base: base.clone(),
                        theirs: theirs.clone(),
                    }]));
                    Resolution::Open
                }
                1 => {
                    let candidate = pick.get(candidates);
                    by_hand.push_str(&candidate.text);
                    writes.push((
                        analyses[index].id.clone(),
                        review::Fill::Pick { kind: candidate.kind, text: candidate.text.clone() },
                        candidate.text.clone(),
                    ));
                    Resolution::Candidate(candidate.kind)
                }
                _ => {
                    by_hand.push_str(own);
                    writes.push((analyses[index].id.clone(), review::Fill::Text(own.clone()), own.clone()));
                    candidates
                        .iter()
                        .find(|candidate| candidate.text == *own)
                        .map_or(Resolution::Text(own.clone()), |candidate| Resolution::Candidate(candidate.kind))
                }
            };
            expected.push(found);
        }
        by_hand.push_str(&last);
        // A file without a final break: only where it ends with a line of
        // common text, as git leaves it (an empty last line would be removed,
        // not its break).
        let unbroken = unbroken && last.lines().last().is_some_and(|line| !line.is_empty());
        let mut current = text.clone();
        if unbroken {
            current.pop();
            by_hand.pop();
        }
        if !writes.is_empty() {
            let shift = order.index(writes.len());
            writes.rotate_left(shift);
        }
        let mut guard = review::Guard::default();
        for (id, fill, written) in &writes {
            let (file, region) = review::rewrite(&text, (true, true), &current, 7, id, fill, &guard).unwrap();
            prop_assert_eq!(&region, written);
            current = file;
            guard.records.insert(id.clone(), Record {
                conflict: id.clone(),
                by: String::from("agent"),
                pick: None,
                text: region,
                reason: String::new(),
                dropped: None,
                at: 0,
            });
        }
        prop_assert_eq!(&current, &by_hand);
        let known = review::known(&guard.records, &guard.reviewed);
        let review = review::resolutions(&text, &current, 7, &known).unwrap();
        prop_assert!(!review.changed_outside);
        let found: Vec<Resolution> = review.conflicts.into_iter().map(|c| c.resolution).collect();
        prop_assert_eq!(found, expected);
    }

    /// Whatever text fills the conflicts, from the same few lines the text
    /// around them uses (so a resolution can repeat the lines after it),
    /// the review places each one so the file is rebuilt exactly, and never
    /// says the file changed outside the conflicts when it did not. A line
    /// added outside them is either flagged, every conflict then unknown,
    /// or absorbed into a resolution that still rebuilds the file.
    #[test]
    fn a_review_rebuilds_the_file_or_flags_it(
        parts in prop::collection::vec(part(), 1..6),
        fills in prop::collection::vec(lines(3), 6),
        outside in prop::option::of((any::<prop::sample::Index>(), line())),
    ) {
        let text = written(&parts);
        let segments = conflict::parse(&text).unwrap();
        let analyses = resolution::analyze(&segments);
        let mut current = String::new();
        let mut index = 0;
        for (at, segment) in segments.iter().enumerate() {
            match segment {
                conflict::Segment::Common(common) => current.push_str(common),
                conflict::Segment::Conflict(_) => {
                    current.push_str(&fills[index % fills.len()]);
                    index += 1;
                }
            }
            if let Some((place, line)) = &outside
                && place.index(segments.len()) == at
            {
                current.push_str(line);
            }
        }
        let review = review::resolutions(&text, &current, 7, &review::Known::new()).unwrap();
        if review.changed_outside {
            prop_assert!(outside.is_some(), "flagged with nothing changed outside");
            prop_assert!(review.conflicts.iter().all(|c| c.resolution == Resolution::Unknown));
            return Ok(());
        }
        let mut rebuilt = String::new();
        let mut index = 0;
        for segment in &segments {
            match segment {
                conflict::Segment::Common(common) => rebuilt.push_str(common),
                conflict::Segment::Conflict(found) => {
                    match &review.conflicts[index].resolution {
                        Resolution::Open => rebuilt.push_str(&written(&[Part::Conflict {
                            ours: found.ours.text.clone(),
                            base: found.base.as_ref().map(|base| base.text.clone()),
                            theirs: found.theirs.text.clone(),
                        }])),
                        Resolution::Candidate(kind) => rebuilt.push_str(
                            &analyses[index].candidates.iter().find(|c| c.kind == *kind).unwrap().text,
                        ),
                        Resolution::Text(own) => rebuilt.push_str(own),
                        Resolution::Unknown => prop_assert!(false, "unknown without a change outside"),
                    }
                    index += 1;
                }
            }
        }
        prop_assert_eq!(rebuilt, current);
    }

    /// A file resolved outside Scry, opened for review: saved without a
    /// change, it is written back byte for byte (each choice is what the
    /// file holds); and its history follows the same model as any session,
    /// marking reviewed and reopening included, undoing everything giving
    /// back the choices it was opened with.
    #[test]
    fn a_review_session_keeps_the_file_and_its_history(
        parts in prop::collection::vec(part(), 1..6),
        fills in prop::collection::vec(prop::option::of(lines(3)), 6),
        commands in prop::collection::vec(command(), 0..30),
    ) {
        let text = written(&parts);
        let segments = conflict::parse(&text).unwrap();
        let mut current = String::new();
        let mut index = 0;
        for segment in &segments {
            match segment {
                conflict::Segment::Common(common) => current.push_str(common),
                conflict::Segment::Conflict(found) => {
                    match &fills[index % fills.len()] {
                        Some(fill) => current.push_str(fill),
                        None => current.push_str(&written(&[Part::Conflict {
                            ours: found.ours.text.clone(),
                            base: found.base.as_ref().map(|base| base.text.clone()),
                            theirs: found.theirs.text.clone(),
                        }])),
                    }
                    index += 1;
                }
            }
        }
        let session = review::session(
            &text,
            &current,
            7,
            None,
            (true, true),
            &std::collections::BTreeMap::new(),
            &std::collections::BTreeMap::new(),
        )
        .unwrap()
        .expect("nothing changed outside the conflicts");
        if session.unresolved() == 0 {
            prop_assert_eq!(session.merged_text(), Some(current.clone()));
        }
        prop_assert_eq!(session.status().unreviewed, session.conflict_count() - session.unresolved());
        follows_history(session, commands)?;
    }
}
