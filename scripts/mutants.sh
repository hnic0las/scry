#!/usr/bin/env bash
# Runs cargo-mutants safely: its arguments are passed on as they are.
#
#   scripts/mutants.sh                                  the whole workspace
#   scripts/mutants.sh --in-diff /tmp/scry.diff         the changed lines
#   scripts/mutants.sh -f crates/scry-core/src/words.rs one file
#
# A mutant can turn a loop into an endless one that keeps filling memory
# (a list growing, a question asked again and again into a buffer). Several
# at once filled memory and swap until the Mac crashed on 2026-11-15. A
# watchdog here kills any test process over $SCRY_MUTANTS_MAX_MB (2,048 MB
# by default): its tests fail, so the mutant counts as caught.
#
# Standard input is closed: with a terminal attached, the CLI's question
# mutants wait for an answer and count as caught, hiding untested prompts.
set -euo pipefail

cd "$(dirname "$0")/.."
export PATH="/opt/homebrew/opt/rustup/bin:$PATH"

max_kb=$(( ${SCRY_MUTANTS_MAX_MB:-2048} * 1024 ))
jobs=${SCRY_MUTANTS_JOBS:-4}

# cargo-mutants builds and tests each mutant in a copy of the tree named
# cargo-mutants-<crate>-<random>.tmp in the temporary folder: the test
# programs run from its target folder. The compiler, whose arguments name
# that folder too, is left alone.
watch() {
    while sleep 1; do
        ps -Ao pid=,rss=,command= | while read -r pid rss command; do
            program=${command%% *}
            case "$program" in
                *cargo-mutants-*.tmp/target/*)
                    if [ "$rss" -gt "$max_kb" ]; then
                        echo "mutants.sh: killed $pid at $(( rss / 1024 )) MB: $program" >&2
                        kill -9 "$pid" 2>/dev/null || true
                    fi
                    ;;
            esac
        done
    done
}

watch &
watcher=$!
trap 'kill "$watcher" 2>/dev/null || true' EXIT

cargo mutants --jobs "$jobs" "$@" < /dev/null
