#!/bin/bash
# Builds two repositories stopped on the conflict shapes of review batch 2:
#   conflicts/edge/merge  a merge with a `-merge` JSON file, a file renamed on
#                         both sides, and an ordinary text conflict;
#   conflicts/edge/slow   a rebase stopped on a conflict, with `exec sleep 6`
#                         next, so Continue takes a while.
set -e
D="${1:-$(cd "$(dirname "$0")/.." && pwd)/conflicts/edge}"; rm -rf "$D"; mkdir -p "$D"
g(){ git -c user.name=t -c user.email=t@t -c commit.gpgsign=false "$@"; }
# 1. -merge + rename on both sides, merged.
mkdir "$D/merge" && cd "$D/merge" && git init -q -b main
echo '*.json -merge' > .gitattributes; echo '{"v":0}' > flags.json; echo text > old.txt; echo 'a' > notes.md
g add -A && g commit -qm base
g switch -qc feature; echo '{"v":"feature"}' > flags.json; g mv old.txt theirs.txt; echo 'b' > notes.md; g commit -qam feature
g switch -q main; echo '{"v":"main"}' > flags.json; g mv old.txt ours.txt; echo 'c' > notes.md; g commit -qam main
g merge feature >/dev/null 2>&1 || true
# 2. A rebase stopped on a conflict, with a slow step after it.
mkdir "$D/slow" && cd "$D/slow" && git init -q -b main
echo a > a.txt; g add -A && g commit -qm base
g switch -qc topic; echo topic > a.txt; g commit -qam topic; echo more > b.txt; g add b.txt; g commit -qm more
g switch -q main; echo main > a.txt; g commit -qam main; g switch -q topic
GIT_SEQUENCE_EDITOR="sed -i '' '1a\\
exec sleep 6
'" g rebase -i main >/dev/null 2>&1 || true
echo "Ready: $D/merge (merge) and $D/slow (rebase)"
