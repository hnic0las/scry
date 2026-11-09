//! The rows the merge editor shows: ours, result and theirs side by side,
//! aligned line for line, so every UI draws the same thing.
//!
//! Common lines appear on one row in all three panes. A conflict takes as
//! many rows as its longest pane; shorter panes get spacer rows.

use crate::conflict::Segment;
use crate::session::MergeSession;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CellKind {
    /// A line outside any conflict.
    Common,
    /// A side's line inside a conflict.
    Conflict,
    /// No line here: fills a shorter pane so the panes stay aligned.
    Spacer,
    /// The result of a conflict with no choice yet.
    Unresolved,
    /// A result line from the chosen candidate.
    Chosen,
    /// The chosen candidate removes the block.
    Removed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cell {
    /// The line number in this pane's own file; `None` for rows without a
    /// line (spacer, unresolved, removed).
    pub number: Option<usize>,
    pub text: String,
    pub kind: CellKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Row {
    /// Above each conflict, across the three panes.
    Lens { conflict: usize },
    Code {
        ours: Cell,
        result: Cell,
        theirs: Cell,
        /// The conflict this row belongs to, from 0.
        conflict: Option<usize>,
    },
}

/// Line counters, one per pane.
struct Numbers {
    ours: usize,
    result: usize,
    theirs: usize,
}

fn line(number: &mut usize, text: &str, kind: CellKind) -> Cell {
    let cell = Cell {
        number: Some(*number),
        text: text.to_string(),
        kind,
    };
    *number += 1;
    cell
}

fn empty(kind: CellKind) -> Cell {
    Cell {
        number: None,
        text: String::new(),
        kind,
    }
}

impl MergeSession {
    pub fn rows(&self) -> Vec<Row> {
        let mut rows = Vec::new();
        let mut numbers = Numbers {
            ours: 1,
            result: 1,
            theirs: 1,
        };
        let mut index = 0;
        for segment in self.segments() {
            match segment {
                Segment::Common(text) => {
                    for text in text.lines() {
                        rows.push(Row::Code {
                            ours: line(&mut numbers.ours, text, CellKind::Common),
                            result: line(&mut numbers.result, text, CellKind::Common),
                            theirs: line(&mut numbers.theirs, text, CellKind::Common),
                            conflict: None,
                        });
                    }
                }
                Segment::Conflict(conflict) => {
                    rows.push(Row::Lens { conflict: index });
                    self.push_conflict_rows(
                        &mut rows,
                        &mut numbers,
                        index,
                        &conflict.ours.text,
                        &conflict.theirs.text,
                    );
                    index += 1;
                }
            }
        }
        rows
    }

    fn push_conflict_rows(
        &self,
        rows: &mut Vec<Row>,
        numbers: &mut Numbers,
        index: usize,
        ours: &str,
        theirs: &str,
    ) {
        let ours: Vec<&str> = ours.lines().collect();
        let theirs: Vec<&str> = theirs.lines().collect();
        let chosen_text = self.choice(index).and_then(|kind| {
            self.analysis(index)
                .candidates
                .iter()
                .find(|candidate| candidate.kind == kind)
                .map(|candidate| candidate.text.as_str())
        });
        // The result shows the chosen lines, or one row saying the block is
        // unresolved or removed.
        let result: Vec<&str> = chosen_text
            .map(|text| text.lines().collect())
            .unwrap_or_default();
        let result_rows = result.len().max(1);
        let count = ours.len().max(theirs.len()).max(result_rows);

        for row in 0..count {
            let ours_cell = match ours.get(row) {
                Some(text) => line(&mut numbers.ours, text, CellKind::Conflict),
                None => empty(CellKind::Spacer),
            };
            let theirs_cell = match theirs.get(row) {
                Some(text) => line(&mut numbers.theirs, text, CellKind::Conflict),
                None => empty(CellKind::Spacer),
            };
            let result_cell = match (chosen_text, result.get(row)) {
                (Some(_), Some(text)) => line(&mut numbers.result, text, CellKind::Chosen),
                (Some(_), None) if row == 0 => empty(CellKind::Removed),
                (None, _) if row == 0 => empty(CellKind::Unresolved),
                _ => empty(CellKind::Spacer),
            };
            rows.push(Row::Code {
                ours: ours_cell,
                result: result_cell,
                theirs: theirs_cell,
                conflict: Some(index),
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resolution::CandidateKind;

    const TEXT: &str =
        "a\n<<<<<<< ours\no1\n||||||| base\nb\n=======\nt1\nt2\nt3\n>>>>>>> theirs\nz\n";

    fn kinds(rows: &[Row]) -> Vec<String> {
        rows.iter()
            .map(|row| match row {
                Row::Lens { conflict } => format!("lens {conflict}"),
                Row::Code {
                    ours,
                    result,
                    theirs,
                    ..
                } => format!("{:?} {:?} {:?}", ours.kind, result.kind, theirs.kind),
            })
            .collect()
    }

    #[test]
    fn unresolved_conflict_is_padded_to_its_longest_side() {
        let session = MergeSession::from_text(TEXT, None).unwrap();
        let rows = session.rows();
        assert_eq!(
            kinds(&rows),
            [
                "Common Common Common",
                "lens 0",
                "Conflict Unresolved Conflict",
                "Spacer Spacer Conflict",
                "Spacer Spacer Conflict",
                "Common Common Common",
            ]
        );
        // After the conflict, each pane continues its own numbering.
        let Row::Code {
            ours,
            result,
            theirs,
            ..
        } = &rows[5]
        else {
            panic!("expected a code row");
        };
        assert_eq!(
            (ours.number, result.number, theirs.number),
            (Some(3), Some(2), Some(5))
        );
        assert_eq!(ours.text, "z");
    }

    #[test]
    fn chosen_candidate_fills_the_result() {
        let mut session = MergeSession::from_text(TEXT, None).unwrap();
        session.choose(0, CandidateKind::TheirsThenOurs).unwrap();
        let rows = session.rows();
        assert_eq!(
            kinds(&rows)[2..6],
            [
                "Conflict Chosen Conflict",
                "Spacer Chosen Conflict",
                "Spacer Chosen Conflict",
                "Spacer Chosen Spacer",
            ]
        );
    }

    #[test]
    fn removed_block_shows_one_row() {
        let mut session = MergeSession::from_text(TEXT, None).unwrap();
        session.choose(0, CandidateKind::Neither).unwrap();
        let rows = session.rows();
        assert_eq!(kinds(&rows)[2], "Conflict Removed Conflict");
    }
}
