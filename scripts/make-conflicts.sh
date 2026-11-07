#!/usr/bin/env bash
# Rebuilds conflicts/ with one throwaway repository per git operation, each
# stopped on a conflict, to try `git scry` by hand:
#
#   scripts/make-conflicts.sh
#   cd conflicts/rebase && git scry session
#
# The repositories ignore your git configuration (no signing, no hooks).
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)/conflicts"

export GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1
export GIT_AUTHOR_NAME="Alex Martin" GIT_AUTHOR_EMAIL=alex@example.com
export GIT_COMMITTER_NAME="Alex Martin" GIT_COMMITTER_EMAIL=alex@example.com
export GIT_EDITOR=true

# main adds a timeout to upload() and a cancel() function; feature/offline-sync
# documents offline mode, adds retries to upload() and a pause() function at
# the same place as cancel(), then updates the notes.
diverged() {
  local dir="$root/$1"
  mkdir -p "$dir" && cd "$dir"
  git init --quiet --initial-branch=main
  git config merge.conflictStyle zdiff3
  printf 'func upload(_ item: Item) async throws {\n    try await client.send(item)\n}\n\nfunc download(_ id: Item.ID) async throws -> Item {\n    try await client.fetch(id)\n}\n' > SyncEngine.swift
  git add . && git commit --quiet -m "Initial sync engine"

  git switch --quiet -c feature/offline-sync
  printf '# Offline mode\n' > Notes.md
  git add . && git commit --quiet -m "Document offline mode"
  printf 'func upload(_ item: Item) async throws {\n    try await retry(times: 3, backoff: .exponential) {\n        try await client.send(item)\n    }\n}\n\nfunc download(_ id: Item.ID) async throws -> Item {\n    try await client.fetch(id)\n}\n\nfunc pause() {\n    queue.suspend()\n}\n' > SyncEngine.swift
  git commit --quiet -am "Retry failed uploads with backoff"
  printf 'Uploads are retried.\n' >> Notes.md
  git commit --quiet -am "Update notes"

  git switch --quiet main
  printf 'func upload(_ item: Item) async throws {\n    try await client.send(item, timeout: .seconds(30))\n}\n\nfunc download(_ id: Item.ID) async throws -> Item {\n    try await client.fetch(id)\n}\n\nfunc cancel() {\n    queue.cancelAll()\n}\n' > SyncEngine.swift
  git commit --quiet -am "Add a 30 s timeout to uploads (#412)"
}

rm -rf "$root"

diverged rebase
git switch --quiet feature/offline-sync
git rebase main > /dev/null 2>&1 || true

diverged merge
git switch --quiet feature/offline-sync
git merge main > /dev/null 2>&1 || true

diverged cherry-pick
git cherry-pick ":/Retry failed uploads" > /dev/null 2>&1 || true

diverged revert
printf 'func upload(_ item: Item) async throws {\n    try await client.send(item, timeout: .seconds(60))\n}\n\nfunc download(_ id: Item.ID) async throws -> Item {\n    try await client.fetch(id)\n}\n\nfunc cancel() {\n    queue.cancelAll()\n}\n' > SyncEngine.swift
git commit --quiet -am "Raise the upload timeout to 60 s"
git revert --no-edit HEAD~1 > /dev/null 2>&1 || true

echo "Repositories stopped on a conflict, in $root:"
for dir in rebase merge cherry-pick revert; do
  echo "  conflicts/$dir"
done
