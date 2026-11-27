//! A file's rows checked against git on generated pairs of files, and
//! against their own rules on generated patches.
//!
//! Git defines which lines changed, so the rows must account for the two
//! files exactly and count as `git diff --numstat` does, on every input
//! and not only on the cases written by hand in `diff.rs`.

use std::fs;
use std::process::Command as Process;

use proptest::prelude::*;
use scry_core::diff::{self, FileDiff, LineKind, Marks, RowKind, TextDiff};
use scry_core::git::GitCli;

/// How many pairs the oracle generates: 150, or `SCRY_ORACLE_CASES` for a
/// deeper run before a release. Every case is deterministic: a failure
/// names its seed, which reproduces it.
fn cases() -> u64 {
    std::env::var("SCRY_ORACLE_CASES")
        .ok()
        .and_then(|cases| cases.parse().ok())
        .unwrap_or(150)
}

/// A small deterministic random generator (xorshift), so that the cases are
/// the same on every run and every platform.
struct Random(u64);

impl Random {
    fn new(seed: u64) -> Random {
        Random(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }

    fn below(&mut self, bound: usize) -> usize {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 % bound as u64) as usize
    }
}

/// Lines from a small alphabet, so that the two files share lines, some
/// alike enough to be paired, and some that look like a patch's own
/// lines.
const ALPHABET: [&str; 12] = [
    "a",
    "b",
    "c",
    "",
    "x y",
    "let timeout = 30",
    "let timeout = 60",
    "func upload(item) {",
    "func upload(item, retries) {",
    "-- removed?",
    "++ added?",
    "@@ -1 +1 @@",
];

/// An old file of up to 12 lines and a new one edited from it by a few
/// deletions, insertions and changes; either may lack its final line
/// ending.
fn pair(seed: u64) -> (String, String) {
    let mut random = Random::new(seed);
    let line = |random: &mut Random| ALPHABET[random.below(ALPHABET.len())].to_string();
    let old: Vec<String> = (0..random.below(13)).map(|_| line(&mut random)).collect();
    let mut new = old.clone();
    for _ in 0..random.below(6) {
        let at = random.below(new.len() + 1);
        match random.below(3) {
            0 if at < new.len() => {
                new.remove(at);
            }
            1 if at < new.len() => new[at] = line(&mut random),
            _ => new.insert(at, line(&mut random)),
        }
    }
    let mut text = |lines: &[String]| {
        let mut text = lines.join("\n");
        // An empty last line with no line ending would be no line at all.
        let ends = lines.last().is_none_or(|last| last.is_empty()) || random.below(4) > 0;
        if ends && !lines.is_empty() {
            text.push('\n');
        }
        text
    };
    (text(&old), text(&new))
}

/// A text's lines as Scry gives them: without line endings.
fn lines_of(text: &str) -> Vec<String> {
    text.lines().map(String::from).collect()
}

/// What must hold for any text diff, whatever produced it: both sides are
/// accounted for line by line, once and in order, by the rows and by the
/// unified view, and the marks stay inside their lines.
fn check(text: &TextDiff, old: &[String], new: &[String]) -> Result<(), String> {
    if text.old_lines != old {
        return Err(format!("old lines {:?}, expected {old:?}", text.old_lines));
    }
    if text.new_lines != new {
        return Err(format!("new lines {:?}, expected {new:?}", text.new_lines));
    }
    let numbers = |side: &[Option<usize>], count: usize, what: &str| {
        let seen: Vec<usize> = side.iter().flatten().copied().collect();
        if seen == (1..=count).collect::<Vec<_>>() {
            Ok(())
        } else {
            Err(format!("{what} numbers {seen:?} for {count} lines"))
        }
    };
    let rows_old: Vec<_> = text.rows.iter().map(|row| row.old).collect();
    let rows_new: Vec<_> = text.rows.iter().map(|row| row.new).collect();
    numbers(&rows_old, old.len(), "old row")?;
    numbers(&rows_new, new.len(), "new row")?;
    for row in &text.rows {
        let right_sides = match row.kind {
            RowKind::Unchanged => {
                row.old.is_some()
                    && row.new.is_some()
                    && old[row.old.unwrap() - 1] == new[row.new.unwrap() - 1]
            }
            RowKind::Changed => row.old.is_some() && row.new.is_some(),
            RowKind::Removed => row.old.is_some() && row.new.is_none(),
            RowKind::Added => row.old.is_none() && row.new.is_some(),
        };
        if !right_sides {
            return Err(format!("row {row:?} does not fit its kind"));
        }
        let marks = [
            (&row.old_words, row.old, old),
            (&row.new_words, row.new, new),
        ];
        for (words, number, lines) in marks {
            let Some(number) = number else {
                if !words.is_empty() {
                    return Err(format!("row {row:?} marks a line it does not have"));
                }
                continue;
            };
            let line = &lines[number - 1];
            let mut position = 0;
            for word in words {
                if word.start < position
                    || word.start >= word.end
                    || line.get(word.clone()).is_none()
                {
                    return Err(format!("marks {words:?} do not fit {line:?}"));
                }
                position = word.end;
            }
        }
    }

    let unified = text.unified();
    let unified_old: Vec<_> = unified.iter().map(|line| line.old).collect();
    let unified_new: Vec<_> = unified.iter().map(|line| line.new).collect();
    numbers(&unified_old, old.len(), "old unified")?;
    numbers(&unified_new, new.len(), "new unified")?;
    for line in &unified {
        let fits = match line.kind {
            LineKind::Unchanged => line.old.is_some() && line.new.is_some(),
            LineKind::Removed => line.old.is_some() && line.new.is_none(),
            LineKind::Added => line.old.is_none() && line.new.is_some(),
        };
        if !fits {
            return Err(format!("unified line {line:?} does not fit its kind"));
        }
    }
    // In a block the removed lines come before the added ones.
    for pair in unified.windows(2) {
        if pair[0].kind == LineKind::Added && pair[1].kind == LineKind::Removed {
            return Err(String::from("an added line comes before a removed one"));
        }
    }

    let changed_rows = text
        .rows
        .iter()
        .filter(|row| row.kind != RowKind::Unchanged)
        .count();
    let changes = text.changes();
    if changes.iter().map(|change| change.row_count).sum::<usize>() != changed_rows {
        return Err(format!(
            "changes {changes:?} for {changed_rows} changed rows"
        ));
    }
    for change in &changes {
        let rows = &text.rows[change.first_row..change.first_row + change.row_count];
        let before = change.first_row.checked_sub(1).map(|at| text.rows[at].kind);
        let after = text
            .rows
            .get(change.first_row + change.row_count)
            .map(|row| row.kind);
        if rows.iter().any(|row| row.kind == RowKind::Unchanged)
            || before.is_some_and(|kind| kind != RowKind::Unchanged)
            || after.is_some_and(|kind| kind != RowKind::Unchanged)
        {
            return Err(format!("change {change:?} is not a whole block"));
        }
    }
    Ok(())
}

/// On generated pairs of files, the rows hold exactly the two files' lines
/// and count added and removed lines as `git diff --numstat` does, with
/// and without the pairing of like lines.
#[test]
fn rows_account_for_both_files_and_count_as_git_does() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("gitconfig"), "").unwrap();
    let config = dir.path().join("gitconfig").display().to_string();
    let git = GitCli::new(dir.path().to_path_buf())
        .with_env("GIT_CONFIG_GLOBAL", &config)
        .with_env("GIT_CONFIG_NOSYSTEM", "1");
    let (old_path, new_path) = (dir.path().join("old"), dir.path().join("new"));
    for seed in 0..cases() {
        let (old, new) = pair(seed);
        fs::write(&old_path, &old).unwrap();
        fs::write(&new_path, &new).unwrap();
        let numstat = Process::new("git")
            .current_dir(dir.path())
            .env("GIT_CONFIG_GLOBAL", &config)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .args(["diff", "--no-index", "--numstat", "--", "old", "new"])
            .output()
            .unwrap();
        let numstat = String::from_utf8(numstat.stdout).unwrap();
        let mut counts = numstat.split('\t').map(|count| count.parse::<usize>().ok());
        let expected = match (counts.next().flatten(), counts.next().flatten()) {
            (Some(added), Some(removed)) => (added, removed),
            // No line of output: the files are the same.
            _ => (0, 0),
        };
        for marks in [Marks::Auto, Marks::Always] {
            let diff = diff::plain_files(&git, &old_path, &new_path, marks).unwrap();
            let FileDiff::Text(text) = diff else {
                panic!("seed {seed}: not a text diff");
            };
            if let Err(problem) = check(&text, &lines_of(&old), &lines_of(&new)) {
                panic!("seed {seed}: {problem}\nold {old:?}\nnew {new:?}");
            }
            assert_eq!(
                text.counts(),
                expected,
                "seed {seed}\nold {old:?}\nnew {new:?}"
            );
            assert_eq!(
                (
                    text.old_missing_final_newline,
                    text.new_missing_final_newline
                ),
                (
                    !old.is_empty() && !old.ends_with('\n'),
                    !new.is_empty() && !new.ends_with('\n')
                ),
                "seed {seed}\nold {old:?}\nnew {new:?}"
            );
        }
    }
}

/// One line of a generated patch: on both sides, the old one or the new.
fn patch_line() -> impl Strategy<Value = (char, String)> {
    (
        prop::sample::select(vec![' ', ' ', '-', '+']),
        prop::sample::select(ALPHABET.to_vec()),
    )
        .prop_map(|(sign, line)| (sign, line.to_string()))
}

proptest! {
    /// Whatever lines a patch holds, in whatever order, its rows account
    /// for every one of them: nothing git printed is lost, doubled or
    /// moved to the other side, with or without the marks.
    #[test]
    fn any_patch_gives_rows_for_all_its_lines(
        lines in prop::collection::vec(patch_line(), 0..40),
        always in any::<bool>(),
    ) {
        let mut patch = String::from("diff --git a/f b/f\n--- a/f\n+++ b/f\n@@ -1 +1 @@\n");
        for (sign, line) in &lines {
            patch.push_str(&format!("{sign}{line}\n"));
        }
        let side = |other: char| -> Vec<String> {
            lines
                .iter()
                .filter(|(sign, _)| *sign != other)
                .map(|(_, line)| line.clone())
                .collect()
        };
        let marks = if always { Marks::Always } else { Marks::Auto };
        let text = TextDiff::from_patch(diff::parse_patch(&patch), marks);
        if let Err(problem) = check(&text, &side('+'), &side('-')) {
            return Err(TestCaseError::fail(problem));
        }
    }
}
