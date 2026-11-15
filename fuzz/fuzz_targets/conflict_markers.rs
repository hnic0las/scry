//! The conflict-marker parser on any text and any marker size: it never
//! panics, and what it accepts is consistent with the text it read.
#![no_main]

use libfuzzer_sys::fuzz_target;
use scry_core::conflict::{self, Segment};

fuzz_target!(|data: &[u8]| {
    let Some((&size, text)) = data.split_first() else {
        return;
    };
    let Ok(text) = std::str::from_utf8(text) else {
        return;
    };
    let marker_size = usize::from(size % 16) + 1;
    let Ok(segments) = conflict::parse_with_marker_size(text, marker_size) else {
        return;
    };
    // Every byte of content comes from the text: the parser only drops
    // marker lines, never invents text.
    let mut kept = 0;
    let mut previous_common = false;
    for segment in &segments {
        match segment {
            Segment::Common(common) => {
                assert!(!common.is_empty(), "an empty common segment");
                assert!(!previous_common, "two common segments in a row");
                assert!(text.contains(common.as_str()));
                kept += common.len();
                previous_common = true;
            }
            Segment::Conflict(found) => {
                for side in [Some(&found.ours), found.base.as_ref(), Some(&found.theirs)]
                    .into_iter()
                    .flatten()
                {
                    assert!(text.contains(side.text.as_str()));
                    kept += side.text.len();
                }
                assert!(found.start_line >= 1);
                previous_common = false;
            }
        }
    }
    assert!(kept <= text.len());
    // Without an opening marker there is nothing to parse.
    if conflict::conflicts(&segments).next().is_none() {
        assert!(segments.len() <= 1);
        assert_eq!(kept, text.len());
    }
});
