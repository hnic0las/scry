//! Scry checked against `git merge-file` on generated merges: git defines
//! what a merge's conflicts are and what taking one side gives, so Scry must
//! agree with it on every input, not only on the cases written by hand.

use std::fs;
use std::path::Path;
use std::process::Command as Process;

use scry_core::command::Command;
use scry_core::git::GitCli;
use scry_core::resolution::CandidateKind;
use scry_core::session::{self, MergeSession, MergetoolFiles};

/// How many merges each test generates: 150, or `SCRY_ORACLE_CASES` for a
/// deeper run before a release (`SCRY_ORACLE_CASES=5000 cargo test --test
/// merge_file_oracle`). Every case is deterministic: a failure names its
/// seed, which reproduces it.
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

/// Lines from a small alphabet, so that the sides share lines and git finds
/// both clean merges and conflicts.
fn line(random: &mut Random) -> String {
    ["a", "b", "c", "d", "e", "", "x y"][random.below(7)].to_string()
}

/// A base of up to 8 lines and two sides, each edited from it by a few
/// deletions, insertions and changes.
fn versions(seed: u64) -> (Vec<String>, Vec<String>, Vec<String>) {
    let mut random = Random::new(seed);
    let base: Vec<String> = (0..random.below(9)).map(|_| line(&mut random)).collect();
    let side = |random: &mut Random| {
        let mut lines = base.clone();
        for _ in 0..random.below(4) {
            let at = random.below(lines.len() + 1);
            match random.below(3) {
                0 if at < lines.len() => {
                    lines.remove(at);
                }
                1 if at < lines.len() => lines[at] = line(random),
                _ => lines.insert(at, line(random)),
            }
        }
        lines
    };
    let ours = side(&mut random);
    let theirs = side(&mut random);
    (base, ours, theirs)
}

fn text(lines: &[String]) -> String {
    lines.iter().map(|line| format!("{line}\n")).collect()
}

/// A folder holding the three versions, and git with the user's
/// configuration left out (it could set `merge.conflictStyle`).
struct Case {
    dir: tempfile::TempDir,
}

impl Case {
    fn new(base: &str, ours: &str, theirs: &str) -> Case {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("gitconfig"), "").unwrap();
        fs::write(dir.path().join("base"), base).unwrap();
        fs::write(dir.path().join("ours"), ours).unwrap();
        fs::write(dir.path().join("theirs"), theirs).unwrap();
        Case { dir }
    }

    fn path(&self) -> &Path {
        self.dir.path()
    }

    /// `git merge-file` on the three versions with these options: its
    /// output, and its exit code (the number of conflicts).
    fn merge_file(&self, options: &[&str]) -> (String, i32) {
        let output = Process::new("git")
            .current_dir(self.path())
            .env("GIT_CONFIG_GLOBAL", self.path().join("gitconfig"))
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .args([
                "merge-file",
                "-p",
                "-L",
                "ours",
                "-L",
                "base",
                "-L",
                "theirs",
            ])
            .args(options)
            .args(["ours", "base", "theirs"])
            .output()
            .unwrap();
        (
            String::from_utf8(output.stdout).unwrap(),
            output.status.code().unwrap(),
        )
    }

    fn git(&self) -> GitCli {
        GitCli::new(self.path().to_path_buf())
            .with_env(
                "GIT_CONFIG_GLOBAL",
                &self.path().join("gitconfig").display().to_string(),
            )
            .with_env("GIT_CONFIG_NOSYSTEM", "1")
    }
}

/// The merged text with every conflict resolved with `kind`, or what went
/// wrong.
fn resolved_with(mut session: MergeSession, kind: CandidateKind) -> Result<String, String> {
    for conflict in 0..session.conflict_count() {
        session
            .apply(Command::Choose {
                conflict,
                candidate: kind,
            })
            .map_err(|error| format!("conflict {conflict}: {error}"))?;
    }
    session
        .merged_text()
        .ok_or_else(|| String::from("a conflict is left"))
}

/// What taking each side gives, as git computes it, for files written in
/// `style`: the options git takes for ours, theirs, and both.
const RESOLUTIONS: [(CandidateKind, &str); 3] = [
    (CandidateKind::Ours, "--ours"),
    (CandidateKind::Theirs, "--theirs"),
    (CandidateKind::OursThenTheirs, "--union"),
];

/// Read from the markers git writes, in each conflict style, Scry finds as
/// many conflicts as git counts, and taking ours, theirs or both everywhere
/// gives exactly what `git merge-file --ours`, `--theirs` and `--union`
/// write: the text Scry would save is one git itself produces.
#[test]
fn reading_git_markers_agrees_with_merge_file() {
    let mut conflicts = 0;
    for seed in 0..cases() {
        let (base, ours, theirs) = versions(seed);
        let case = Case::new(&text(&base), &text(&ours), &text(&theirs));
        for style in ["--merge", "--diff3", "--zdiff3"] {
            let style_option = if style == "--merge" {
                None
            } else {
                Some(style)
            };
            let options: Vec<&str> = style_option.into_iter().collect();
            let (written, count) = case.merge_file(&options);
            let session = || {
                MergeSession::from_text(&written, None)
                    .unwrap_or_else(|error| panic!("seed {seed} {style}: {error}\n{written}"))
            };
            assert_eq!(
                session().conflict_count(),
                count as usize,
                "seed {seed} {style}: conflict count\n{written}"
            );
            conflicts += count;
            for (kind, option) in RESOLUTIONS {
                let mut with_option = options.clone();
                with_option.push(option);
                let (expected, _) = case.merge_file(&with_option);
                assert_eq!(
                    resolved_with(session(), kind).as_deref(),
                    Ok(expected.as_str()),
                    "seed {seed} {style} {option}\nbase {base:?}\nours {ours:?}\ntheirs {theirs:?}\n{written}"
                );
            }
        }
    }
    // The generator must produce conflicts, or the test checks nothing.
    assert!(conflicts > i32::try_from(cases()).unwrap(), "{conflicts}");
}

/// Through `git mergetool`'s files, with git's default `merge` style: Scry
/// rebuilds the conflicts with their base, and whatever it rebuilt, taking
/// ours or theirs everywhere still gives what git gives. Sides without a
/// final line break are generated too, which git's markers hide.
#[test]
fn loading_mergetool_files_agrees_with_merge_file() {
    let mut failures: Vec<String> = Vec::new();
    for seed in 0..cases() {
        let (base, ours, theirs) = versions(seed);
        let mut random = Random::new(seed ^ 0xA5A5);
        let mut unterminated = |lines: &[String]| {
            let text = text(lines);
            match random.below(4) {
                0 if !text.is_empty() => text[..text.len() - 1].to_string(),
                _ => text,
            }
        };
        let (base, ours, theirs) = (
            unterminated(&base),
            unterminated(&ours),
            unterminated(&theirs),
        );
        let case = Case::new(&base, &ours, &theirs);
        let (written, count) = case.merge_file(&[]);
        if count == 0 {
            continue;
        }
        let merged = case.path().join("merged");
        fs::write(&merged, &written).unwrap();
        let files = MergetoolFiles {
            base: case.path().join("base"),
            local: case.path().join("ours"),
            remote: case.path().join("theirs"),
            merged,
        };
        let session = || {
            session::load(&case.git(), &files, "ours", "theirs")
                .unwrap_or_else(|error| panic!("seed {seed}: {error}\n{written}"))
        };
        for (kind, option) in &RESOLUTIONS[..2] {
            let (expected, _) = case.merge_file(&[option]);
            let resolved = resolved_with(session(), *kind);
            if resolved.as_deref() != Ok(expected.as_str()) {
                failures.push(format!(
                    "seed {seed} {option}: {resolved:?} instead of {expected:?}\n  base {base:?} ours {ours:?} theirs {theirs:?}"
                ));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} cases differ:\n{}",
        failures.len(),
        failures.join("\n")
    );
}
