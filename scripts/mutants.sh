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

max_mb=${SCRY_MUTANTS_MAX_MB:-2048}
jobs=${SCRY_MUTANTS_JOBS:-4}

# cargo-mutants builds and tests each mutant in a copy of the tree named
# cargo-mutants-<crate>-<random>.tmp in the temporary folder: the test
# programs run from its target folder. The compiler, whose arguments name
# that folder too, is left alone.
#
# The size checked is the process's physical footprint, compressed memory
# included. ps's RSS leaves compressed pages out: on 2026-11-27 three
# mutants that grew a vector without end stayed under the limit by RSS
# while filling memory, and the Mac stopped (a watchdog panic). Checked
# five times a second, as such a mutant grows by gigabytes a second.
watchdog=$(cat <<'PY'
import ctypes, subprocess, sys, time

limit = int(sys.argv[1]) * 1024 * 1024
libproc = ctypes.CDLL("/usr/lib/libproc.dylib")

class Usage(ctypes.Structure):
    _fields_ = [("uuid", ctypes.c_uint8 * 16)] + [
        (name, ctypes.c_uint64)
        for name in ("user", "system", "idle", "interrupt", "pageins", "wired",
                     "resident", "footprint", "start", "exit")
    ]

def footprint(pid):
    usage = Usage()
    if libproc.proc_pid_rusage(pid, 0, ctypes.byref(usage)) != 0:
        return 0
    return usage.footprint

while True:
    listing = subprocess.run(["ps", "-Ao", "pid=,command="], capture_output=True, text=True).stdout
    for line in listing.splitlines():
        pid, _, command = line.strip().partition(" ")
        program = command.split(" ", 1)[0]
        if "cargo-mutants-" in program and ".tmp/target/" in program:
            size = footprint(int(pid))
            if size > limit:
                print(f"mutants.sh: killed {pid} at {size >> 20} MB: {program}", file=sys.stderr, flush=True)
                subprocess.run(["kill", "-9", pid])
    time.sleep(0.2)
PY
)

# Python itself in the background, not a function around it: `$!` is then
# the watchdog, which the trap stops. A function's subshell was killed
# instead and left the watchdog running, holding the caller's output open.
python3 -I -c "$watchdog" "$max_mb" &
watcher=$!
trap 'kill "$watcher" 2>/dev/null || true' EXIT

cargo mutants --jobs "$jobs" "$@" < /dev/null
