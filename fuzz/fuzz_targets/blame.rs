//! What `git blame --incremental` prints, read line by line, on any text:
//! reading never panics, every entry names a commit already listed, and a
//! commit is listed once.
#![no_main]

use std::collections::HashSet;

use libfuzzer_sys::fuzz_target;
use scry_core::file_blame::Parser;

fuzz_target!(|text: &str| {
    let mut parser = Parser::new();
    for line in text.lines() {
        if let Some(lines) = parser.feed(line) {
            if let Some(commit) = lines.commit {
                let commit = &parser.commits()[commit];
                assert!(commit.id.chars().any(|c| c != '0'));
                assert!(commit.short_id().len() <= 7);
            }
        }
    }
    let ids: HashSet<&str> = parser.commits().iter().map(|c| c.id.as_str()).collect();
    assert_eq!(ids.len(), parser.commits().len());
});
