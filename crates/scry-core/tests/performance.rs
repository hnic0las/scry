//! Timings on a large conflicted file, such as an Xcode string catalog
//! (`.xcstrings`, JSON) after two branches added translations.
//!
//! The timing report is ignored by default; run it with
//! `cargo test -p scry-core --release --test performance -- --ignored --nocapture`.

use std::time::{Duration, Instant};

use scry_core::command::Command;
use scry_core::resolution::CandidateKind;
use scry_core::session::MergeSession;
use scry_core::syntax::Language;

/// A JSON string catalog of about 1.5 MB with `conflicts` conflicts spread
/// through it, each on one translation.
fn string_catalog(conflicts: usize) -> String {
    let entries = 6_000;
    let every = entries / conflicts;
    let mut text = String::from("{\n  \"sourceLanguage\" : \"en\",\n  \"strings\" : {\n");
    for entry in 0..entries {
        let comma = if entry + 1 < entries { "," } else { "" };
        let value = |who: &str| {
            format!(
                "              \"value\" : \"Traduction {who} de la clé numéro {entry}, assez longue pour peser\"\n"
            )
        };
        text.push_str(&format!(
            "    \"key.{entry}\" : {{\n      \"localizations\" : {{\n        \"fr\" : {{\n          \"stringUnit\" : {{\n            \"state\" : \"translated\",\n"
        ));
        if entry % every == every / 2 && entry / every < conflicts {
            text.push_str("<<<<<<< HEAD\n");
            text.push_str(&value("ours"));
            text.push_str("||||||| base\n");
            text.push_str(&value("base"));
            text.push_str("=======\n");
            text.push_str(&value("theirs"));
            text.push_str(">>>>>>> feature\n");
        } else {
            text.push_str(&value("commune"));
        }
        text.push_str(&format!(
            "          }}\n        }}\n      }}\n    }}{comma}\n"
        ));
    }
    text.push_str("  }\n}\n");
    text
}

fn time<T>(label: &str, action: impl FnOnce() -> T) -> (T, Duration) {
    let start = Instant::now();
    let result = action();
    let elapsed = start.elapsed();
    println!("{label:<32} {:>9.1} ms", elapsed.as_secs_f64() * 1000.0);
    (result, elapsed)
}

/// Reports how long each step the app takes on a large file lasts, to
/// compare before and after a change; not a pass/fail check.
#[test]
#[ignore]
fn report_timings_on_a_large_string_catalog() {
    let text = string_catalog(40);
    println!("file: {} KB, 40 conflicts", text.len() / 1024);

    let (session, _) = time("open (from_text)", || {
        MergeSession::from_text(&text, Some(Language::Json)).unwrap()
    });
    let mut session = session;
    time("rows", || session.rows(false));
    time("merged_check (unresolved)", || session.merged_check());
    time("choose", || {
        session
            .apply(Command::Choose {
                conflict: 0,
                candidate: CandidateKind::Theirs,
            })
            .unwrap()
    });
    time("rows after a choice", || session.rows(false));
    for index in 1..40 {
        session
            .apply(Command::Choose {
                conflict: index,
                candidate: CandidateKind::Ours,
            })
            .unwrap();
    }
    time("merged_check (resolved)", || session.merged_check());
    let draft = "              \"value\" : \"Une traduction écrite à la main\"\n";
    time("highlight_draft (keystroke)", || {
        session.highlight_draft(5, draft)
    });
    time("newline (Return)", || {
        session.newline(5, "              \"value\" : {", "")
    });
}
