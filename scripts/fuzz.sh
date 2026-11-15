#!/usr/bin/env bash
# Fuzzes Scry's hand-written parsers with cargo-fuzz (libFuzzer), one
# target after another, before each release.
#
#   scripts/fuzz.sh                    every target, 30 minutes each
#   scripts/fuzz.sh 60                 every target, 60 seconds each
#   scripts/fuzz.sh 600 pbxproj        one target, 10 minutes
#
# Needs the nightly toolchain and cargo-fuzz (`rustup toolchain install
# nightly`, `cargo install cargo-fuzz`). Each target's corpus grows in
# fuzz/corpus/<target> (ignored by git) from the seeds in fuzz/seeds/<target>,
# so a run picks up where the last one stopped. A crash stops its target
# and is saved in fuzz/artifacts/<target>: turn it into a regression test in
# scry-core before fixing it (`cargo +nightly fuzz fmt <target> <file>`
# prints it).
set -uo pipefail

cd "$(dirname "$0")/../fuzz"
export PATH="/opt/homebrew/opt/rustup/bin:$HOME/.cargo/bin:$PATH"

seconds=${1:-1800}
shift || true
targets=("$@")
if [ ${#targets[@]} -eq 0 ]; then
    targets=(conflict_markers merge_session pbxproj catalog resolved lockfile)
fi

cargo +nightly fuzz build || exit 1

failed=()
for target in "${targets[@]}"; do
    mkdir -p "corpus/$target"
    echo "== $target, ${seconds}s"
    # -rss_limit_mb: a parser growing memory without bound is a defect too.
    if ! cargo +nightly fuzz run "$target" "corpus/$target" "seeds/$target" -- \
        -max_total_time="$seconds" -rss_limit_mb=2048 -timeout=10 \
        -print_final_stats=1 2>&1 | tail -20; then
        failed+=("$target")
    fi
done

if [ ${#failed[@]} -gt 0 ]; then
    echo "Crashes in: ${failed[*]} (see fuzz/artifacts/)"
    exit 1
fi
echo "No crash in: ${targets[*]}"
