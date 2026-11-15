//! String Catalog merges on any three texts (split at zero bytes): merging
//! never panics, and resolving with either side writes valid JSON.
#![no_main]

use libfuzzer_sys::fuzz_target;
use scry_core::catalog::{self, Pick};

fuzz_target!(|text: &str| {
    let mut parts = text.splitn(3, '\0');
    let (base, ours, theirs) = (
        parts.next().unwrap_or(""),
        parts.next().unwrap_or(""),
        parts.next().unwrap_or(""),
    );
    let Ok(merge) = catalog::merge(base, ours, theirs) else {
        return;
    };
    for pick in [Pick::Ours, Pick::Theirs] {
        let picks = vec![pick; merge.conflicts.len()];
        let written = merge.resolve(&picks).expect("one pick per conflict");
        serde_json::from_str::<serde_json::Value>(&written).expect("valid JSON");
    }
});
