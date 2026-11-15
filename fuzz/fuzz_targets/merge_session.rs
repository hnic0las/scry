//! A merge session opened from any text with conflict markers: opening it,
//! then taking ours, theirs or both in every conflict, never panics, and
//! settles every conflict.
#![no_main]

use libfuzzer_sys::fuzz_target;
use scry_core::command::Command;
use scry_core::resolution::CandidateKind;
use scry_core::session::MergeSession;

fuzz_target!(|text: &str| {
    for kind in [
        CandidateKind::Ours,
        CandidateKind::Theirs,
        CandidateKind::OursThenTheirs,
    ] {
        let Ok(mut session) = MergeSession::from_text(text, None) else {
            return;
        };
        for conflict in 0..session.conflict_count() {
            // A conflict may not offer this candidate: that is an error,
            // not a panic.
            if session
                .apply(Command::Choose {
                    conflict,
                    candidate: kind,
                })
                .is_err()
            {
                return;
            }
        }
        assert!(session.merged_text().is_some(), "a conflict is left");
    }
});
