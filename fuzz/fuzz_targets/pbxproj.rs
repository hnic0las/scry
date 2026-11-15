//! The Xcode project review on any text: reviewing it and keeping both
//! sides never panics, and keeping both leaves no conflict the review said
//! it settles.
#![no_main]

use libfuzzer_sys::fuzz_target;
use scry_core::{conflict, pbxproj};

fuzz_target!(|text: &str| {
    let size = conflict::DEFAULT_MARKER_SIZE;
    let Ok(review) = pbxproj::review(text, size) else {
        return;
    };
    for remove_missing in [false, true] {
        let Ok(settled) = pbxproj::keep_both(text, size, remove_missing) else {
            continue;
        };
        // The conflicts left are the ones the review could not settle.
        if let Ok(segments) = conflict::parse_with_marker_size(&settled, size) {
            let left = conflict::conflicts(&segments).count();
            assert_eq!(left, review.conflicts.len() - review.resolvable);
        }
    }
});
