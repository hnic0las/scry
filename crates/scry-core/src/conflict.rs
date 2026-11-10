//! Parsing of files that contain git conflict markers.
//!
//! Supports the three `merge.conflictStyle` values: `merge` (ours and theirs),
//! and `diff3` / `zdiff3`, which add the merge base between `|||||||` and
//! `=======`.
//!
//! Text is kept byte for byte, line endings included, so that a file can be
//! rebuilt exactly; CRLF files from Windows stay CRLF.

use std::fmt;

/// Marker length git uses unless the `conflict-marker-size` attribute says otherwise.
pub const DEFAULT_MARKER_SIZE: usize = 7;

/// One part of a file with conflict markers, in file order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Segment {
    /// Text outside any conflict.
    Common(String),
    /// A conflict region.
    Conflict(Conflict),
}

/// One side of a conflict: the label git wrote after the marker, and the text.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Side {
    /// Label after the marker, for example `HEAD` or `abc1234 (commit message)`.
    /// Empty when git wrote none.
    pub label: String,
    /// Lines of this side, line endings included.
    pub text: String,
}

/// A conflict region between `<<<<<<<` and `>>>>>>>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conflict {
    /// 1-based line number of the opening marker.
    pub start_line: usize,
    pub ours: Side,
    /// Present with the `diff3` and `zdiff3` conflict styles.
    pub base: Option<Side>,
    pub theirs: Side,
}

/// Why a file could not be parsed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    /// 1-based line number where the problem was found.
    pub line: usize,
    pub kind: ParseErrorKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseErrorKind {
    /// A marker that cannot appear at this point, such as a nested `<<<<<<<`.
    UnexpectedMarker(Marker),
    /// The file ends inside a conflict.
    UnterminatedConflict,
}

/// The four conflict markers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Marker {
    /// `<<<<<<<`
    Ours,
    /// `|||||||`
    Base,
    /// `=======`
    Separator,
    /// `>>>>>>>`
    Theirs,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.kind {
            ParseErrorKind::UnexpectedMarker(marker) => {
                write!(
                    f,
                    "line {}: unexpected {marker:?} conflict marker",
                    self.line
                )
            }
            ParseErrorKind::UnterminatedConflict => {
                write!(f, "line {}: conflict is never closed", self.line)
            }
        }
    }
}

impl std::error::Error for ParseError {}

/// Parses `text` with the default marker size.
pub fn parse(text: &str) -> Result<Vec<Segment>, ParseError> {
    parse_with_marker_size(text, DEFAULT_MARKER_SIZE)
}

/// Parses `text`, with markers of `marker_size` characters.
///
/// Outside a conflict, only an opening marker is significant: lines such as
/// `=======` (a Markdown heading underline) stay common text. Inside a
/// conflict, any marker out of order is an error rather than content: refusing
/// to parse is safer than splitting a conflict at the wrong place and losing
/// work.
pub fn parse_with_marker_size(text: &str, marker_size: usize) -> Result<Vec<Segment>, ParseError> {
    enum State {
        Outside,
        Ours,
        Base,
        Theirs,
    }

    let mut segments = Vec::new();
    let mut common = String::new();
    let mut state = State::Outside;
    let mut start_line = 0;
    let mut ours = Side::default();
    let mut base: Option<Side> = None;
    let mut theirs = Side::default();

    for (index, line) in text.split_inclusive('\n').enumerate() {
        let line_number = index + 1;
        let marker = parse_marker(line, marker_size);
        let unexpected = |marker| ParseError {
            line: line_number,
            kind: ParseErrorKind::UnexpectedMarker(marker),
        };

        match state {
            State::Outside => match marker {
                Some((Marker::Ours, label)) => {
                    if !common.is_empty() {
                        segments.push(Segment::Common(std::mem::take(&mut common)));
                    }
                    start_line = line_number;
                    ours = Side {
                        label,
                        text: String::new(),
                    };
                    base = None;
                    theirs = Side::default();
                    state = State::Ours;
                }
                _ => common.push_str(line),
            },
            State::Ours => match marker {
                None => ours.text.push_str(line),
                Some((Marker::Base, label)) => {
                    base = Some(Side {
                        label,
                        text: String::new(),
                    });
                    state = State::Base;
                }
                Some((Marker::Separator, _)) => state = State::Theirs,
                Some((other, _)) => return Err(unexpected(other)),
            },
            State::Base => match marker {
                None => {
                    if let Some(base) = base.as_mut() {
                        base.text.push_str(line);
                    }
                }
                Some((Marker::Separator, _)) => state = State::Theirs,
                Some((other, _)) => return Err(unexpected(other)),
            },
            State::Theirs => match marker {
                None => theirs.text.push_str(line),
                Some((Marker::Theirs, label)) => {
                    theirs.label = label;
                    segments.push(Segment::Conflict(Conflict {
                        start_line,
                        ours: std::mem::take(&mut ours),
                        base: base.take(),
                        theirs: std::mem::take(&mut theirs),
                    }));
                    state = State::Outside;
                }
                Some((other, _)) => return Err(unexpected(other)),
            },
        }
    }

    if !matches!(state, State::Outside) {
        return Err(ParseError {
            line: start_line,
            kind: ParseErrorKind::UnterminatedConflict,
        });
    }
    if !common.is_empty() {
        segments.push(Segment::Common(common));
    }
    Ok(segments)
}

/// The conflicts among `segments`, in file order.
pub fn conflicts(segments: &[Segment]) -> impl Iterator<Item = &Conflict> {
    segments.iter().filter_map(|segment| match segment {
        Segment::Conflict(conflict) => Some(conflict),
        Segment::Common(_) => None,
    })
}

/// Recognises a marker line: exactly `size` marker characters, then a space
/// and a label, or the end of the line. `=======` never carries a label.
fn parse_marker(line: &str, size: usize) -> Option<(Marker, String)> {
    let content = line.strip_suffix('\n').unwrap_or(line);
    let content = content.strip_suffix('\r').unwrap_or(content);

    let marker = match content.chars().next()? {
        '<' => Marker::Ours,
        '|' => Marker::Base,
        '=' => Marker::Separator,
        '>' => Marker::Theirs,
        _ => return None,
    };
    let marker_char = content.chars().next()?;
    // `get` gives nothing when `size` falls inside a character of more than
    // one byte (`> é…`): such a line is text, not a marker.
    let prefix = content.get(..size)?;
    if !prefix.chars().all(|c| c == marker_char) {
        return None;
    }

    let rest = &content[size..];
    if rest.is_empty() {
        return Some((marker, String::new()));
    }
    if marker == Marker::Separator {
        return None;
    }
    rest.strip_prefix(' ')
        .map(|label| (marker, label.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn only_conflict(segments: &[Segment]) -> &Conflict {
        let found: Vec<_> = conflicts(segments).collect();
        assert_eq!(found.len(), 1, "expected exactly one conflict");
        found[0]
    }

    /// A file without markers must come back whole, byte for byte, so nothing is lost.
    #[test]
    fn file_without_conflict_is_one_common_segment() {
        let text = "a\nb\n";
        assert_eq!(
            parse(text).unwrap(),
            vec![Segment::Common(text.to_string())]
        );
    }

    /// Lines that start like a marker and go on with accented characters
    /// (a Markdown quote in French, a table) are plain text; they once made
    /// the parser panic.
    #[test]
    fn non_ascii_lines_that_start_like_markers_are_text() {
        let text = "> ééé\n======é\n<<ééé\n| ééé |\n>>>>>>> é\n";
        assert_eq!(
            parse(text).unwrap(),
            vec![Segment::Common(text.to_string())]
        );
    }

    /// An empty file is valid input and gives nothing to show, not an error.
    #[test]
    fn empty_file_has_no_segment() {
        assert_eq!(parse("").unwrap(), vec![]);
    }

    /// The default `merge` style: both sides, their labels, no base, and the text around the
    /// conflict kept apart.
    #[test]
    fn merge_style_conflict() {
        let text = "before\n<<<<<<< HEAD\nours\n=======\ntheirs\n>>>>>>> feature\nafter\n";
        let segments = parse(text).unwrap();
        assert_eq!(segments.len(), 3);
        assert_eq!(segments[0], Segment::Common("before\n".to_string()));
        assert_eq!(segments[2], Segment::Common("after\n".to_string()));

        let conflict = only_conflict(&segments);
        assert_eq!(conflict.start_line, 2);
        assert_eq!(
            conflict.ours,
            Side {
                label: "HEAD".into(),
                text: "ours\n".into()
            }
        );
        assert_eq!(conflict.base, None);
        assert_eq!(
            conflict.theirs,
            Side {
                label: "feature".into(),
                text: "theirs\n".into()
            }
        );
    }

    /// The `diff3` style adds the base; the parser must keep it with its label, since
    /// classification and word changes compare against it.
    #[test]
    fn diff3_style_conflict_keeps_the_base() {
        let text = "<<<<<<< HEAD\nours\n||||||| merged common ancestors\nbase\n=======\ntheirs\n>>>>>>> feature\n";
        let conflict = parse(text).unwrap().into_iter().next().unwrap();
        let Segment::Conflict(conflict) = conflict else {
            panic!("expected a conflict")
        };
        assert_eq!(
            conflict.base,
            Some(Side {
                label: "merged common ancestors".into(),
                text: "base\n".into()
            })
        );
    }

    /// Windows line endings stay in the sides' text and out of the labels, so a merge never
    /// rewrites them.
    #[test]
    fn crlf_line_endings_are_preserved() {
        let text = "a\r\n<<<<<<< HEAD\r\nours\r\n=======\r\ntheirs\r\n>>>>>>> b\r\n";
        let segments = parse(text).unwrap();
        let conflict = only_conflict(&segments);
        assert_eq!(conflict.ours.label, "HEAD");
        assert_eq!(conflict.ours.text, "ours\r\n");
        assert_eq!(conflict.theirs.label, "b");
    }

    /// Markers may carry no label and a side may be empty (one side added lines); both are
    /// still a conflict.
    #[test]
    fn markers_without_labels_and_empty_sides() {
        let text = "<<<<<<<\n=======\nadded\n>>>>>>>\n";
        let segments = parse(text).unwrap();
        let conflict = only_conflict(&segments);
        assert_eq!(conflict.ours, Side::default());
        assert_eq!(conflict.theirs.text, "added\n");
    }

    /// A closing marker on the file's last line, without a final newline, still closes the
    /// conflict.
    #[test]
    fn last_line_without_newline() {
        let text = "<<<<<<< HEAD\nours\n=======\ntheirs\n>>>>>>> b";
        let conflict = parse(text).unwrap().into_iter().next().unwrap();
        let Segment::Conflict(conflict) = conflict else {
            panic!("expected a conflict")
        };
        assert_eq!(conflict.theirs.label, "b");
    }

    /// Each conflict reports the line it starts on, in file order, for navigation.
    #[test]
    fn several_conflicts_in_order() {
        let text =
            "<<<<<<< a\n1\n=======\n2\n>>>>>>> b\nmiddle\n<<<<<<< a\n3\n=======\n4\n>>>>>>> b\n";
        let segments = parse(text).unwrap();
        let lines: Vec<_> = conflicts(&segments).map(|c| c.start_line).collect();
        assert_eq!(lines, vec![1, 7]);
    }

    /// `=======` and `>>>>>>>` alone are ordinary text (a Markdown underline, a quote), not
    /// an error.
    #[test]
    fn separator_and_closing_markers_outside_a_conflict_are_text() {
        let text = "Title\n=======\n>>>>>>> not a conflict\n";
        assert_eq!(
            parse(text).unwrap(),
            vec![Segment::Common(text.to_string())]
        );
    }

    /// A run longer than the marker size is text: only exact markers count.
    #[test]
    fn longer_runs_of_marker_characters_are_text() {
        let text = "<<<<<<<< eight\n";
        assert_eq!(
            parse(text).unwrap(),
            vec![Segment::Common(text.to_string())]
        );
    }

    /// Git always puts a space before a label, so `<<<<<<<HEAD` is text, not a marker.
    #[test]
    fn marker_followed_by_text_without_space_is_text() {
        let text = "<<<<<<<HEAD\n";
        assert_eq!(
            parse(text).unwrap(),
            vec![Segment::Common(text.to_string())]
        );
    }

    /// With `conflict-marker-size` set, markers have another length; the parser follows the
    /// size it is given and nothing else.
    #[test]
    fn custom_marker_size() {
        let text = "<<<<<<<<<< HEAD\nours\n==========\ntheirs\n>>>>>>>>>> b\n";
        let segments = parse_with_marker_size(text, 10).unwrap();
        assert_eq!(only_conflict(&segments).theirs.text, "theirs\n");
        // With the default size, the same lines are not markers.
        assert_eq!(
            parse(text).unwrap(),
            vec![Segment::Common(text.to_string())]
        );
    }

    /// A conflict never closed is refused rather than guessed, and the error points to where
    /// it opens.
    #[test]
    fn unterminated_conflict_reports_its_opening_line() {
        let text = "x\n<<<<<<< HEAD\nours\n=======\ntheirs\n";
        assert_eq!(
            parse(text).unwrap_err(),
            ParseError {
                line: 2,
                kind: ParseErrorKind::UnterminatedConflict
            }
        );
    }

    /// Markers out of order (a second opening inside a conflict) are refused: guessing could
    /// lose the user's work.
    #[test]
    fn nested_opening_marker_is_an_error() {
        let text = "<<<<<<< HEAD\n<<<<<<< again\n";
        assert_eq!(
            parse(text).unwrap_err(),
            ParseError {
                line: 2,
                kind: ParseErrorKind::UnexpectedMarker(Marker::Ours)
            }
        );
    }

    /// A closing marker before the separator is refused, with the line it is on.
    #[test]
    fn closing_marker_before_separator_is_an_error() {
        let text = "<<<<<<< HEAD\nours\n>>>>>>> b\n";
        assert_eq!(
            parse(text).unwrap_err(),
            ParseError {
                line: 3,
                kind: ParseErrorKind::UnexpectedMarker(Marker::Theirs)
            }
        );
    }

    /// The error shown to the user names the line, so they can find the problem in the file.
    #[test]
    fn error_message_names_the_line() {
        let error = parse("<<<<<<< HEAD\n").unwrap_err();
        assert_eq!(error.to_string(), "line 1: conflict is never closed");
    }
}
