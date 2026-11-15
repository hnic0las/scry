//! The rows the merge editor shows: ours, result and theirs side by side,
//! aligned line for line, so every UI draws the same thing.
//!
//! Common lines appear on one row in all three panes. A conflict takes as
//! many rows as its longest pane; shorter panes get spacer rows. With the
//! base shown, a conflict's base lines come first, in the result pane only.

use serde::Serialize;
use std::ops::Range;

use unicode_width::UnicodeWidthChar;

use crate::conflict::Segment;
use crate::highlight::Span;
use crate::session::MergeSession;
use crate::words::LineChange;

/// Tab stops every this many columns, as most editors show code.
pub const TAB_WIDTH: usize = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
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
    /// A line of the conflict's base, shown in the result above the block
    /// when the user asks for the base; the sides get spacers.
    Base,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Cell {
    /// The line number in this pane's own file; `None` for rows without a
    /// line (spacer, unresolved, removed).
    pub number: Option<usize>,
    /// The line as shown: tabs are expanded to spaces up to the next tab
    /// stop, so every UI lines it up the same way, and spans and changed
    /// words are in bytes of this text.
    pub text: String,
    /// How wide `text` is in a monospaced font, in columns: characters
    /// that are wide in East Asian text (CJK, most emoji) count 2. A UI
    /// multiplies it by one column's width instead of measuring the line.
    pub columns: usize,
    pub kind: CellKind,
    /// Syntax highlighting of `text`; empty for plain text or without a
    /// grammar.
    pub spans: Vec<Span>,
    /// For a side's line in a conflict, how it differs from the base;
    /// `Unchanged` everywhere else.
    pub change: LineChange,
}

// Almost every row is a code row, so boxing the large variant would add an
// allocation per row and save no memory.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
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

/// Highlight spans by line, one list per pane, each indexed like the pane's
/// own line numbers (from 1).
struct Highlights<'a> {
    ours: &'a [Vec<Span>],
    result: &'a [Vec<Span>],
    theirs: &'a [Vec<Span>],
}

fn line(
    number: &mut usize,
    text: &str,
    kind: CellKind,
    spans: &[Vec<Span>],
    change: LineChange,
) -> Cell {
    let shown = Shown::new(text);
    let cell = Cell {
        number: Some(*number),
        text: shown.text.clone(),
        columns: shown.columns,
        kind,
        spans: spans
            .get(*number - 1)
            .map(|line| {
                line.iter()
                    .map(|span| Span {
                        start: shown.offset(span.start),
                        end: shown.offset(span.end),
                        role: span.role,
                    })
                    .collect()
            })
            .unwrap_or_default(),
        change: match change {
            LineChange::Edited { words } => LineChange::Edited {
                words: words.into_iter().map(|range| shown.range(range)).collect(),
            },
            other => other,
        },
    };
    *number += 1;
    cell
}

/// A line with its tabs expanded, and where each original byte went.
struct Shown {
    text: String,
    columns: usize,
    /// Byte offset of each tab in the original line, with the bytes
    /// inserted for it (its spaces minus the tab itself).
    tabs: Vec<(usize, usize)>,
}

impl Shown {
    fn new(line: &str) -> Shown {
        let mut text = String::with_capacity(line.len());
        let mut columns = 0;
        let mut tabs = Vec::new();
        for (offset, character) in line.char_indices() {
            if character == '\t' {
                let spaces = TAB_WIDTH - columns % TAB_WIDTH;
                text.extend(std::iter::repeat_n(' ', spaces));
                columns += spaces;
                tabs.push((offset, spaces - 1));
            } else {
                text.push(character);
                columns += character.width().unwrap_or(0);
            }
        }
        Shown {
            text,
            columns,
            tabs,
        }
    }

    /// Where byte `offset` of the original line is in the shown text: after
    /// the spaces of every tab before it.
    fn offset(&self, offset: usize) -> usize {
        let inserted: usize = self
            .tabs
            .iter()
            .take_while(|(tab, _)| *tab < offset)
            .map(|(_, extra)| extra)
            .sum();
        offset + inserted
    }

    fn range(&self, range: Range<usize>) -> Range<usize> {
        self.offset(range.start)..self.offset(range.end)
    }
}

fn empty(kind: CellKind) -> Cell {
    Cell {
        number: None,
        text: String::new(),
        columns: 0,
        kind,
        spans: Vec::new(),
        change: LineChange::Unchanged,
    }
}

/// A conflict's base lines, in the result pane only. They have no number:
/// they are not lines of the result, and the UI labels them "base".
fn push_base_rows(rows: &mut Vec<Row>, index: usize, base: Option<&str>) {
    let Some(base) = base else { return };
    for text in base.lines() {
        let mut number = 1;
        let mut cell = line(
            &mut number,
            text,
            CellKind::Base,
            &[],
            LineChange::Unchanged,
        );
        cell.number = None;
        rows.push(Row::Code {
            ours: empty(CellKind::Spacer),
            result: cell,
            theirs: empty(CellKind::Spacer),
            conflict: Some(index),
        });
    }
}

impl MergeSession {
    /// The editor's rows; `show_base` adds each conflict's base lines
    /// above its block.
    pub fn rows(&self, show_base: bool) -> Vec<Row> {
        let mut rows = Vec::new();
        let mut numbers = Numbers {
            ours: 1,
            result: 1,
            theirs: 1,
        };
        let result_spans = self.result_spans();
        let highlights = Highlights {
            ours: self.ours_spans(),
            result: &result_spans,
            theirs: self.theirs_spans(),
        };
        let mut index = 0;
        for segment in self.segments() {
            match segment {
                Segment::Common(text) => {
                    for text in text.lines() {
                        rows.push(Row::Code {
                            ours: line(
                                &mut numbers.ours,
                                text,
                                CellKind::Common,
                                highlights.ours,
                                LineChange::Unchanged,
                            ),
                            result: line(
                                &mut numbers.result,
                                text,
                                CellKind::Common,
                                highlights.result,
                                LineChange::Unchanged,
                            ),
                            theirs: line(
                                &mut numbers.theirs,
                                text,
                                CellKind::Common,
                                highlights.theirs,
                                LineChange::Unchanged,
                            ),
                            conflict: None,
                        });
                    }
                }
                Segment::Conflict(conflict) => {
                    rows.push(Row::Lens { conflict: index });
                    if show_base {
                        push_base_rows(
                            &mut rows,
                            index,
                            conflict.base.as_ref().map(|base| base.text.as_str()),
                        );
                    }
                    self.push_conflict_rows(
                        &mut rows,
                        &mut numbers,
                        &highlights,
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
        highlights: &Highlights,
        index: usize,
        ours: &str,
        theirs: &str,
    ) {
        let (ours_changes, theirs_changes) = self.line_changes(index);
        let ours: Vec<&str> = ours.lines().collect();
        let theirs: Vec<&str> = theirs.lines().collect();
        let chosen_text = self.chosen_text(index);
        // The result shows the chosen lines, or one row saying the block is
        // unresolved or removed.
        let result: Vec<&str> = chosen_text
            .map(|text| text.lines().collect())
            .unwrap_or_default();
        let result_rows = result.len().max(1);
        let count = ours.len().max(theirs.len()).max(result_rows);

        for row in 0..count {
            let ours_cell = match ours.get(row) {
                Some(text) => line(
                    &mut numbers.ours,
                    text,
                    CellKind::Conflict,
                    highlights.ours,
                    ours_changes[row].clone(),
                ),
                None => empty(CellKind::Spacer),
            };
            let theirs_cell = match theirs.get(row) {
                Some(text) => line(
                    &mut numbers.theirs,
                    text,
                    CellKind::Conflict,
                    highlights.theirs,
                    theirs_changes[row].clone(),
                ),
                None => empty(CellKind::Spacer),
            };
            let result_cell = match (chosen_text, result.get(row)) {
                (Some(_), Some(text)) => line(
                    &mut numbers.result,
                    text,
                    CellKind::Chosen,
                    highlights.result,
                    LineChange::Unchanged,
                ),
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
    use crate::highlight::SyntaxRole;
    use crate::resolution::CandidateKind;
    use crate::syntax::Language;

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

    /// A conflict takes as many rows as its longest side, with spacers in shorter panes, and
    /// each pane keeps numbering its own lines after it.
    #[test]
    fn unresolved_conflict_is_padded_to_its_longest_side() {
        let session = MergeSession::from_text(TEXT, None).unwrap();
        let rows = session.rows(false);
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

    /// With the base shown, its lines come after the lens row, in the result only and
    /// without a number, so the sides get spacers and every pane keeps its numbering.
    #[test]
    fn base_lines_go_in_the_result_above_the_block() {
        let session = MergeSession::from_text(TEXT, None).unwrap();
        let rows = session.rows(true);
        assert_eq!(
            kinds(&rows),
            [
                "Common Common Common",
                "lens 0",
                "Spacer Base Spacer",
                "Conflict Unresolved Conflict",
                "Spacer Spacer Conflict",
                "Spacer Spacer Conflict",
                "Common Common Common",
            ]
        );
        let Row::Code { result, .. } = &rows[2] else {
            panic!("expected a code row");
        };
        assert_eq!((result.number, result.text.as_str()), (None, "b"));
        let Row::Code { result, .. } = &rows[6] else {
            panic!("expected a code row");
        };
        assert_eq!(result.number, Some(2));
    }

    /// A conflict without a base (no diff3 markers) adds no row when the base is shown.
    #[test]
    fn no_base_adds_no_rows() {
        let text = "a\n<<<<<<< ours\no\n=======\nt\n>>>>>>> theirs\nz\n";
        let session = MergeSession::from_text(text, None).unwrap();
        assert_eq!(session.rows(true), session.rows(false));
    }

    /// Once a candidate is chosen, the result pane shows its lines, padded like the sides.
    #[test]
    fn chosen_candidate_fills_the_result() {
        let mut session = MergeSession::from_text(TEXT, None).unwrap();
        session.choose(0, CandidateKind::TheirsThenOurs).unwrap();
        let rows = session.rows(false);
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

    /// Each cell is highlighted as a line of its own pane's file, so a side's line after the
    /// conflict gets the spans of that side, not another pane's.
    #[test]
    fn cells_carry_the_spans_of_their_pane() {
        let text = "let a = 1\n<<<<<<< ours\nlet b = \"x\"\n||||||| base\nlet b = 0\n=======\n// none\n>>>>>>> theirs\nlet z = 2\n";
        let mut session = MergeSession::from_text(text, Some(Language::Swift)).unwrap();
        session.choose(0, CandidateKind::Ours).unwrap();
        let rows = session.rows(false);
        let roles =
            |cell: &Cell| -> Vec<SyntaxRole> { cell.spans.iter().map(|span| span.role).collect() };
        let Row::Code {
            ours,
            result,
            theirs,
            ..
        } = &rows[2]
        else {
            panic!("expected a code row");
        };
        assert!(roles(ours).contains(&SyntaxRole::String));
        assert!(roles(result).contains(&SyntaxRole::String));
        assert_eq!(roles(theirs), [SyntaxRole::Comment]);
        // After the conflict, each pane uses its own line of its own file.
        let Row::Code { theirs, .. } = &rows[3] else {
            panic!("expected a code row");
        };
        assert_eq!(theirs.text, "let z = 2");
        assert!(roles(theirs).contains(&SyntaxRole::Number));
    }

    /// Conflict cells carry their line change against the base (added, edited with the
    /// changed words), and cells outside conflicts carry none.
    #[test]
    fn conflict_lines_carry_their_change_against_the_base() {
        let text = "a\n<<<<<<< ours\nsend(item, timeout)\n||||||| base\nsend(item)\n=======\nretry {\n    send(item)\n}\n>>>>>>> theirs\nz\n";
        let session = MergeSession::from_text(text, None).unwrap();
        let rows = session.rows(false);
        let changes: Vec<(LineChange, LineChange)> = rows
            .iter()
            .filter_map(|row| match row {
                Row::Code { ours, theirs, .. } => {
                    Some((ours.change.clone(), theirs.change.clone()))
                }
                Row::Lens { .. } => None,
            })
            .collect();
        let edited = |words: Vec<std::ops::Range<usize>>| LineChange::Edited { words };
        assert_eq!(
            changes,
            [
                (LineChange::Unchanged, LineChange::Unchanged),
                (edited(std::iter::once(9..18).collect()), LineChange::Added),
                (LineChange::Unchanged, edited(Vec::new())),
                (LineChange::Unchanged, LineChange::Added),
                (LineChange::Unchanged, LineChange::Unchanged),
            ]
        );
    }

    /// Choosing none removes the block; the result shows one row saying so.
    #[test]
    fn removed_block_shows_one_row() {
        let mut session = MergeSession::from_text(TEXT, None).unwrap();
        session.choose(0, CandidateKind::Neither).unwrap();
        let rows = session.rows(false);
        assert_eq!(kinds(&rows)[2], "Conflict Removed Conflict");
        // The removal is said once; the other rows of the block are spacers.
        assert_eq!(kinds(&rows)[3], "Spacer Spacer Conflict");
        assert_eq!(kinds(&rows)[4], "Spacer Spacer Conflict");
    }

    /// Each conflict's rows show its own sides, not the first conflict's.
    #[test]
    fn each_conflict_shows_its_own_sides() {
        let text = "<<<<<<< ours\none\n=======\nuno\n>>>>>>> theirs\nmid\n<<<<<<< ours\ntwo\n=======\ndos\n>>>>>>> theirs\n";
        let mut session = MergeSession::from_text(text, None).unwrap();
        session.choose(1, CandidateKind::Theirs).unwrap();
        let chosen: Vec<String> = session
            .rows(false)
            .iter()
            .filter_map(|row| match row {
                Row::Code { result, .. } if result.kind == CellKind::Chosen => {
                    Some(result.text.clone())
                }
                _ => None,
            })
            .collect();
        assert_eq!(chosen, ["dos"]);
        let sides: Vec<(String, String)> = session
            .rows(false)
            .iter()
            .filter_map(|row| match row {
                Row::Code { ours, theirs, .. } if ours.kind == CellKind::Conflict => {
                    Some((ours.text.clone(), theirs.text.clone()))
                }
                _ => None,
            })
            .collect();
        assert_eq!(
            sides,
            [
                (String::from("one"), String::from("uno")),
                (String::from("two"), String::from("dos"))
            ]
        );
    }

    /// Tabs become spaces up to the next stop of four columns, and wide
    /// characters count two: the UI sizes lines from `columns`, and a line
    /// measured short could not be scrolled to its end.
    #[test]
    fn lines_are_measured_in_columns() {
        let tabbed = Shown::new("\tif a {\tb");
        assert_eq!(tabbed.text, "    if a {  b");
        assert_eq!(tabbed.columns, 13);
        // "if" after the first tab, "b" after both.
        assert_eq!(tabbed.offset(1), 4);
        assert_eq!(tabbed.offset(8), 12);
        assert_eq!(&tabbed.text[tabbed.range(8..9)], "b");

        let wide = Shown::new("let 名前 = 1 🚀");
        assert_eq!(wide.text, "let 名前 = 1 🚀");
        assert_eq!(wide.columns, 4 + 4 + 5 + 2);
    }

    /// Colours and changed words follow the expanded text, so a keyword
    /// after a tab stays coloured where it is drawn.
    #[test]
    fn spans_and_changed_words_follow_expanded_tabs() {
        let text = "package a\n\nfunc f() {\n<<<<<<< ours\n\treturn 1\n||||||| base\n\treturn 0\n=======\n\treturn 2\n>>>>>>> theirs\n}\n";
        let session = MergeSession::from_text(text, Some(Language::Go)).unwrap();
        let rows = session.rows(false);
        let ours = rows
            .iter()
            .find_map(|row| match row {
                Row::Code { ours, .. } if ours.kind == CellKind::Conflict => Some(ours),
                _ => None,
            })
            .unwrap();

        assert_eq!(ours.text, "    return 1");
        assert_eq!(ours.columns, 12);
        let keyword = ours
            .spans
            .iter()
            .find(|span| span.role == SyntaxRole::Keyword)
            .unwrap();
        assert_eq!(&ours.text[keyword.start..keyword.end], "return");
        let LineChange::Edited { words } = &ours.change else {
            panic!("expected an edited line, got {:?}", ours.change);
        };
        assert_eq!(&ours.text[words[0].clone()], "1");
    }
}
