//! Word-level changes: which lines of a conflict side were added or edited
//! against the base, and which words changed in an edited line.
//!
//! Lines equal to a base line are matched first (longest common
//! subsequence). Between two matches, each remaining side line is paired
//! with the most similar remaining base line, in order; a side line with no
//! similar base line is added. Whitespace never counts as a changed word, so
//! a re-indented line is edited with no word marked.

use std::ops::Range;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LineChange {
    /// Equal to a base line.
    Unchanged,
    /// No counterpart in the base.
    Added,
    /// Edited from a base line. `words` are the changed parts, in UTF-8
    /// bytes from the line's start; empty when only whitespace changed or
    /// there is no base to compare with.
    Edited { words: Vec<Range<usize>> },
}

/// Lines below this similarity (0 to 1) are not paired: the side line is
/// added rather than edited.
const MIN_SIMILARITY: f64 = 0.5;

/// Above these sizes the tables would cost too much memory or time; the
/// lines then count as edited, or added, without detail.
const MAX_LINE_TABLE: usize = 4_000_000;
const MAX_GAP_PAIRS: usize = 40_000;
const MAX_TOKEN_TABLE: usize = 250_000;

/// The change of each line of `side` (as `str::lines` splits it) against
/// `base`. Without a base, every line is edited with no word marked: the
/// side differs, but from what is unknown.
pub fn line_changes(base: Option<&str>, side: &str) -> Vec<LineChange> {
    let side: Vec<&str> = side.lines().collect();
    let Some(base) = base else {
        return vec![LineChange::Edited { words: Vec::new() }; side.len()];
    };
    let base: Vec<&str> = base.lines().collect();

    let mut changes = vec![LineChange::Added; side.len()];
    let matches = common_subsequence(&base, &side, MAX_LINE_TABLE);
    // Walk the gaps between matches, plus a sentinel match after the end.
    let mut base_start = 0;
    let mut side_start = 0;
    let ends = matches
        .iter()
        .copied()
        .chain(std::iter::once((base.len(), side.len())));
    for (base_end, side_end) in ends {
        let pairs = pair_gap(&base[base_start..base_end], &side[side_start..side_end]);
        for (base_index, side_index) in pairs {
            let base_line = base[base_start + base_index];
            let side_line = side[side_start + side_index];
            changes[side_start + side_index] = LineChange::Edited {
                words: changed_words(base_line, side_line),
            };
        }
        if side_end < side.len() {
            changes[side_end] = LineChange::Unchanged;
        }
        base_start = base_end + 1;
        side_start = side_end + 1;
    }
    changes
}

/// The parts of `line` that differ from `base_line`, ignoring whitespace.
/// Changed words separated only by whitespace form one range.
pub fn changed_words(base_line: &str, line: &str) -> Vec<Range<usize>> {
    let base_tokens = words_of(base_line);
    let tokens = words_of(line);
    let base_texts: Vec<&str> = base_tokens
        .iter()
        .map(|range| &base_line[range.clone()])
        .collect();
    let texts: Vec<&str> = tokens.iter().map(|range| &line[range.clone()]).collect();
    if base_texts.len() * texts.len() > MAX_TOKEN_TABLE {
        return whole(line);
    }
    let mut kept = vec![false; tokens.len()];
    for (_, index) in common_subsequence(&base_texts, &texts, MAX_TOKEN_TABLE) {
        kept[index] = true;
    }

    let mut ranges: Vec<Range<usize>> = Vec::new();
    for (token, kept) in tokens.into_iter().zip(kept) {
        if kept {
            continue;
        }
        match ranges.last_mut() {
            Some(last) if line[last.end..token.start].trim().is_empty() => last.end = token.end,
            _ => ranges.push(token),
        }
    }
    // Two neighbouring ranges joined above may still be split by a kept
    // token; that is right, the kept token did not change.
    ranges
}

/// The line without its surrounding whitespace, as one range.
fn whole(line: &str) -> Vec<Range<usize>> {
    let start = line.len() - line.trim_start().len();
    let end = line.trim_end().len();
    if start < end {
        std::iter::once(start..end).collect()
    } else {
        Vec::new()
    }
}

/// Words of a line: runs of letters, digits and `_`, and every other
/// non-space character on its own. Whitespace is left out.
fn words_of(line: &str) -> Vec<Range<usize>> {
    let mut words: Vec<Range<usize>> = Vec::new();
    let mut in_word = false;
    for (index, c) in line.char_indices() {
        let end = index + c.len_utf8();
        if c.is_whitespace() {
            in_word = false;
        } else if c.is_alphanumeric() || c == '_' {
            match words.last_mut() {
                Some(last) if in_word => last.end = end,
                _ => words.push(index..end),
            }
            in_word = true;
        } else {
            words.push(index..end);
            in_word = false;
        }
    }
    words
}

/// How alike two lines are, from 0 to 1: twice the length of their common
/// words over their total length, whitespace left out.
fn similarity(a: &str, b: &str) -> f64 {
    let a_texts: Vec<&str> = words_of(a).into_iter().map(|range| &a[range]).collect();
    let b_texts: Vec<&str> = words_of(b).into_iter().map(|range| &b[range]).collect();
    let total: usize = a_texts.iter().chain(&b_texts).map(|text| text.len()).sum();
    if total == 0 {
        return 1.0;
    }
    if a_texts.len() * b_texts.len() > MAX_TOKEN_TABLE {
        return 0.0;
    }
    let common: usize = common_subsequence(&a_texts, &b_texts, MAX_TOKEN_TABLE)
        .into_iter()
        .map(|(index, _)| a_texts[index].len())
        .sum();
    2.0 * common as f64 / total as f64
}

/// Pairs the lines of a gap in order, maximising their total similarity;
/// returns (base index, side index) pairs. Too large a gap gets no pair.
fn pair_gap(base: &[&str], side: &[&str]) -> Vec<(usize, usize)> {
    if base.is_empty() || side.is_empty() || base.len() * side.len() > MAX_GAP_PAIRS {
        return Vec::new();
    }
    let rows = base.len();
    let columns = side.len();
    let mut similar = vec![0.0_f64; rows * columns];
    for i in 0..rows {
        for j in 0..columns {
            similar[i * columns + j] = similarity(base[i], side[j]);
        }
    }
    // best[i][j]: the best total for base[i..] and side[j..].
    let width = columns + 1;
    let mut best = vec![0.0_f64; (rows + 1) * width];
    for i in (0..rows).rev() {
        for j in (0..columns).rev() {
            let mut value = best[(i + 1) * width + j].max(best[i * width + j + 1]);
            let score = similar[i * columns + j];
            if score >= MIN_SIMILARITY {
                value = value.max(score + best[(i + 1) * width + j + 1]);
            }
            best[i * width + j] = value;
        }
    }
    let mut pairs = Vec::new();
    let (mut i, mut j) = (0, 0);
    while i < rows && j < columns {
        let score = similar[i * columns + j];
        if score >= MIN_SIMILARITY && best[i * width + j] == score + best[(i + 1) * width + j + 1] {
            pairs.push((i, j));
            i += 1;
            j += 1;
        } else if best[i * width + j] == best[(i + 1) * width + j] {
            i += 1;
        } else {
            j += 1;
        }
    }
    pairs
}

/// A longest common subsequence of `a` and `b`, as (index in a, index in b)
/// pairs in order.
///
/// The common prefix and suffix are matched first, as diff tools do: when a
/// token could match in two places (the `)` of `send(item)` against the two
/// of `send(item, timeout: .seconds(30))`), this keeps the insertion in one
/// piece. The middle gets no pair when its table would exceed `max_table`
/// cells.
fn common_subsequence(a: &[&str], b: &[&str], max_table: usize) -> Vec<(usize, usize)> {
    let mut prefix = 0;
    while prefix < a.len() && prefix < b.len() && a[prefix] == b[prefix] {
        prefix += 1;
    }
    let mut suffix = 0;
    while suffix < a.len() - prefix
        && suffix < b.len() - prefix
        && a[a.len() - 1 - suffix] == b[b.len() - 1 - suffix]
    {
        suffix += 1;
    }

    let mut pairs: Vec<(usize, usize)> = (0..prefix).map(|index| (index, index)).collect();
    let middle_a = &a[prefix..a.len() - suffix];
    let middle_b = &b[prefix..b.len() - suffix];
    for (i, j) in middle_subsequence(middle_a, middle_b, max_table) {
        pairs.push((prefix + i, prefix + j));
    }
    for index in 0..suffix {
        pairs.push((a.len() - suffix + index, b.len() - suffix + index));
    }
    pairs
}

/// The dynamic-programming part of `common_subsequence`.
fn middle_subsequence(a: &[&str], b: &[&str], max_table: usize) -> Vec<(usize, usize)> {
    let width = b.len() + 1;
    if (a.len() + 1) * width > max_table {
        return Vec::new();
    }
    // length[i][j]: the length of the longest common subsequence of a[i..]
    // and b[j..].
    let mut length = vec![0u32; (a.len() + 1) * width];
    for i in (0..a.len()).rev() {
        for j in (0..b.len()).rev() {
            length[i * width + j] = if a[i] == b[j] {
                length[(i + 1) * width + j + 1] + 1
            } else {
                length[(i + 1) * width + j].max(length[i * width + j + 1])
            };
        }
    }
    let mut pairs = Vec::new();
    let (mut i, mut j) = (0, 0);
    while i < a.len() && j < b.len() {
        if a[i] == b[j] {
            pairs.push((i, j));
            i += 1;
            j += 1;
        } else if length[(i + 1) * width + j] >= length[i * width + j + 1] {
            i += 1;
        } else {
            j += 1;
        }
    }
    pairs
}

#[cfg(test)]
mod tests {
    use super::*;

    fn marked(line: &str, words: &[Range<usize>]) -> Vec<String> {
        words
            .iter()
            .map(|range| line[range.clone()].to_string())
            .collect()
    }

    /// The design's example: an argument added to a call is one changed range, not several
    /// pieces.
    #[test]
    fn added_argument_is_one_range() {
        let base = "        try await client.send(item)";
        let ours = "        try await client.send(item, timeout: .seconds(30))";
        assert_eq!(
            marked(ours, &changed_words(base, ours)),
            [", timeout: .seconds(30)"]
        );
    }

    /// A call wrapped in a retry: the call is paired with its base line (edited, indentation
    /// only, no word marked) and the wrapping lines are added.
    #[test]
    fn wrapped_call_pairs_the_similar_line() {
        let base = "        try await client.send(item)\n";
        let theirs = "        try await retry(times: 3, backoff: .exponential) {\n            try await client.send(item)\n        }\n";
        assert_eq!(
            line_changes(Some(base), theirs),
            [
                LineChange::Added,
                LineChange::Edited { words: Vec::new() },
                LineChange::Added,
            ]
        );
    }

    /// Lines equal to a base line are unchanged; a new line between them is added.
    #[test]
    fn equal_lines_are_unchanged_and_new_ones_added() {
        let base = "a()\nb()\n";
        let side = "a()\nx = 1\nb()\n";
        assert_eq!(
            line_changes(Some(base), side),
            [
                LineChange::Unchanged,
                LineChange::Added,
                LineChange::Unchanged
            ]
        );
    }

    /// An empty base (both sides added code) makes every line added; without a base the
    /// lines are edited with no word marked, since what they changed from is unknown.
    #[test]
    fn empty_base_adds_every_line_and_no_base_edits_them() {
        let side = "func cancel() {\n    queue.cancelAll()\n";
        assert_eq!(
            line_changes(Some(""), side),
            [LineChange::Added, LineChange::Added]
        );
        assert_eq!(
            line_changes(None, side),
            [
                LineChange::Edited { words: Vec::new() },
                LineChange::Edited { words: Vec::new() },
            ]
        );
    }

    /// A renamed identifier is marked alone, not the whole line.
    #[test]
    fn renamed_word_inside_a_line() {
        let base = "let total = price * count";
        let side = "let total = price * quantity";
        assert_eq!(marked(side, &changed_words(base, side)), ["quantity"]);
    }

    /// Changed ranges are valid UTF-8 boundaries, so UIs can slice text with accents.
    #[test]
    fn ranges_are_utf8_boundaries() {
        let base = "let é = \"à\"";
        let side = "let é = \"àb\"";
        let words = changed_words(base, side);
        assert_eq!(marked(side, &words), ["àb"]);
    }

    /// A side line unlike any base line is added rather than paired as edited.
    #[test]
    fn dissimilar_lines_are_added_not_edited() {
        let base = "return cache[key]\n";
        let side = "logger.debug(\"miss\")\n";
        assert_eq!(line_changes(Some(base), side), [LineChange::Added]);
    }
}
