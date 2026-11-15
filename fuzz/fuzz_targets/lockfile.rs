//! Lockfile reading on any three texts (split at zero bytes), in every
//! format: listing packages and their changes never panics, and the line
//! merge (`go.sum`) keeps what both sides have.
#![no_main]

use libfuzzer_sys::fuzz_target;
use scry_core::lockfile::{self, Format};

const NAMES: [&str; 18] = [
    "Cargo.lock",
    "package-lock.json",
    "yarn.lock",
    "pnpm-lock.yaml",
    "Podfile.lock",
    "Gemfile.lock",
    "poetry.lock",
    "composer.lock",
    "go.sum",
    "uv.lock",
    "Pipfile.lock",
    "pubspec.lock",
    "Cartfile.resolved",
    "gradle.lockfile",
    "flake.lock",
    "bun.lock",
    "deno.lock",
    "mix.lock",
];

fuzz_target!(|text: &str| {
    let mut parts = text.splitn(3, '\0');
    let (base, ours, theirs) = (
        parts.next().unwrap_or(""),
        parts.next().unwrap_or(""),
        parts.next().unwrap_or(""),
    );
    for name in NAMES {
        let format = Format::of(name).expect("a lockfile name");
        for change in lockfile::changes(format, base, ours, theirs) {
            assert!(change.ours != change.base || change.theirs != change.base);
        }
    }
    // Each line is written back followed by "\n": split on "\n" alone, so
    // that a line ending in a lone "\r" reads back as written.
    let merged = lockfile::merge_lines(base, ours, theirs);
    for line in ours.lines().filter(|line| !line.trim().is_empty()) {
        if theirs.lines().any(|other| other == line) {
            assert!(merged.split('\n').any(|kept| kept == line), "{line:?} lost");
        }
    }
});
