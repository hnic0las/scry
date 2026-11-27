//! Word-level changes: which lines of a conflict side were added or edited
//! against the base, and which words changed in an edited line.
//!
//! Lines equal to a base line are matched first (longest common
//! subsequence). Between two matches, each remaining side line is paired
//! with the most similar remaining base line, in order; a side line with no
//! similar base line is added. Whitespace never counts as a changed word, so
//! a re-indented line is edited with no word marked.

use serde::Serialize;
use std::ops::Range;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
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
/// The most token comparisons a gap's pairing may cost with exact
/// similarities (the words' longest common subsequence for every pair of
/// lines). A larger gap, such as 150 long lines edited on each side, is
/// paired by shared words regardless of order, which is linear.
const MAX_GAP_TOKEN_WORK: usize = 4_000_000;

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

/// A line's words as text (`words_of`).
fn tokens(line: &str) -> Vec<&str> {
    words_of(line)
        .into_iter()
        .map(|range| &line[range])
        .collect()
}

/// A line's words with how often each occurs, and their total length.
struct Bag<'a> {
    counts: std::collections::HashMap<&'a str, usize>,
    length: usize,
}

impl<'a> Bag<'a> {
    fn of(tokens: &[&'a str]) -> Bag<'a> {
        let mut counts = std::collections::HashMap::new();
        for token in tokens {
            *counts.entry(*token).or_default() += 1;
        }
        Bag {
            counts,
            length: tokens.iter().map(|token| token.len()).sum(),
        }
    }

    /// Like `similarity`, counting the words both lines have whatever their
    /// order: an upper bound of it, in linear time.
    fn shared(&self, other: &Bag) -> f64 {
        let total = self.length + other.length;
        if total == 0 {
            return 1.0;
        }
        let common: usize = self
            .counts
            .iter()
            .map(|(token, count)| token.len() * count.min(other.counts.get(token).unwrap_or(&0)))
            .sum();
        2.0 * common as f64 / total as f64
    }
}

/// How alike two lines are, from 0 to 1: twice the length of their common
/// words over their total length, whitespace left out.
fn similarity(a_texts: &[&str], b_texts: &[&str]) -> f64 {
    let total: usize = a_texts.iter().chain(b_texts).map(|text| text.len()).sum();
    if total == 0 {
        return 1.0;
    }
    if a_texts.len() * b_texts.len() > MAX_TOKEN_TABLE {
        return 0.0;
    }
    let common: usize = common_subsequence(a_texts, b_texts, MAX_TOKEN_TABLE)
        .into_iter()
        .map(|(index, _)| a_texts[index].len())
        .sum();
    2.0 * common as f64 / total as f64
}

/// The lines of two runs that are alike, as (index in `base`, index in
/// `side`) pairs in order: what a diff shows as one line edited rather than
/// one removed and one added. Too large a pair of runs gets no pair.
pub fn paired_lines(base: &[&str], side: &[&str]) -> Vec<(usize, usize)> {
    pair_gap(base, side)
}

/// Pairs the lines of a gap in order, maximising their total similarity;
/// returns (base index, side index) pairs. Too large a gap gets no pair.
fn pair_gap(base: &[&str], side: &[&str]) -> Vec<(usize, usize)> {
    if base.is_empty() || side.is_empty() || base.len() * side.len() > MAX_GAP_PAIRS {
        return Vec::new();
    }
    let rows = base.len();
    let columns = side.len();
    let base_tokens: Vec<Vec<&str>> = base.iter().map(|line| tokens(line)).collect();
    let side_tokens: Vec<Vec<&str>> = side.iter().map(|line| tokens(line)).collect();
    let base_count: usize = base_tokens.iter().map(Vec::len).sum();
    let side_count: usize = side_tokens.iter().map(Vec::len).sum();
    let mut similar = vec![0.0_f64; rows * columns];
    if base_count * side_count > MAX_GAP_TOKEN_WORK {
        let base_bags: Vec<Bag> = base_tokens.iter().map(|tokens| Bag::of(tokens)).collect();
        let side_bags: Vec<Bag> = side_tokens.iter().map(|tokens| Bag::of(tokens)).collect();
        for i in 0..rows {
            for j in 0..columns {
                similar[i * columns + j] = base_bags[i].shared(&side_bags[j]);
            }
        }
    } else {
        for i in 0..rows {
            for j in 0..columns {
                similar[i * columns + j] = similarity(&base_tokens[i], &side_tokens[j]);
            }
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

    /// A gap of 150 long lines edited on each side is paired by shared words, in linear
    /// time per pair: the exact pairing took seconds per call. Each line is still paired
    /// with its base line and its changed word marked. Shared words ignore their order, so
    /// a line with its words reversed is still edited; a line sharing none of its words,
    /// or only a few, is added.
    #[test]
    fn a_large_gap_is_paired_by_shared_words() {
        let words = |line: usize| -> Vec<String> {
            (0..100).map(|word| format!("w{line}x{word}")).collect()
        };
        let base: String = (0..150)
            .map(|line| format!("let v{line} = [{}, old]\n", words(line).join(", ")))
            .collect();
        let ours: String = (0..150)
            .map(|line| match line {
                1 => {
                    let mut reversed = words(line);
                    reversed.reverse();
                    format!("let v{line} = [{}, old]\n", reversed.join(", "))
                }
                2 => format!("let other = [{}]\n", words(1000).join(", ")),
                3 => {
                    let mixed: Vec<String> = (0..100)
                        .map(|word| match word % 4 {
                            0 => format!("w{line}x{word}"),
                            _ => format!("n{word}"),
                        })
                        .collect();
                    format!("{}\n", mixed.join(" "))
                }
                _ => format!("let v{line} = [{}, new]\n", words(line).join(", ")),
            })
            .collect();
        let changes = line_changes(Some(&base), &ours);
        assert_eq!(changes.len(), 150);
        assert!(
            matches!(changes[1], LineChange::Edited { .. }),
            "{:?}",
            changes[1]
        );
        assert_eq!(changes[2], LineChange::Added);
        assert_eq!(changes[3], LineChange::Added);
        for (index, line) in ours
            .lines()
            .enumerate()
            .filter(|(index, _)| ![1, 2, 3].contains(index))
        {
            let LineChange::Edited { words } = &changes[index] else {
                panic!("line {index} not paired: {:?}", changes[index]);
            };
            assert_eq!(marked(line, words), ["new"], "line {index}");
        }
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

    /// After an unchanged line, an edited line is still compared with its own base line,
    /// not the first one: the gap's positions are offset by the lines before it.
    #[test]
    fn edited_line_after_unchanged_ones_is_compared_with_its_base_line() {
        let base = "let a = 1\nlet b = 2\nlet c = old(3)\n";
        let side = "let a = 1\nlet b = 2\nlet c = new(3)\nlet d = 4\n";
        let changes = line_changes(Some(base), side);
        assert_eq!(changes[0], LineChange::Unchanged);
        assert_eq!(changes[1], LineChange::Unchanged);
        let LineChange::Edited { words } = &changes[2] else {
            panic!("not paired: {:?}", changes[2]);
        };
        assert_eq!(marked("let c = new(3)", words), ["new"]);
        assert_eq!(changes[3], LineChange::Added);
    }

    /// A matched line belongs to neither gap around it: a new side line that resembles the
    /// unchanged line just above is added, and the unchanged line is not paired with an
    /// edited base line below it.
    #[test]
    fn unchanged_lines_stay_out_of_the_next_gap() {
        assert_eq!(
            line_changes(Some("x = 1\nfoo bar\n"), "x = 1\nx = 2\n"),
            [LineChange::Unchanged, LineChange::Added]
        );
        assert_eq!(
            line_changes(Some("x = 1\nx = 9\n"), "x = 1\nqqq\n"),
            [LineChange::Unchanged, LineChange::Added]
        );
    }

    /// A blank line has no whole-line range to mark.
    #[test]
    fn a_blank_line_has_no_whole_range() {
        assert_eq!(whole(" \t "), Vec::<Range<usize>>::new());
        assert_eq!(whole(""), Vec::<Range<usize>>::new());
        assert_eq!(marked(" ab ", &whole(" ab ")), ["ab"]);
    }

    /// Changed words are joined only across whitespace: an unchanged word between them
    /// keeps them apart, so the UI does not mark it.
    #[test]
    fn an_unchanged_word_splits_changed_ranges() {
        let side = "x = keep + y";
        assert_eq!(
            marked(side, &changed_words("a = keep + b", side)),
            ["x", "y"]
        );
    }

    /// Two lines too long to compare word by word (more than 250,000 word pairs) mark the
    /// whole line, without its surrounding whitespace; at the limit they are still
    /// compared word by word.
    #[test]
    fn lines_over_the_word_table_mark_the_whole_line() {
        let line = |count: usize, last: &str| -> String {
            let words: Vec<String> = (1..count).map(|word| format!("w{word}")).collect();
            format!("  {} {last}  ", words.join(" "))
        };
        let side = line(501, "new");
        assert_eq!(
            marked(&side, &changed_words(&line(500, "old"), &side)),
            [side.trim()]
        );
        let side = line(500, "new");
        assert_eq!(
            marked(&side, &changed_words(&line(500, "old"), &side)),
            ["new"]
        );
        assert_eq!(changed_words("   ", " \t "), Vec::<Range<usize>>::new());
    }

    /// Similarity counts the common words in order, weighted by their length; the bag of
    /// shared words ignores order, so it is an upper bound used on large gaps.
    #[test]
    fn similarity_weighs_words_in_order() {
        assert_eq!(similarity(&["abc", "d"], &["abc", "e"]), 6.0 / 8.0);
        assert_eq!(similarity(&["a", "b"], &["b", "a"]), 0.5);
        assert_eq!(similarity(&[], &[]), 1.0);
        let bag = |tokens: &[&'static str]| Bag::of(tokens);
        assert_eq!(bag(&["a", "b"]).shared(&bag(&["b", "a"])), 1.0);
        assert_eq!(bag(&["abc", "d"]).shared(&bag(&["e", "abc"])), 6.0 / 8.0);
        assert_eq!(
            bag(&["a", "a", "b"]).shared(&bag(&["a", "c", "d"])),
            2.0 / 6.0
        );
        assert_eq!(bag(&[]).shared(&bag(&[])), 1.0);
        // Lines over the word table are not compared at all.
        let long: Vec<&str> = vec!["w"; 501];
        assert_eq!(similarity(&long, &long[..500]), 0.0);
        assert_eq!(similarity(&long[..500], &long[..500]), 1.0);
    }

    /// Sequences of up to 5 tokens over a 3-letter alphabet, in a fixed order.
    fn short_sequences() -> Vec<Vec<&'static str>> {
        let mut all: Vec<Vec<&'static str>> = vec![Vec::new()];
        let mut last: Vec<Vec<&'static str>> = vec![Vec::new()];
        for _ in 0..5 {
            last = last
                .iter()
                .flat_map(|sequence| {
                    ["a", "b", "c"].into_iter().map(move |token| {
                        let mut longer = sequence.clone();
                        longer.push(token);
                        longer
                    })
                })
                .collect();
            all.extend(last.iter().cloned());
        }
        all
    }

    /// The length of a longest common subsequence, computed over prefixes (the code
    /// computes it over suffixes), as an independent reference.
    fn reference_lcs_length(a: &[&str], b: &[&str]) -> usize {
        let mut length = vec![vec![0usize; b.len() + 1]; a.len() + 1];
        for i in 1..=a.len() {
            for j in 1..=b.len() {
                length[i][j] = if a[i - 1] == b[j - 1] {
                    length[i - 1][j - 1] + 1
                } else {
                    length[i - 1][j].max(length[i][j - 1])
                };
            }
        }
        length[a.len()][b.len()]
    }

    /// Whether `pairs` match equal tokens of `a` and `b`, strictly increasing on both sides.
    fn is_common_subsequence(a: &[&str], b: &[&str], pairs: &[(usize, usize)]) -> bool {
        pairs
            .iter()
            .all(|&(i, j)| i < a.len() && j < b.len() && a[i] == b[j])
            && pairs
                .windows(2)
                .all(|two| two[0].0 < two[1].0 && two[0].1 < two[1].1)
    }

    /// On every pair of short token sequences, the common subsequence is valid and as long
    /// as possible: a shorter one would mark words as changed that did not change. The
    /// common prefix and suffix are taken first, so only an exhaustive set reaches the
    /// table in the middle with every shape.
    #[test]
    fn common_subsequence_is_a_longest_one() {
        let sequences = short_sequences();
        for a in &sequences {
            for b in &sequences {
                let pairs = common_subsequence(a, b, usize::MAX);
                assert!(
                    is_common_subsequence(a, b, &pairs),
                    "{a:?} {b:?}: {pairs:?}"
                );
                assert_eq!(pairs.len(), reference_lcs_length(a, b), "{a:?} {b:?}");
                let middle = middle_subsequence(a, b, usize::MAX);
                assert!(
                    is_common_subsequence(a, b, &middle),
                    "{a:?} {b:?}: {middle:?}"
                );
                assert_eq!(middle.len(), pairs.len(), "{a:?} {b:?}");
            }
        }
    }

    /// Over the table limit the middle gets no pair, but the common prefix and suffix are
    /// still matched; at the limit the middle is still computed.
    #[test]
    fn common_subsequence_over_the_table_keeps_prefix_and_suffix() {
        let a = ["x", "a", "b", "y"];
        let b = ["x", "b", "a", "y"];
        // The middle, a b against b a, needs a 3 by 3 table.
        assert_eq!(common_subsequence(&a, &b, 8), [(0, 0), (3, 3)]);
        assert_eq!(common_subsequence(&a, &b, 9).len(), 3);
        assert_eq!(middle_subsequence(&["a"], &["a"], 3), []);
        assert_eq!(middle_subsequence(&["a"], &["a"], 4), [(0, 0)]);
    }

    /// (base, side) line pairs.
    type Pairing = Vec<(usize, usize)>;

    /// Every set of (base, side) pairs that keeps both orders, as a brute-force reference.
    fn monotone_pairings(rows: usize, columns: usize) -> Vec<Pairing> {
        let cells: Vec<(usize, usize)> = (0..rows)
            .flat_map(|i| (0..columns).map(move |j| (i, j)))
            .collect();
        (0u32..1 << cells.len())
            .map(|mask| {
                cells
                    .iter()
                    .enumerate()
                    .filter(|(bit, _)| mask & (1 << bit) != 0)
                    .map(|(_, cell)| *cell)
                    .collect::<Vec<_>>()
            })
            .filter(|pairs| {
                pairs
                    .windows(2)
                    .all(|two| two[0].0 < two[1].0 && two[0].1 < two[1].1)
            })
            .collect()
    }

    /// On every gap of up to 3 lines on each side, drawn from lines of varied likeness, the
    /// pairing keeps the order of both sides, pairs only similar enough lines, and reaches
    /// the best total similarity that any such pairing can.
    #[test]
    fn gap_pairing_is_the_best_ordered_one() {
        let lines = ["a b c d", "a b c x", "c d", "x y z", "z y x", ""];
        let mut gaps: Vec<Vec<&str>> = vec![Vec::new()];
        for size in 1..=3 {
            let mut longer: Vec<Vec<&str>> = vec![Vec::new()];
            for _ in 0..size {
                longer = longer
                    .iter()
                    .flat_map(|gap| {
                        lines.iter().map(move |line| {
                            let mut gap = gap.clone();
                            gap.push(*line);
                            gap
                        })
                    })
                    .collect();
            }
            gaps.extend(longer);
        }
        // Every score and every ordered pairing, computed once: the pairs of gaps are many.
        let index = |line: &str| lines.iter().position(|known| *known == line).unwrap();
        let likeness: Vec<Vec<f64>> = lines
            .iter()
            .map(|a| {
                lines
                    .iter()
                    .map(|b| similarity(&tokens(a), &tokens(b)))
                    .collect()
            })
            .collect();
        let pairings: Vec<Vec<Vec<Pairing>>> = (0..=3)
            .map(|rows| {
                (0..=3)
                    .map(|columns| monotone_pairings(rows, columns))
                    .collect()
            })
            .collect();
        for base in &gaps {
            for side in &gaps {
                let score = |(i, j): (usize, usize)| likeness[index(base[i])][index(side[j])];
                let pairs = pair_gap(base, side);
                assert!(
                    pairs
                        .windows(2)
                        .all(|two| two[0].0 < two[1].0 && two[0].1 < two[1].1),
                    "{base:?} {side:?}: {pairs:?}"
                );
                assert!(
                    pairs.iter().all(|&pair| score(pair) >= MIN_SIMILARITY),
                    "{base:?} {side:?}: {pairs:?}"
                );
                let total: f64 = pairs.iter().map(|&pair| score(pair)).sum();
                let best = pairings[base.len()][side.len()]
                    .iter()
                    .filter(|pairing| pairing.iter().all(|&pair| score(pair) >= MIN_SIMILARITY))
                    .map(|pairing| pairing.iter().map(|&pair| score(pair)).sum::<f64>())
                    .fold(0.0, f64::max);
                assert!(
                    (total - best).abs() < 1e-9,
                    "{base:?} {side:?}: {pairs:?} {total} < {best}"
                );
            }
        }
    }

    /// A gap of up to 40,000 line pairs is paired; a larger one is left unpaired (its
    /// lines are added) rather than cost too much.
    #[test]
    fn gaps_over_the_pair_limit_are_not_paired() {
        let lines = |count: usize, word: &str| -> Vec<String> {
            (0..count)
                .map(|line| format!("let value{line} = {word}"))
                .collect()
        };
        let base = lines(200, "old");
        let base: Vec<&str> = base.iter().map(String::as_str).collect();
        let side = lines(200, "new");
        let side: Vec<&str> = side.iter().map(String::as_str).collect();
        assert_eq!(pair_gap(&base, &side).len(), 200);
        let longer = lines(201, "new");
        let longer: Vec<&str> = longer.iter().map(String::as_str).collect();
        assert_eq!(pair_gap(&base, &longer), []);
    }

    mod properties {
        use super::*;
        use proptest::prelude::*;

        /// A line built from pieces that make words, punctuation, spaces and
        /// characters of several bytes, so ranges can fall inside one.
        fn line() -> impl Strategy<Value = String> {
            let pieces = vec![
                "a", "b", "x1", "é", "日本", "_", "(", ")", ",", ".", " ", "  ", "\t", "→",
            ];
            prop::collection::vec(prop::sample::select(pieces), 0..12)
                .prop_map(|pieces| pieces.concat())
        }

        /// Ranges of `line` as the editor marks them: in order, apart, on
        /// character boundaries, neither empty nor starting or ending with
        /// whitespace, each word wholly in or wholly out.
        fn check_ranges(line: &str, ranges: &[Range<usize>]) -> Result<(), TestCaseError> {
            for range in ranges {
                prop_assert!(
                    range.start < range.end && range.end <= line.len(),
                    "{:?}",
                    range
                );
                prop_assert!(
                    line.is_char_boundary(range.start) && line.is_char_boundary(range.end)
                );
                let text = &line[range.clone()];
                prop_assert_eq!(text.trim(), text);
            }
            for two in ranges.windows(2) {
                prop_assert!(two[0].end < two[1].start, "{:?}", two);
            }
            for word in words_of(line) {
                let inside = ranges
                    .iter()
                    .any(|range| range.start <= word.start && word.end <= range.end);
                let outside = ranges
                    .iter()
                    .all(|range| word.end <= range.start || range.end <= word.start);
                prop_assert!(inside || outside, "word {:?} cut by {:?}", word, ranges);
            }
            Ok(())
        }

        proptest! {
            #![proptest_config(ProptestConfig::with_cases(512))]

            /// On any pair of lines, the marked ranges are ones the editor
            /// can paint, and the words left unmarked are as many as the
            /// lines have in common, in order: a word marked although it did
            /// not change, or left unmarked although it did, would mislead
            /// the reader of the diff.
            #[test]
            fn changed_words_mark_exactly_what_changed(base in line(), side in line()) {
                let ranges = changed_words(&base, &side);
                check_ranges(&side, &ranges)?;
                let kept: Vec<&str> = words_of(&side)
                    .into_iter()
                    .filter(|word| ranges.iter().all(|range| word.end <= range.start || range.end <= word.start))
                    .map(|word| &side[word])
                    .collect();
                let base_tokens = tokens(&base);
                prop_assert_eq!(kept.len(), reference_lcs_length(&base_tokens, &tokens(&side)));
                prop_assert_eq!(reference_lcs_length(&base_tokens, &kept), kept.len());
                prop_assert!(changed_words(&side, &side).is_empty());
            }

            /// Any side against any base: one change per side line, as many
            /// unchanged lines as the two have in common, and edited lines
            /// marked with ranges the editor can paint.
            #[test]
            fn line_changes_keep_every_common_line(
                base in prop::collection::vec(line(), 0..8),
                side in prop::collection::vec(line(), 0..8),
            ) {
                let base_text: String = base.iter().map(|line| format!("{line}\n")).collect();
                let side_text: String = side.iter().map(|line| format!("{line}\n")).collect();
                let changes = line_changes(Some(&base_text), &side_text);
                prop_assert_eq!(changes.len(), side.len());
                let base_lines: Vec<&str> = base.iter().map(String::as_str).collect();
                let side_lines: Vec<&str> = side.iter().map(String::as_str).collect();
                let unchanged = changes
                    .iter()
                    .filter(|change| **change == LineChange::Unchanged)
                    .count();
                prop_assert_eq!(unchanged, reference_lcs_length(&base_lines, &side_lines));
                for (change, line) in changes.iter().zip(&side) {
                    if let LineChange::Edited { words } = change {
                        check_ranges(line, words)?;
                    }
                }
            }
        }
    }
}
