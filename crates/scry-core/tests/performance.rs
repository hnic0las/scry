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

/// Opening must grow with the file, not with the square of its conflicts: a
/// catalog after a large translation merge has thousands.
#[test]
#[ignore]
fn report_open_timings_by_conflict_count() {
    for conflicts in [40, 500, 3_000] {
        let text = string_catalog(conflicts);
        let (session, _) = time(&format!("open, {conflicts} conflicts"), || {
            MergeSession::from_text(&text, Some(Language::Json)).unwrap()
        });
        time(&format!("rows, {conflicts} conflicts"), || {
            session.rows(false)
        });
    }
}

/// One conflict of 150 long lines on each side, every line different: the
/// word diff of its rows must not be redone on every `rows` call.
#[test]
#[ignore]
fn report_rows_on_one_large_conflict() {
    let side = |who: &str| -> String {
        (0..150)
            .map(|line| {
                let words: Vec<String> =
                    (0..100).map(|word| format!("{who}{line}w{word}")).collect();
                format!("let v{line} = [{}]\n", words.join(", "))
            })
            .collect()
    };
    let text = format!(
        "start\n<<<<<<< ours\n{}||||||| base\n{}=======\n{}>>>>>>> theirs\nend\n",
        side("o"),
        side("b"),
        side("t")
    );
    let (session, _) = time("open, one large conflict", || {
        MergeSession::from_text(&text, Some(Language::Swift)).unwrap()
    });
    time("rows", || session.rows(false));
    time("rows again", || session.rows(false));
}

/// An Xcode project of about `files` × 3 lines (build files, file
/// references, the Sources phase), where each side added a file in
/// `conflicts` places of the Sources phase.
fn project_file(files: usize, conflicts: usize) -> String {
    let build = |n: usize| format!("A{n:023X}");
    let reference = |n: usize| format!("B{n:023X}");
    let mut text = String::from("// !$*UTF8*$!\n{\n\tobjects = {\n");
    for n in 0..files + 2 * conflicts {
        text.push_str(&format!(
            "\t\t{} /* F{n}.swift in Sources */ = {{isa = PBXBuildFile; fileRef = {} /* F{n}.swift */; }};\n",
            build(n),
            reference(n)
        ));
        text.push_str(&format!(
            "\t\t{} /* F{n}.swift */ = {{isa = PBXFileReference; path = F{n}.swift; sourceTree = \"<group>\"; }};\n",
            reference(n)
        ));
    }
    text.push_str("\t\tC00000000000000000000001 /* Sources */ = {\n\t\t\tisa = PBXSourcesBuildPhase;\n\t\t\tfiles = (\n");
    let every = files / conflicts;
    for n in 0..files {
        text.push_str(&format!(
            "\t\t\t\t{} /* F{n}.swift in Sources */,\n",
            build(n)
        ));
        if n % every == 0 && n / every < conflicts {
            let ours = files + 2 * (n / every);
            text.push_str("<<<<<<< HEAD\n");
            text.push_str(&format!(
                "\t\t\t\t{} /* F{ours}.swift in Sources */,\n",
                build(ours)
            ));
            text.push_str("=======\n");
            text.push_str(&format!(
                "\t\t\t\t{} /* F{}.swift in Sources */,\n",
                build(ours + 1),
                ours + 1
            ));
            text.push_str(">>>>>>> feature\n");
        }
    }
    text.push_str("\t\t\t);\n\t\t};\n\t};\n}\n");
    text
}

/// Reports how long reviewing and keeping both sides of a large Xcode
/// project takes; not a pass/fail check.
#[test]
#[ignore]
fn report_project_file_timings() {
    let text = project_file(13_000, 20);
    println!("{} lines", text.lines().count());
    let (review, _) = time("pbxproj review", || {
        scry_core::pbxproj::review(&text, 7).unwrap()
    });
    assert_eq!(review.resolvable, 20);
    time("pbxproj keep both", || {
        scry_core::pbxproj::keep_both(&text, 7, true).unwrap()
    });
}
