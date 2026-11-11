#!/usr/bin/env bash
# Builds conflicts/interactive-rebase: a multi-platform app ("Atlas") whose
# feature branch is rebased interactively onto main, and stops on conflicts
# at four of its steps, in many languages:
#
#   1. pick    Queue uploads while offline     Swift, Kotlin, Rust, Xcode project
#   2. reword  Explain the sync queue          (applies cleanly)
#   3. pick    Show the user's full name       TypeScript, TSX, CSS, Python
#   4. fixup   fixup! Show the user's full name (applies cleanly)
#   5. pick    Upgrade dependencies and CI     Ruby, JSON, TOML, YAML, Bash,
#                                              Objective-C deleted on one side,
#                                              a binary icon
#   6. drop    WIP debug logging
#   7. pick    Translate the sync messages     String catalog, Go
#
# Each stop leaves the next ones intact, whatever you choose:
#
#   scripts/make-interactive-rebase.sh      (make-conflicts.sh runs it too)
#   open the app on conflicts/interactive-rebase (Open Repository…, ⌘O)
#
# Like make-conflicts.sh, the repository ignores the rest of your git
# configuration (no signing, no hooks).
set -euo pipefail

workspace="$(cd "$(dirname "$0")/.." && pwd)"
dir="$workspace/conflicts/interactive-rebase"
global_tool="$(git config --global --get merge.tool 2>/dev/null || true)"

export GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1
export GIT_AUTHOR_NAME="Alex Martin" GIT_AUTHOR_EMAIL=alex@example.com
export GIT_COMMITTER_NAME="Alex Martin" GIT_COMMITTER_EMAIL=alex@example.com
export GIT_EDITOR=true

# put <path>: writes stdin to the file, creating its folders.
put() {
  mkdir -p "$(dirname "$1")"
  cat > "$1"
}

# icon <path> <colour byte>: a tiny binary file, different per colour.
icon() {
  mkdir -p "$(dirname "$1")"
  printf '\x89PNG\r\n\x1a\n\x00\x00\x00\x0dIHDR\x00\x00\x00\x10\x00\x00\x00\x10\x08\x02\x00\x00\x00%b\x00\x00\x00' "$2" > "$1"
}

commit() {
  git add -A && git commit --quiet -m "$1"
}

rm -rf "$dir"
mkdir -p "$dir" && cd "$dir"
git init --quiet --initial-branch=main
git config merge.conflictStyle zdiff3
if [ "$global_tool" != scry ]; then
  git config merge.tool scry
  git config mergetool.scry.cmd "cargo run -q --manifest-path '$workspace/Cargo.toml' -p git-scry -- resolve \"\$BASE\" \"\$LOCAL\" \"\$REMOTE\" \"\$MERGED\""
  git config mergetool.scry.trustExitCode true
fi
git config mergetool.prompt false
git config mergetool.keepBackup false

# ---------------------------------------------------------------- base

put README.md <<'EOF'
# Atlas

Weather for iOS, Android and the web, synced through one Rust core.
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

put ios/Sources/Legacy/OldCache.m <<'EOF'
#import "OldCache.h"

@implementation OldCache

- (nullable NSData *)dataForKey:(NSString *)key {
    return [self.storage objectForKey:key];
}

- (void)setData:(NSData *)data forKey:(NSString *)key {
    [self.storage setObject:data forKey:key];
}

@end
EOF

put ios/Atlas.xcodeproj/project.pbxproj <<'EOF'
// !$*UTF8*$!
{
	archiveVersion = 1;
	objectVersion = 77;
	objects = {

/* Begin PBXBuildFile section */
		A1000001 /* SyncEngine.swift in Sources */ = {isa = PBXBuildFile; fileRef = A2000001 /* SyncEngine.swift */; };
		A1000002 /* ForecastView.swift in Sources */ = {isa = PBXBuildFile; fileRef = A2000002 /* ForecastView.swift */; };
/* End PBXBuildFile section */

/* Begin PBXFileReference section */
		A2000001 /* SyncEngine.swift */ = {isa = PBXFileReference; lastKnownFileType = sourcecode.swift; path = SyncEngine.swift; sourceTree = "<group>"; };
		A2000002 /* ForecastView.swift */ = {isa = PBXFileReference; lastKnownFileType = sourcecode.swift; path = ForecastView.swift; sourceTree = "<group>"; };
/* End PBXFileReference section */

/* Begin PBXGroup section */
		A3000001 /* Sources */ = {
			isa = PBXGroup;
			children = (
				A2000001 /* SyncEngine.swift */,
				A2000002 /* ForecastView.swift */,
			);
			path = Sources;
			sourceTree = "<group>";
		};
/* End PBXGroup section */
	};
	rootObject = A9000001 /* Project object */;
}
EOF

put android/app/src/main/java/com/atlas/sync/SyncWorker.kt <<'EOF'
package com.atlas.sync

import androidx.work.CoroutineWorker
import androidx.work.WorkerParameters

class SyncWorker(context: Context, params: WorkerParameters) : CoroutineWorker(context, params) {

    override suspend fun doWork(): Result {
        val pending = repository.pendingItems()
        pending.forEach { item ->
            api.send(item)
        }
        return Result.success()
    }

    companion object {
        const val TAG = "sync"
        const val MAX_ATTEMPTS = 3
    }
}
EOF

put core/Cargo.toml <<'EOF'
[package]
name = "atlas-core"
version = "0.4.0"
edition = "2021"

[dependencies]
serde = { version = "1.0.190", features = ["derive"] }
tokio = { version = "1.33", features = ["rt", "sync"] }
uniffi = "0.25"
EOF

put core/src/queue.rs <<'EOF'
use std::collections::VecDeque;

/// Changes waiting to be sent, oldest first.
pub struct Queue {
    items: VecDeque<Change>,
}

impl Queue {
    pub fn new() -> Queue {
        Queue {
            items: VecDeque::with_capacity(64),
        }
    }

    pub fn push(&mut self, change: Change) {
        self.items.push_back(change);
    }

    pub fn next(&mut self) -> Option<Change> {
        self.items.pop_front()
    }
}
EOF

put web/src/api/user.ts <<'EOF'
export interface User {
  id: string;
  name: string;
  email: string;
}

export async function fetchUser(id: string): Promise<User> {
  const response = await fetch(`/api/users/${id}`);
  if (!response.ok) {
    throw new Error(`User ${id}: ${response.status}`);
  }
  return response.json();
}

export function initials(user: User): string {
  return user.name.slice(0, 1).toUpperCase();
}
EOF

put web/src/components/Profile.tsx <<'EOF'
import { User, initials } from "../api/user";
import "../styles/profile.css";

export function Profile({ user }: { user: User }) {
  return (
    <section className="profile">
      <span className="avatar">{initials(user)}</span>
      <h2>{user.name}</h2>
      <a href={`mailto:${user.email}`}>{user.email}</a>
    </section>
  );
}
EOF

put web/src/styles/profile.css <<'EOF'
.profile {
  display: flex;
  gap: 12px;
  align-items: center;
}

.profile h2 {
  font-size: 18px;
  font-weight: 600;
  color: #1d1d1f;
}

.avatar {
  width: 32px;
  height: 32px;
  border-radius: 50%;
}
EOF

put backend/app/users.py <<'EOF'
from dataclasses import dataclass


@dataclass
class User:
    id: str
    name: str
    email: str


def to_json(user: User) -> dict:
    return {
        "id": user.id,
        "name": user.name,
        "email": user.email,
    }


def find(store, user_id: str) -> User | None:
    row = store.get(user_id)
    return User(**row) if row else None
EOF

put backend/app/serializers.py <<'EOF'
def page(items: list, size: int = 20) -> dict:
    return {"items": items[:size], "more": len(items) > size}
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
	fmt.Fprintf(w, "%d changes waiting", pending)
}

func main() {
	http.HandleFunc("/sync/status", syncStatus)
	log.Println("listening on :8080")
	log.Fatal(http.ListenAndServe(":8080", nil))
}
EOF

put ios/Podfile <<'EOF'
platform :ios, '16.0'

target 'Atlas' do
  use_frameworks!
  pod 'Alamofire', '~> 5.8'
  pod 'KeychainAccess', '~> 4.2'
end
EOF

put ios/Package.resolved <<'EOF'
{
  "pins" : [
    {
      "identity" : "swift-collections",
      "kind" : "remoteSourceControl",
      "location" : "https://github.com/apple/swift-collections.git",
      "state" : {
        "revision" : "94cf62b3ba8d4bed62680a282d4c25f9c63c2efb",
        "version" : "1.0.6"
      }
    }
  ],
  "version" : 2
}
EOF

put .github/workflows/ci.yml <<'EOF'
name: CI
on: [push, pull_request]

jobs:
  ios:
    runs-on: macos-14
    steps:
      - uses: actions/checkout@v4
      - run: sudo xcode-select -s /Applications/Xcode_15.2.app
      - run: xcodebuild test -scheme Atlas -destination 'platform=iOS Simulator,name=iPhone 15'
EOF

put scripts/release.sh <<'EOF'
#!/usr/bin/env bash
set -euo pipefail

version="$1"
echo "Releasing Atlas $version"
fastlane ios beta version:"$version"
EOF

put ios/Resources/Localizable.xcstrings <<'EOF'
{
  "sourceLanguage" : "en",
  "strings" : {
    "sync.done" : {
      "localizations" : {
        "fr" : {
          "stringUnit" : {
            "state" : "translated",
            "value" : "Synchronisé"
          }
        }
      }
    },
    "sync.pending" : {
      "localizations" : {
        "fr" : {
          "stringUnit" : {
            "state" : "translated",
            "value" : "%lld modifications en attente"
          }
        }
      }
    }
  },
  "version" : "1.0"
}
EOF

icon ios/Resources/AppIcon.png '\x01'
commit "Initial Atlas app"

# ---------------------------------------------------------- feature branch

git switch --quiet -c feature/sync-v2

# 1. Queue uploads while offline: Swift, Kotlin, Rust, Xcode project.
put ios/Sources/SyncEngine.swift <<'EOF'
import Foundation

/// Sends local changes to the server and fetches remote ones.
final class SyncEngine {
    private let client: APIClient
    private let queue = OperationQueue()
    private let offline = OfflineQueue()

    init(client: APIClient) {
        self.client = client
        queue.maxConcurrentOperationCount = 4
    }

    func upload(_ item: Item) async throws {
        guard client.isReachable else {
            offline.enqueue(item)
            return
        }
        try await client.send(item)
    }

    func download(_ id: Item.ID) async throws -> Item {
        try await client.fetch(id)
    }

    func pause() {
        queue.isSuspended = true
    }
}
EOF
put ios/Sources/OfflineQueue.swift <<'EOF'
/// Items waiting for the network.
final class OfflineQueue {
    private var items: [Item] = []

    func enqueue(_ item: Item) {
        items.append(item)
    }
}
EOF
python3 - <<'EOF'
p = "ios/Atlas.xcodeproj/project.pbxproj"
s = open(p).read()
s = s.replace(
    "/* End PBXBuildFile section */",
    "\t\tA1000003 /* OfflineQueue.swift in Sources */ = {isa = PBXBuildFile; fileRef = A2000003 /* OfflineQueue.swift */; };\n/* End PBXBuildFile section */")
s = s.replace(
    "/* End PBXFileReference section */",
    "\t\tA2000003 /* OfflineQueue.swift */ = {isa = PBXFileReference; lastKnownFileType = sourcecode.swift; path = OfflineQueue.swift; sourceTree = \"<group>\"; };\n/* End PBXFileReference section */")
s = s.replace(
    "\t\t\t\tA2000002 /* ForecastView.swift */,\n\t\t\t);",
    "\t\t\t\tA2000002 /* ForecastView.swift */,\n\t\t\t\tA2000003 /* OfflineQueue.swift */,\n\t\t\t);")
open(p, "w").write(s)
EOF
put android/app/src/main/java/com/atlas/sync/SyncWorker.kt <<'EOF'
package com.atlas.sync

import androidx.work.CoroutineWorker
import androidx.work.WorkerParameters

class SyncWorker(context: Context, params: WorkerParameters) : CoroutineWorker(context, params) {

    override suspend fun doWork(): Result {
        if (!network.isConnected()) {
            return Result.retry()
        }
        val pending = repository.pendingItems()
        pending.forEach { item ->
            api.send(item)
            repository.markSent(item)
        }
        return Result.success()
    }

    companion object {
        const val TAG = "sync"
        const val MAX_ATTEMPTS = 5
    }
}
EOF
put core/src/queue.rs <<'EOF'
use std::collections::VecDeque;

/// Changes waiting to be sent, oldest first; kept while offline.
pub struct Queue {
    items: VecDeque<Change>,
    offline: bool,
}

impl Queue {
    pub fn new() -> Queue {
        Queue {
            items: VecDeque::with_capacity(256),
            offline: false,
        }
    }

    pub fn push(&mut self, change: Change) {
        self.items.push_back(change);
    }

    pub fn next(&mut self) -> Option<Change> {
        if self.offline {
            return None;
        }
        self.items.pop_front()
    }
}
EOF
commit "Queue uploads while offline"

# 2. Explain the sync queue: applies cleanly.
cat >> README.md <<'EOF'

## Sync

Changes made offline wait in a queue and are sent once the network is back.
EOF
commit "Explain the sync queue in the README"

# 3. Show the user's full name: TypeScript, TSX, CSS, Python.
put web/src/api/user.ts <<'EOF'
export interface User {
  id: string;
  firstName: string;
  lastName: string;
  email: string;
}

export async function fetchUser(id: string): Promise<User> {
  const response = await fetch(`/api/users/${id}`);
  if (!response.ok) {
    throw new Error(`User ${id}: ${response.status}`);
  }
  return response.json();
}

export function fullName(user: User): string {
  return `${user.firstName} ${user.lastName}`;
}

export function initials(user: User): string {
  return (user.firstName[0] + user.lastName[0]).toUpperCase();
}
EOF
put web/src/components/Profile.tsx <<'EOF'
import { User, fullName, initials } from "../api/user";
import "../styles/profile.css";

export function Profile({ user }: { user: User }) {
  return (
    <section className="profile">
      <span className="avatar">{initials(user)}</span>
      <h2 title={user.email}>{fullName(user)}</h2>
      <a href={`mailto:${user.email}`}>{user.email}</a>
    </section>
  );
}
EOF
put web/src/styles/profile.css <<'EOF'
.profile {
  display: flex;
  gap: 12px;
  align-items: center;
}

.profile h2 {
  font-size: 20px;
  font-weight: 700;
  letter-spacing: -0.01em;
  color: #1d1d1f;
}

.avatar {
  width: 40px;
  height: 40px;
  border-radius: 50%;
}
EOF
put backend/app/users.py <<'EOF'
from dataclasses import dataclass


@dataclass
class User:
    id: str
    first_name: str
    last_name: str
    email: str


def to_json(user: User) -> dict:
    return {
        "id": user.id,
        "firstName": user.first_name,
        "lastName": user.last_name,
        "email": user.email,
    }


def find(store, user_id: str) -> User | None:
    row = store.get(user_id)
    return User(**row) if row else None
EOF
commit "Show the user's full name"

# 4. A fixup of step 3, in a file main leaves alone: applies cleanly.
put backend/app/serializers.py <<'EOF'
def page(items: list, size: int = 20) -> dict:
    return {"items": items[:size], "more": len(items) > size}


def user_page(users: list) -> dict:
    return page([u.first_name + " " + u.last_name for u in users])
EOF
commit "fixup! Show the user's full name"

# 5. Upgrade dependencies and CI: Ruby, JSON, TOML, YAML, Bash, a deleted
# Objective-C file and a binary icon.
put ios/Podfile <<'EOF'
platform :ios, '17.0'

target 'Atlas' do
  use_frameworks!
  pod 'Alamofire', '~> 5.9'
  pod 'KeychainAccess', '~> 4.2'
  pod 'Reachability', '~> 3.2'
end
EOF
put ios/Package.resolved <<'EOF'
{
  "pins" : [
    {
      "identity" : "swift-collections",
      "kind" : "remoteSourceControl",
      "location" : "https://github.com/apple/swift-collections.git",
      "state" : {
        "revision" : "671108c96644956dddcd89dd59c203dcdb36cec7",
        "version" : "1.1.0"
      }
    }
  ],
  "version" : 2
}
EOF
put core/Cargo.toml <<'EOF'
[package]
name = "atlas-core"
version = "0.5.0"
edition = "2021"

[dependencies]
serde = { version = "1.0.197", features = ["derive"] }
tokio = { version = "1.36", features = ["rt", "sync", "time"] }
uniffi = "0.26"
EOF
put .github/workflows/ci.yml <<'EOF'
name: CI
on: [push, pull_request]

jobs:
  ios:
    runs-on: macos-15
    steps:
      - uses: actions/checkout@v4
      - run: sudo xcode-select -s /Applications/Xcode_16.1.app
      - run: xcodebuild test -scheme Atlas -destination 'platform=iOS Simulator,name=iPhone 16'
EOF
put scripts/release.sh <<'EOF'
#!/usr/bin/env bash
set -euo pipefail

version="$1"
echo "Releasing Atlas $version to TestFlight"
fastlane ios beta version:"$version" skip_waiting:true
EOF
git rm --quiet ios/Sources/Legacy/OldCache.m
icon ios/Resources/AppIcon.png '\x02'
commit "Upgrade dependencies and CI"

# 6. Dropped in the rebase.
put ios/Sources/Debug.swift <<'EOF'
func debugLog(_ message: String) { print("[sync]", message) }
EOF
commit "WIP debug logging"

# 7. Translate the sync messages: String catalog, Go.
python3 - <<'EOF'
p = "ios/Resources/Localizable.xcstrings"
s = open(p).read()
s = s.replace('"value" : "Synchronisé"', '"value" : "Tout est synchronisé"')
s = s.replace('"value" : "%lld modifications en attente"', '"value" : "%lld changements à envoyer"')
open(p, "w").write(s)
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
	if pending == 0 {
		fmt.Fprint(w, "everything is synced")
		return
	}
	fmt.Fprintf(w, "%d changes to send", pending)
}

func main() {
	http.HandleFunc("/sync/status", syncStatus)
	log.Println("listening on :8080")
	log.Fatal(http.ListenAndServe(":8080", nil))
}
EOF
commit "Translate the sync messages"

# ------------------------------------------------------------------- main

git switch --quiet main

put ios/Sources/SyncEngine.swift <<'EOF'
import Foundation

/// Sends local changes to the server and fetches remote ones.
final class SyncEngine {
    private let client: APIClient
    private let queue = OperationQueue()

    init(client: APIClient) {
        self.client = client
        queue.maxConcurrentOperationCount = 3
        queue.qualityOfService = .utility
    }

    func upload(_ item: Item) async throws {
        try await client.send(item, timeout: .seconds(30))
    }

    func download(_ id: Item.ID) async throws -> Item {
        try await client.fetch(id)
    }

    func cancel() {
        queue.cancelAllOperations()
        client.invalidate()
    }
}
EOF
put ios/Sources/ForecastCache.swift <<'EOF'
/// The last forecast, shown while a new one loads.
final class ForecastCache {}
EOF
python3 - <<'EOF'
p = "ios/Atlas.xcodeproj/project.pbxproj"
s = open(p).read()
s = s.replace(
    "/* End PBXBuildFile section */",
    "\t\tA1000004 /* ForecastCache.swift in Sources */ = {isa = PBXBuildFile; fileRef = A2000004 /* ForecastCache.swift */; };\n/* End PBXBuildFile section */")
s = s.replace(
    "/* End PBXFileReference section */",
    "\t\tA2000004 /* ForecastCache.swift */ = {isa = PBXFileReference; lastKnownFileType = sourcecode.swift; path = ForecastCache.swift; sourceTree = \"<group>\"; };\n/* End PBXFileReference section */")
s = s.replace(
    "\t\t\t\tA2000002 /* ForecastView.swift */,\n\t\t\t);",
    "\t\t\t\tA2000002 /* ForecastView.swift */,\n\t\t\t\tA2000004 /* ForecastCache.swift */,\n\t\t\t);")
open(p, "w").write(s)
EOF
commit "Add a 30 s upload timeout and a forecast cache (#412)"

put android/app/src/main/java/com/atlas/sync/SyncWorker.kt <<'EOF'
package com.atlas.sync

import androidx.work.CoroutineWorker
import androidx.work.WorkerParameters

class SyncWorker(context: Context, params: WorkerParameters) : CoroutineWorker(context, params) {

    override suspend fun doWork(): Result {
        val pending = repository.pendingItems()
        pending.chunked(BATCH_SIZE).forEach { batch ->
            api.sendAll(batch)
        }
        return Result.success()
    }

    companion object {
        const val TAG = "sync"
        const val MAX_ATTEMPTS = 3
        const val BATCH_SIZE = 20
    }
}
EOF
put core/src/queue.rs <<'EOF'
use std::collections::VecDeque;

/// Changes waiting to be sent, oldest first.
pub struct Queue {
    items: VecDeque<Change>,
    limit: usize,
}

impl Queue {
    pub fn new(limit: usize) -> Queue {
        Queue {
            items: VecDeque::with_capacity(limit),
            limit,
        }
    }

    pub fn push(&mut self, change: Change) {
        if self.items.len() == self.limit {
            self.items.pop_front();
        }
        self.items.push_back(change);
    }

    pub fn next(&mut self) -> Option<Change> {
        self.items.pop_front()
    }
}
EOF
commit "Send changes in batches, with a bounded queue"

put web/src/api/user.ts <<'EOF'
export interface User {
  id: string;
  displayName: string;
  email: string;
  avatarUrl?: string;
}

export async function fetchUser(id: string): Promise<User> {
  const response = await fetch(`/api/v2/users/${id}`);
  if (!response.ok) {
    throw new Error(`User ${id}: ${response.status}`);
  }
  return response.json();
}

export function initials(user: User): string {
  return user.displayName.slice(0, 2).toUpperCase();
}
EOF
put web/src/components/Profile.tsx <<'EOF'
import { User, initials } from "../api/user";
import "../styles/profile.css";

export function Profile({ user }: { user: User }) {
  return (
    <section className="profile">
      {user.avatarUrl ? <img className="avatar" src={user.avatarUrl} alt="" /> : <span className="avatar">{initials(user)}</span>}
      <h2>{user.displayName}</h2>
      <a href={`mailto:${user.email}`}>{user.email}</a>
    </section>
  );
}
EOF
put web/src/styles/profile.css <<'EOF'
.profile {
  display: flex;
  gap: 12px;
  align-items: center;
}

.profile h2 {
  font-size: 18px;
  font-weight: 600;
  color: var(--text-primary);
}

.avatar {
  width: 32px;
  height: 32px;
  border-radius: 8px;
  object-fit: cover;
}
EOF
put backend/app/users.py <<'EOF'
from dataclasses import dataclass


@dataclass
class User:
    id: str
    display_name: str
    email: str
    avatar_url: str | None = None


def to_json(user: User) -> dict:
    return {
        "id": user.id,
        "displayName": user.display_name,
        "email": user.email,
        "avatarUrl": user.avatar_url,
    }


def find(store, user_id: str) -> User | None:
    row = store.get(user_id)
    return User(**row) if row else None
EOF
commit "Rename name to displayName and add avatars (API v2)"

put ios/Podfile <<'EOF'
platform :ios, '16.4'

target 'Atlas' do
  use_frameworks!
  pod 'Alamofire', '~> 5.8.1'
  pod 'KeychainAccess', '~> 4.2'
  pod 'Kingfisher', '~> 7.10'
end
EOF
put ios/Package.resolved <<'EOF'
{
  "pins" : [
    {
      "identity" : "swift-collections",
      "kind" : "remoteSourceControl",
      "location" : "https://github.com/apple/swift-collections.git",
      "state" : {
        "revision" : "d029d9d39c87bed85b1c50adee7c41795261a192",
        "version" : "1.0.7"
      }
    }
  ],
  "version" : 2
}
EOF
put core/Cargo.toml <<'EOF'
[package]
name = "atlas-core"
version = "0.4.1"
edition = "2021"

[dependencies]
serde = { version = "1.0.195", features = ["derive"] }
tokio = { version = "1.35", features = ["rt", "sync", "macros"] }
uniffi = "0.25"
EOF
put .github/workflows/ci.yml <<'EOF'
name: CI
on: [push, pull_request]

jobs:
  ios:
    runs-on: macos-14
    steps:
      - uses: actions/checkout@v4
      - run: sudo xcode-select -s /Applications/Xcode_15.4.app
      - run: xcodebuild test -scheme Atlas -destination 'platform=iOS Simulator,name=iPhone 15 Pro'
EOF
put scripts/release.sh <<'EOF'
#!/usr/bin/env bash
set -euo pipefail

version="${1:?usage: release.sh <version>}"
echo "Releasing Atlas $version"
fastlane ios beta version:"$version" changelog:"$(git log -1 --format=%s)"
EOF
put ios/Sources/Legacy/OldCache.m <<'EOF'
#import "OldCache.h"

@implementation OldCache

- (nullable NSData *)dataForKey:(NSString *)key {
    @synchronized (self) {
        return [self.storage objectForKey:key];
    }
}

- (void)setData:(NSData *)data forKey:(NSString *)key {
    @synchronized (self) {
        [self.storage setObject:data forKey:key];
    }
}

@end
EOF
icon ios/Resources/AppIcon.png '\x03'
commit "Patch dependencies, Xcode 15.4, thread-safe OldCache, new icon"

python3 - <<'EOF'
p = "ios/Resources/Localizable.xcstrings"
s = open(p).read()
s = s.replace('"value" : "Synchronisé"', '"value" : "Synchronisation terminée"')
s = s.replace('"value" : "%lld modifications en attente"', '"value" : "%lld modifications à synchroniser"')
open(p, "w").write(s)
EOF
put backend/server/main.go <<'EOF'
package main

import (
	"encoding/json"
	"log"
	"net/http"
)

func syncStatus(w http.ResponseWriter, r *http.Request) {
	pending := queue.Len()
	w.Header().Set("Content-Type", "application/json")
	json.NewEncoder(w).Encode(map[string]int{"pending": pending})
}

func main() {
	http.HandleFunc("/sync/status", syncStatus)
	log.Println("listening on :8080")
	log.Fatal(http.ListenAndServe(":8080", nil))
}
EOF
commit "Serve the sync status as JSON, reword French strings"

# ------------------------------------------------------- interactive rebase

git switch --quiet feature/sync-v2
todo="$(mktemp)"
{
  echo "pick $(git rev-parse --short ':/Queue uploads while offline') Queue uploads while offline"
  echo "reword $(git rev-parse --short ':/Explain the sync queue') Explain the sync queue in the README"
  echo "pick $(git rev-parse --short ":/^Show the user's full name") Show the user's full name"
  echo "fixup $(git rev-parse --short ":/fixup! Show the user") fixup! Show the user's full name"
  echo "pick $(git rev-parse --short ':/Upgrade dependencies') Upgrade dependencies and CI"
  echo "drop $(git rev-parse --short ':/WIP debug logging') WIP debug logging"
  echo "pick $(git rev-parse --short ':/Translate the sync') Translate the sync messages"
} > "$todo"
GIT_SEQUENCE_EDITOR="cp '$todo'" git rebase -i main > /dev/null 2>&1 || true
rm -f "$todo"

echo "conflicts/interactive-rebase: stopped on step 1 of 7 (4 steps conflict)"
git status --short | sed 's/^/  /'
