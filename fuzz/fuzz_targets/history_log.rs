//! What `git log --follow --raw --numstat -z` prints for one file, read
//! as the points of its history, on any text: reading never panics, and
//! each point names its file and its commit as git gave them.
#![no_main]

use libfuzzer_sys::fuzz_target;
use scry_core::history;

fuzz_target!(|text: &str| {
    let head = text.split('\u{1f}').next().unwrap_or("");
    for point in history::parse_log(text, head) {
        assert!(!point.id.contains(['\u{1}', '\u{1f}', '\u{2}']));
        assert!(point.short_id().len() <= 7);
        assert!(text.contains(point.id.as_str()));
        assert!(text.contains(point.path.as_str()));
        if let Some(parent) = &point.parent {
            assert!(!parent.is_empty() && !parent.contains(' '));
        }
    }
});
