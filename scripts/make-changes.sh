#!/usr/bin/env bash
# Rebuilds changes/ with throwaway repositories that have something to
# read, to try `git scry diff`, `git scry` alone, `git difftool` and the
# Open Repository window by hand:
#
#   scripts/make-changes.sh
#   cd changes/atlas && git scry diff
#
#   changes/atlas             a multi-platform app on feature/offline-queue,
#                             with a history by several authors and a
#                             working copy holding every kind of change
#                             (the list is printed at the end)
#   changes/many              400 changed files in nested folders, and one
#                             file with 250 changes
#   changes/merging           a merge stopped on a conflict, with files it
#                             merged by itself and one edited since
#   changes/unborn            a repository with no commit yet
#   changes/clean             nothing to show
#
# For a folder that is not a repository, take one outside this checkout:
# changes/ is inside it, so a plain folder here belongs to Scry's own.
#
# git difftool opens the Scry app when Scry is your diff tool (Scry ›
# Settings), and otherwise runs this checkout's git-scry, which opens the
# installed app. Like make-conflicts.sh, the repositories are built
# without the rest of your git configuration (no signing, no hooks).
set -euo pipefail

workspace="$(cd "$(dirname "$0")/.." && pwd)"
root="$workspace/changes"
# Read before GIT_CONFIG_GLOBAL below hides it.
global_tool="$(git config --global --get diff.tool 2>/dev/null || true)"

export GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1
export GIT_EDITOR=true

# put <path>: writes stdin to the file, creating its folders.
put() {
  mkdir -p "$(dirname "$1")"
  cat > "$1"
}

# icon <path> <colour byte> [padding]: a tiny binary file, different per
# colour, and longer with padding.
icon() {
  mkdir -p "$(dirname "$1")"
  printf '\x89PNG\r\n\x1a\n\x00\x00\x00\x0dIHDR\x00\x00\x00\x10\x00\x00\x00\x10\x08\x02\x00\x00\x00%b\x00\x00\x00' "$2" > "$1"
  if [ "${3:-0}" -gt 0 ]; then head -c "$3" /dev/zero >> "$1"; fi
}

# change <path> <perl program>: edits the file in place, read whole.
change() {
  perl -0pi -e "$2" "$1"
}

# as <name> <date>: who makes the next commits, and when.
as() {
  local email
  email="$(printf '%s' "$1" | tr 'A-Z ' 'a-z.')@example.com"
  export GIT_AUTHOR_NAME="$1" GIT_AUTHOR_EMAIL="$email"
  export GIT_COMMITTER_NAME="$1" GIT_COMMITTER_EMAIL="$email"
  export GIT_AUTHOR_DATE="$2" GIT_COMMITTER_DATE="$2"
}

commit() {
  git add -A && git commit --quiet -m "$1"
}

# repository <dir>: a new repository on main, where `git difftool` runs
# Scry.
repository() {
  rm -rf "$1"
  mkdir -p "$1" && cd "$1"
  git init --quiet --initial-branch=main
  # A local diff tool would win over the one the app's settings set:
  # without Scry as your diff tool, run this checkout's code, rebuilt if
  # needed.
  if [ "$global_tool" != scry ]; then
    git config diff.tool scry
    git config difftool.scry.cmd "cargo run -q --manifest-path '$workspace/Cargo.toml' -p git-scry -- difftool -- \"\$LOCAL\" \"\$REMOTE\" \"\$MERGED\""
    git config difftool.prompt false
  fi
}

rm -rf "$root"
mkdir -p "$root"

# -------------------------------------------------------------------- atlas

# The colours, a repository of its own that Atlas holds as a submodule.
palette="$root/.palette"
repository "$palette"
as "Alex Martin" "2026-09-01T09:00:00+0200"
put palette.json <<'EOF'
{
  "accent": "#2f6fed",
  "warning": "#d98c00"
}
EOF
commit "Start the palette"

repository "$root/atlas"

# ----------------------------------------------------------------- history

as "Alex Martin" "2026-09-01T10:00:00+0200"

put .gitignore <<'EOF'
build/
EOF

put README.md <<'EOF'
# Atlas

Weather for iOS, Android and the web, synced through one Rust core.

## Build

    make core
    make web
EOF

put ios/Sources/SyncEngine.swift <<'EOF'
import Foundation

/// Sends local changes to the server and fetches remote ones.
final class SyncEngine {
    private let client: APIClient
    private let queue = OperationQueue()

    init(client: APIClient) {
        self.client = client
        queue.maxConcurrentOperationCount = 2
    }

    func upload(_ item: Item) async throws {
        try await client.send(item)
    }

    func download(_ id: Item.ID) async throws -> Item {
        try await client.fetch(id)
    }

    func cancel() {
        queue.cancelAllOperations()
    }
}
EOF

put ios/Sources/Views/ProfileView.swift <<'EOF'
import SwiftUI

struct ProfileView: View {
    let user: User

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(user.name)
                .font(.title2)
            Text(user.email)
                .foregroundStyle(.secondary)
        }
        .padding()
    }
}
EOF

put ios/Sources/Legacy/OldCache.m <<'EOF'
#import "OldCache.h"

@implementation OldCache

- (instancetype)init {
    self = [super init];
    if (self) {
        _entries = [NSMutableDictionary dictionary];
    }
    return self;
}

- (void)store:(id)value forKey:(NSString *)key {
    self.entries[key] = value;
}

@end
EOF

put android/app/src/main/java/com/atlas/sync/SyncWorker.kt <<'EOF'
package com.atlas.sync

import android.content.Context
import androidx.work.CoroutineWorker
import androidx.work.WorkerParameters

class SyncWorker(context: Context, params: WorkerParameters) : CoroutineWorker(context, params) {
    override suspend fun doWork(): Result {
        val pending = repository.pending()
        if (pending.isEmpty()) return Result.success()
        pending.forEach { api.upload(it) }
        return Result.success()
    }
}
EOF

put core/Cargo.toml <<'EOF'
[package]
name = "atlas-core"
version = "0.4.0"
edition = "2021"

[dependencies]
serde = { version = "1", features = ["derive"] }
tokio = { version = "1", features = ["rt", "time"] }
EOF

put core/src/queue.rs <<'EOF'
//! The queue of changes waiting to be sent.

use std::collections::VecDeque;
use std::time::Duration;

/// How many times an upload is tried before it is given up.
const MAX_ATTEMPTS: u32 = 3;

/// One change waiting to be sent.
#[derive(Debug, Clone, PartialEq)]
pub struct Pending {
    pub id: u64,
    pub payload: Vec<u8>,
    pub attempts: u32,
}

/// What became of an upload.
#[derive(Debug, PartialEq)]
pub enum Outcome {
    Sent,
    Retry { after: Duration },
    GivenUp,
}

#[derive(Default)]
pub struct Queue {
    items: VecDeque<Pending>,
    next_id: u64,
}

impl Queue {
    pub fn new() -> Queue {
        Queue::default()
    }

    /// Adds a change at the end of the queue and gives its id.
    pub fn push(&mut self, payload: Vec<u8>) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        self.items.push_back(Pending {
            id,
            payload,
            attempts: 0,
        });
        id
    }

    /// The next change to send, left in the queue until it is sent.
    pub fn peek(&self) -> Option<&Pending> {
        self.items.front()
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Records the result of sending the first change.
    pub fn record(&mut self, sent: bool) -> Outcome {
        let Some(mut item) = self.items.pop_front() else {
            return Outcome::Sent;
        };
        if sent {
            return Outcome::Sent;
        }
        item.attempts += 1;
        if item.attempts >= MAX_ATTEMPTS {
            return Outcome::GivenUp;
        }
        let after = Duration::from_secs(2);
        self.items.push_front(item);
        Outcome::Retry { after }
    }

    /// Removes every change, sent or not.
    pub fn clear(&mut self) {
        self.items.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pushed_change_is_the_next_to_send() {
        let mut queue = Queue::new();
        let id = queue.push(vec![1, 2, 3]);
        assert_eq!(queue.peek().map(|item| item.id), Some(id));
    }

    #[test]
    fn a_failed_upload_is_retried() {
        let mut queue = Queue::new();
        queue.push(vec![1]);
        assert_eq!(
            queue.record(false),
            Outcome::Retry {
                after: Duration::from_secs(2)
            }
        );
        assert_eq!(queue.len(), 1);
    }
}
EOF

put web/src/api.ts <<'EOF'
export interface User {
  id: string;
  name: string;
  email: string;
}

const BASE_URL = "https://api.atlas.example/v1";

export async function fetchUser(id: string): Promise<User> {
  const response = await fetch(`${BASE_URL}/users/${id}`);
  if (!response.ok) {
    throw new Error(`Request failed: ${response.status}`);
  }
  return response.json();
}

export async function syncStatus(): Promise<number> {
  const response = await fetch(`${BASE_URL}/sync/status`);
  const body = await response.json();
  return body.pending;
}

export async function upload(payload: Uint8Array): Promise<void> {
  await fetch(`${BASE_URL}/sync/upload`, { method: "POST", body: payload });
}
EOF

put web/src/components/Banner.tsx <<'EOF'
import React from "react";

interface Props {
  pending: number;
}

export function Banner({ pending }: Props) {
  if (pending === 0) {
    return null;
  }
  return (
    <div className="banner">
      <span>{pending} changes waiting to sync</span>
    </div>
  );
}
EOF

put web/src/styles/app.css <<'EOF'
:root {
  --accent: #2f6fed;
  --radius: 6px;
}

.banner {
  background: var(--accent);
  color: white;
  padding: 8px 12px;
  border-radius: var(--radius);
}

.profile {
  display: flex;
  gap: 12px;
}
EOF

# One line of 6,000 characters, as a bundler writes it.
mkdir -p web/dist
{
  printf '!function(){"use strict";var atlas={version:"0.4.0"};'
  for i in $(seq 1 140); do
    printf 'atlas.m%d=function(a,b){return a+b*%d};' "$i" "$i"
  done
  printf 'window.atlas=atlas}();\n'
} > web/dist/bundle.min.js

put backend/app/users.py <<'EOF'
from dataclasses import dataclass


@dataclass
class User:
    id: str
    name: str
    email: str


def display_name(user: User) -> str:
    """The name shown in the profile and in sync messages."""
    return user.name.strip()


def find_user(users: list[User], user_id: str) -> User | None:
    for user in users:
        if user.id == user_id:
            return user
    return None


def initials(user: User) -> str:
    parts = user.name.split()
    return "".join(part[0].upper() for part in parts[:2])
EOF

put backend/server/main.go <<'EOF'
package main

import (
	"fmt"
	"log"
	"net/http"
)

func syncStatus(w http.ResponseWriter, r *http.Request) {
	pending := queue.Len()
	if pending > 0 {
		fmt.Fprintf(w, "%d pending", pending)
		return
	}
	fmt.Fprint(w, "up to date")
}

func main() {
	http.HandleFunc("/sync/status", syncStatus)
	log.Println("listening on :8080")
	log.Fatal(http.ListenAndServe(":8080", nil))
}
EOF

# Recipes start with a tab.
printf 'core:\n\tcargo build --manifest-path core/Cargo.toml\n\nweb:\n\tcd web && npm run build\n\nclean:\n\trm -rf build\n\tcargo clean --manifest-path core/Cargo.toml\n' > Makefile

put scripts/release.sh <<'EOF'
#!/usr/bin/env bash
set -euo pipefail

version="$1"
git tag "v$version"
echo "Tagged v$version"
EOF

put config/flags.json <<'EOF'
{
  "offlineQueue": false,
  "fullNames": true,
  "syncInterval": 300
}
EOF

put config/ci.yml <<'EOF'
name: CI
on: [push]
jobs:
  test:
    runs-on: macos-15
    steps:
      - uses: actions/checkout@v4
      - run: make core
EOF

put docs/notes.md <<'EOF'
# Sync notes

Changes are sent in the order they were made.
A change that fails three times is given up.
EOF

put docs/guide.md <<'EOF'
# Guide

Atlas shows the weather for the cities you follow: Zürich, São Paulo, 東京.
Pull to refresh ↓, or wait for the next sync.

Temperatures are in °C; switch to °F in Settings › Units.
EOF

put docs/latest.md <<'EOF'
See the guide.
EOF

# Windows line endings.
printf 'Atlas for Windows\r\n\r\nRun setup.exe, then sign in.\r\nSettings are kept in %%APPDATA%%\\Atlas.\r\n' > docs/windows.txt

# 25,000 lines: with the edited copy, over the 40,000 lines above which
# Scry pairs no lines and marks no words.
mkdir -p data
awk 'BEGIN { for (i = 1; i <= 25000; i++) printf "%d,City %d,%.2f,%.2f\n", i, i, (i * 37 % 18000) / 100 - 90, (i * 91 % 36000) / 100 - 180 }' > data/cities.csv

icon assets/icon.png '\x11'
icon assets/logo.png '\x22'

git -c protocol.file.allow=always submodule --quiet add "$palette" vendor/palette 2> /dev/null
commit "Start Atlas"

as "Sam Lee" "2026-09-18T14:30:00+0200"
change ios/Sources/SyncEngine.swift 's/try await client.send\(item\)/try await client.send(item, timeout: .seconds(30))/'
change web/src/api.ts 's/const response = await fetch\(`\$\{BASE_URL\}\/users\/\$\{id\}`\);/const response = await fetch(`\${BASE_URL}\/users\/\${id}`, { signal: AbortSignal.timeout(30_000) });/'
commit "Add a 30 s timeout to requests (#412)"

as "Priya Nair" "2026-10-02T11:15:00+0200"
change backend/app/users.py 's/return user.name.strip\(\)/return " ".join(user.name.split())/'
change ios/Sources/Views/ProfileView.swift 's/spacing: 8/spacing: 12/'
commit "Collapse spaces in display names"

as "Alex Martin" "2026-10-20T16:45:00+0200"
git mv ios/Sources/Views/ProfileView.swift ios/Sources/Views/AccountView.swift
change ios/Sources/Views/AccountView.swift 's/struct ProfileView/struct AccountView/'
change core/Cargo.toml 's/version = "0.4.0"/version = "0.4.1"/'
commit "Rename the profile screen to Account"

# The branch: three commits.
git switch --quiet -c feature/offline-queue

as "Nicolas Durand" "2026-11-03T09:20:00+0100"
change core/src/queue.rs 's/const MAX_ATTEMPTS: u32 = 3;/const MAX_ATTEMPTS: u32 = 5;/'
change docs/notes.md 's/fails three times/fails five times/'
commit "Try an upload five times"

as "Nicolas Durand" "2026-11-05T17:05:00+0100"
change android/app/src/main/java/com/atlas/sync/SyncWorker.kt 's/        pending.forEach \{ api.upload\(it\) \}\n        return Result.success\(\)/        return try {\n            pending.forEach { api.upload(it) }\n            Result.success()\n        } catch (error: IOException) {\n            Result.retry()\n        }/'
change android/app/src/main/java/com/atlas/sync/SyncWorker.kt 's/import androidx.work.WorkerParameters\n/import androidx.work.WorkerParameters\nimport java.io.IOException\n/'
commit "Retry the Android worker when the network fails"

as "Sam Lee" "2026-11-12T10:00:00+0100"
change config/flags.json 's/"offlineQueue": false/"offlineQueue": true/'
change README.md 's/synced through one Rust core\./synced through one Rust core, online or not./'
commit "Turn the offline queue on"

# main moved since: `main..HEAD` shows this change undone, `main...HEAD`
# does not.
git switch --quiet main
as "Priya Nair" "2026-11-14T15:40:00+0100"
change web/src/styles/app.css 's/--radius: 6px;/--radius: 8px;/'
change README.md 's/    make web\n/    make web\n    make test\n/'
commit "Round the corners a little more"
git switch --quiet feature/offline-queue

# ------------------------------------------------------------ working copy

# Staged.

# Six changes in one file, some of one word, some of whole blocks.
change core/src/queue.rs 's/use std::time::Duration;/use std::time::{Duration, Instant};/'
change core/src/queue.rs 's/    pub attempts: u32,\n\}/    pub attempts: u32,\n    \/\/\/ When the change was made.\n    pub queued_at: Instant,\n}/'
change core/src/queue.rs 's/    Retry \{ after: Duration \},/    Retry { after: Duration, attempt: u32 },/'
change core/src/queue.rs 's/            attempts: 0,\n/            attempts: 0,\n            queued_at: Instant::now(),\n/'
change core/src/queue.rs 's/        let after = Duration::from_secs\(2\);\n        self.items.push_front\(item\);\n        Outcome::Retry \{ after \}/        \/\/ Wait twice as long after each failure.\n        let after = Duration::from_secs(2u64.pow(item.attempts));\n        let attempt = item.attempts;\n        self.items.push_front(item);\n        Outcome::Retry { after, attempt }/'
change core/src/queue.rs 's/            Outcome::Retry \{\n                after: Duration::from_secs\(2\)\n            \}/            Outcome::Retry {\n                after: Duration::from_secs(2),\n                attempt: 1\n            }/'
git add core/src/queue.rs

put android/app/src/main/java/com/atlas/sync/RetryPolicy.kt <<'EOF'
package com.atlas.sync

import kotlin.math.pow

/** How long to wait before an upload is tried again. */
class RetryPolicy(private val maxAttempts: Int = 5) {
    fun shouldRetry(attempt: Int): Boolean = attempt < maxAttempts

    fun delaySeconds(attempt: Int): Long = 2.0.pow(attempt).toLong()
}
EOF
git add android/app/src/main/java/com/atlas/sync/RetryPolicy.kt

git rm --quiet ios/Sources/Legacy/OldCache.m

# Renamed, the same to the last byte.
git mv docs/notes.md docs/sync-notes.md

# Renamed and edited.
git mv web/src/components/Banner.tsx web/src/components/StatusBanner.tsx
change web/src/components/StatusBanner.tsx 's/export function Banner/export function StatusBanner/'
change web/src/components/StatusBanner.tsx 's/      <span>\{pending\} changes waiting to sync<\/span>/      <span>\n        {pending === 1 ? "1 change" : `\${pending} changes`} waiting to sync\n      <\/span>/'
git add web/src/components/StatusBanner.tsx

icon assets/icon.png '\x77' 2048
git add assets/icon.png

# Only its permissions change.
chmod +x scripts/release.sh
git add scripts/release.sh

# Staged, then edited again: one entry, partly staged.
change web/src/api.ts 's/const BASE_URL = "https:\/\/api.atlas.example\/v1";/const BASE_URL = "https:\/\/api.atlas.example\/v2";/'
git add web/src/api.ts
change web/src/api.ts 's/  await fetch\(`\$\{BASE_URL\}\/sync\/upload`, \{ method: "POST", body: payload \}\);/  const response = await fetch(`\${BASE_URL}\/sync\/upload`, { method: "POST", body: payload });\n  if (!response.ok) {\n    throw new Error(`Upload failed: \${response.status}`);\n  }/'

# Not staged.

change backend/app/users.py 's/def initials\(user: User\) -> str:\n    parts = user.name.split\(\)\n    return "".join\(part\[0\].upper\(\) for part in parts\[:2\]\)/def initials(user: User, count: int = 2) -> str:\n    parts = display_name(user).split()\n    return "".join(part[0].upper() for part in parts[:count])/'
change backend/app/users.py 's/    for user in users:\n        if user.id == user_id:\n            return user\n    return None/    return next((user for user in users if user.id == user_id), None)/'

change web/src/styles/app.css 's/  padding: 8px 12px;/  padding: 8px 16px;\n  font-weight: 600;/'
change web/src/styles/app.css 's/\.profile \{\n  display: flex;\n  gap: 12px;\n\}\n/.account {\n  display: grid;\n  gap: 12px;\n}\n\n.account-name {\n  font-size: 1.25rem;\n}\n/'

# Tabs, one of them replaced by spaces.
change Makefile 's/\tcd web && npm run build/\tcd web \&\& npm ci \&\& npm run build/'
change Makefile 's/\trm -rf build\n/        rm -rf build web\/dist\n/'
printf '\ntest:\n\tcargo test --manifest-path core/Cargo.toml\n' >> Makefile

# The large file: one line in the middle, and lines at the end.
change data/cities.csv 's/^12000,City 12000,/12000,Reykjavik,/m'
awk 'BEGIN { for (i = 25001; i <= 25040; i++) printf "%d,City %d,0.00,0.00\n", i, i }' >> data/cities.csv

# The long line.
change web/dist/bundle.min.js 's/version:"0.4.0"/version:"0.5.0"/; s/atlas.m70=function\(a,b\)\{return a\+b\*70\}/atlas.m70=function(a,b){return a-b*70}/'

# Windows line endings, kept.
change docs/windows.txt 's/Run setup.exe, then sign in\./Run setup.exe as an administrator, then sign in./'

# The line ending after the last line goes.
printf '{\n  "offlineQueue": true,\n  "fullNames": true,\n  "syncInterval": 120\n}' > config/flags.json

change docs/guide.md 's/Zürich, São Paulo, 東京/Zürich, São Paulo, 東京, Kraków/; s/Pull to refresh ↓/Pull to refresh ↓ 🔄/'

# Only spaces change: tabs become four spaces in one function.
change backend/server/main.go 's/\tpending := queue.Len\(\)\n\tif pending > 0 \{\n\t\tfmt.Fprintf\(w, "%d pending", pending\)\n\t\treturn\n\t\}/    pending := queue.Len()\n    if pending > 0 {\n        fmt.Fprintf(w, "%d pending", pending)\n        return\n    }/'

rm config/ci.yml

# A file becomes a link.
rm docs/latest.md
ln -s guide.md docs/latest.md

# The submodule has a commit Atlas does not hold yet.
(
  cd vendor/palette
  as "Priya Nair" "2026-11-20T09:00:00+0100"
  change palette.json 's/"warning": "#d98c00"/"warning": "#c77700",\n  "danger": "#c0392b"/'
  git commit --quiet -am "Darken the warning, add danger"
)

# Untracked.

put ios/Sources/OfflineQueue.swift <<'EOF'
import Foundation

/// Keeps uploads while the device is offline, and sends them in order
/// when the network comes back.
actor OfflineQueue {
    private var pending: [Item] = []

    func add(_ item: Item) {
        pending.append(item)
    }

    func drain(with engine: SyncEngine) async throws {
        while let item = pending.first {
            try await engine.upload(item)
            pending.removeFirst()
        }
    }
}
EOF

put web/src/hooks/useSync.ts <<'EOF'
import { useEffect, useState } from "react";
import { syncStatus } from "../api";

export function useSync(intervalMs = 5000): number {
  const [pending, setPending] = useState(0);
  useEffect(() => {
    const timer = setInterval(async () => setPending(await syncStatus()), intervalMs);
    return () => clearInterval(timer);
  }, [intervalMs]);
  return pending;
}
EOF

put web/src/hooks/useOnline.ts <<'EOF'
import { useSyncExternalStore } from "react";

export function useOnline(): boolean {
  return useSyncExternalStore(
    (notify) => {
      window.addEventListener("online", notify);
      window.addEventListener("offline", notify);
      return () => {
        window.removeEventListener("online", notify);
        window.removeEventListener("offline", notify);
      };
    },
    () => navigator.onLine,
  );
}
EOF

icon assets/splash.png '\x33' 4096

# A name with spaces, one that starts with a dash, and an empty file.
put "docs/release notes (draft).md" <<'EOF'
# 0.5.0

- Uploads wait in a queue while the device is offline.
- A failed upload is tried five times, each after a longer wait.
EOF
printf 'Check the queue on a slow network.\n' > ./-todo.txt
mkdir -p notes && : > notes/empty.txt

# Ignored: never listed.
mkdir -p build && printf 'compiled\n' > build/output.log

# --------------------------------------------------------------------- many

repository "$root/many"
as "Alex Martin" "2026-11-01T10:00:00+0100"
for area in api auth billing sync; do
  for layer in models views services tests; do
    mkdir -p "src/$area/$layer"
    for i in $(seq 1 25); do
      printf 'export const name = "%s/%s/%d";\n\nexport function value() {\n  return %d;\n}\n' "$area" "$layer" "$i" "$i" > "src/$area/$layer/module$i.ts"
    done
  done
done
awk 'BEGIN { for (i = 1; i <= 5000; i++) printf "line %d of the long file\n", i }' > long.txt
commit "Start"
find src -name '*.ts' -exec perl -pi -e 's/return (\d+);/return $1 * 2;/' {} +
perl -pi -e 's/^line (\d+) of/line $1, changed, of/ if $. % 20 == 0' long.txt

# ------------------------------------------------------------------ merging

repository "$root/merging"
git config merge.conflictStyle zdiff3
as "Alex Martin" "2026-11-01T10:00:00+0100"
printf 'func upload(_ item: Item) async throws {\n    try await client.send(item)\n}\n' > SyncEngine.swift
printf '# Notes\n' > Notes.md
printf 'timeout = 30\n' > settings.toml
commit "Initial sync engine"
git switch --quiet -c feature/offline-sync
printf 'func upload(_ item: Item) async throws {\n    try await retry(times: 3) {\n        try await client.send(item)\n    }\n}\n' > SyncEngine.swift
commit "Retry failed uploads"
git switch --quiet main
printf 'func upload(_ item: Item) async throws {\n    try await client.send(item, timeout: .seconds(30))\n}\n' > SyncEngine.swift
printf '# Notes\n\nUploads time out after 30 s.\n' > Notes.md
commit "Add a 30 s timeout to uploads"
git switch --quiet feature/offline-sync
git merge main > /dev/null 2>&1 || true
# Merged by itself: Notes.md, staged. Edited since the merge stopped:
printf 'timeout = 60\n' > settings.toml

# ------------------------------------------------------------------- unborn

repository "$root/unborn"
printf '# A new project\n' > README.md
printf 'fn main() {\n    println!("hello");\n}\n' > main.rs
git add README.md
printf '\nNothing is committed yet.\n' >> README.md

# -------------------------------------------------------------------- clean

repository "$root/clean"
as "Alex Martin" "2026-11-01T10:00:00+0100"
printf '# Nothing to show\n' > README.md
commit "Start"

cat <<EOF
Repositories with something to read, in $root:

  changes/atlas     on feature/offline-queue, 3 commits ahead of main and 1 behind
    Staged      queue.rs (6 changes), RetryPolicy.kt (new), OldCache.m (deleted),
                notes.md → sync-notes.md (renamed, unchanged),
                Banner.tsx → StatusBanner.tsx (renamed and edited),
                icon.png (binary), release.sh (permissions only)
    Not staged  api.ts (partly staged), users.py, app.css, Makefile (tabs),
                cities.csv (25,040 lines), bundle.min.js (one line of 6,000
                characters), windows.txt (CRLF), flags.json (no line ending at
                the end), guide.md (accents, emoji), main.go (spaces only),
                ci.yml (deleted), latest.md (a file become a link),
                vendor/palette (a repository of its own)
    Untracked   OfflineQueue.swift, hooks/ (2 files), splash.png (binary),
                "release notes (draft).md", -todo.txt, notes/empty.txt
    Try         git scry diff            git scry diff --staged
                git scry diff main       git scry diff main..HEAD
                git scry diff main...HEAD
                git scry diff HEAD~2     git scry diff -- web
                git scry diff nowhere    (the error stays in the terminal)
                git difftool HEAD~1      git difftool --dir-diff main
  changes/many      400 changed files in 16 folders, and long.txt with 250 changes
  changes/merging   a merge stopped on SyncEngine.swift: \`git scry\` opens the
                    overview, \`git scry diff\` the change with its conflicted file
  changes/unborn    no commit yet: one file staged and edited, one untracked
  changes/clean     nothing to show
EOF
