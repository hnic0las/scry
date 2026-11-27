//! What `git diff` prints, read as a list of changed files and as one
//! file's patch, on any text: reading never panics, and a patch's rows
//! account for every line of both sides, once and in order.
#![no_main]

use libfuzzer_sys::fuzz_target;
use scry_core::diff::{self, Marks, RowKind, TextDiff};

fuzz_target!(|text: &str| {
    for file in diff::parse_changes(text) {
        assert!(file.path.ends_with(file.name.as_str()));
        assert_eq!(format!("{}{}", file.directory, file.name), file.path);
    }
    for marks in [Marks::Auto, Marks::Always] {
        let rows = TextDiff::from_patch(diff::parse_patch(text), marks);
        let numbers = |side: Vec<Option<usize>>, count: usize| {
            let seen: Vec<usize> = side.into_iter().flatten().collect();
            assert!(seen.iter().copied().eq(1..=count), "{seen:?} for {count}");
        };
        numbers(
            rows.rows.iter().map(|row| row.old).collect(),
            rows.old_lines.len(),
        );
        numbers(
            rows.rows.iter().map(|row| row.new).collect(),
            rows.new_lines.len(),
        );
        for row in &rows.rows {
            if row.kind == RowKind::Unchanged {
                let (old, new) = (row.old.expect("old"), row.new.expect("new"));
                assert_eq!(rows.old_lines[old - 1], rows.new_lines[new - 1]);
            }
            // Marks are ranges of their own line, on character boundaries.
            for (words, number, lines) in [
                (&row.old_words, row.old, &rows.old_lines),
                (&row.new_words, row.new, &rows.new_lines),
            ] {
                for word in words {
                    let line = &lines[number.expect("a marked line") - 1];
                    assert!(line.get(word.clone()).is_some());
                }
            }
        }
        let unified = rows.unified();
        numbers(
            unified.iter().map(|line| line.old).collect(),
            rows.old_lines.len(),
        );
        numbers(
            unified.iter().map(|line| line.new).collect(),
            rows.new_lines.len(),
        );
        let (added, removed) = rows.counts();
        assert!(added <= rows.new_lines.len() && removed <= rows.old_lines.len());
    }
});
