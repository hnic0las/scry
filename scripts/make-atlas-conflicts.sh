#!/usr/bin/env bash
# Builds three copies of a multi-platform app ("Atlas") whose branch
# feature/sync-v2 and main changed the same lines in many languages, each
# stopped on conflicts:
#
#   conflicts/interactive-rebase   the branch rebased interactively onto main
#   conflicts/simple-rebase        the same commits rebased with `git rebase main`
#   conflicts/big-merge            main merged into the branch, all at once
#
# The rebases stop at four of their seven steps:
#
#   1. pick    Queue uploads while offline     Swift, Kotlin, Rust, Xcode project
#   2. reword  Explain the sync queue          (applies cleanly)
#   3. pick    Show the user's full name       TypeScript, TSX, CSS, Python
#   4. fixup   fixup! Show the user's full name (applies cleanly)
#   5. pick    Upgrade dependencies and CI     Ruby, JSON, TOML, YAML, Bash,
#                                              Objective-C deleted on one side,
#                                              a binary icon, lockfiles and
#                                              Swift package pins
#   6. drop    WIP debug logging
#   7. pick    Translate the sync messages     String catalog, Go
#
# The simple rebase picks all seven. Each stop leaves the next ones
# intact, whatever you choose:
#
#   scripts/make-atlas-conflicts.sh      (make-conflicts.sh runs it too)
#   open the app on one of them (Open Repository…, ⌘O)
#
# Like make-conflicts.sh, the repositories ignore the rest of your git
# configuration (no signing, no hooks).
set -euo pipefail

workspace="$(cd "$(dirname "$0")/.." && pwd)"
root="$workspace/conflicts"
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

# locks <base|feature|main>: each side's lockfiles and Swift package pins,
# written as their tools write them. The cases they show:
#
# - ios/Package.resolved: a package added (lottie-spm) and one bumped
#   (swift-log) on the branch, one removed on main (swift-algorithms):
#   merged by themselves; swift-collections bumped on both sides, and
#   swift-snapshot-testing pinned to a branch on one side and a release on
#   the other: one choice each; originHash changed on both sides.
# - ios/Podfile.lock: both sides upgrade Alamofire and add a pod, and the
#   Podfile itself conflicts: resolve it first.
# - core/Cargo.lock: two versions of syn held at once, tokio-macros added
#   on main, and Cargo.toml conflicting too.
# - web/package-lock.json: package.json merges cleanly (React on the
#   branch, Vite and TypeScript on main), but its lockfile does not, and
#   a transitive package (csstype) moved on both sides.
# - backend/server/go.sum: checksums, which keep both sides' lines.
# - ios/Gemfile.lock (fastlane and CocoaPods through Bundler): the Gemfile
#   merges cleanly (fastlane on the branch, cocoapods on main), the lock
#   does not: both sides ran a different Bundler (BUNDLED WITH) on a
#   different Mac (PLATFORMS), and a shared dependency moved on both.
# - backend/uv.lock (the Python API, with uv): pillow added on the branch,
#   FastAPI and Starlette bumped on main, pydantic and pydantic-core moved
#   on both sides; pyproject.toml merges cleanly.
locks() {
python3 - "$1" <<'EOF'
import base64, hashlib, json, os, sys

side = sys.argv[1]


def put(path, text):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w") as file:
        file.write(text)


def digest(*parts):
    return hashlib.sha256("/".join(parts).encode()).digest()


def hexdigest(*parts, length=40):
    return digest(*parts).hex()[:length]


# Swift packages: identity → (URL, version or "branch:<name>").
pins = {
    "base": {
        "swift-algorithms": ("https://github.com/apple/swift-algorithms.git", "1.1.0"),
        "swift-collections": ("https://github.com/apple/swift-collections.git", "1.0.6"),
        "swift-log": ("https://github.com/apple/swift-log.git", "1.5.3"),
        "swift-snapshot-testing": ("https://github.com/pointfreeco/swift-snapshot-testing", "1.15.0"),
    },
    "feature": {
        "lottie-spm": ("https://github.com/airbnb/lottie-spm.git", "4.4.1"),
        "swift-algorithms": ("https://github.com/apple/swift-algorithms.git", "1.1.0"),
        "swift-collections": ("https://github.com/apple/swift-collections.git", "1.1.0"),
        "swift-log": ("https://github.com/apple/swift-log.git", "1.5.4"),
        "swift-snapshot-testing": ("https://github.com/pointfreeco/swift-snapshot-testing", "branch:main"),
    },
    "main": {
        "swift-collections": ("https://github.com/apple/swift-collections.git", "1.0.7"),
        "swift-log": ("https://github.com/apple/swift-log.git", "1.5.3"),
        "swift-snapshot-testing": ("https://github.com/pointfreeco/swift-snapshot-testing", "1.16.0"),
    },
}[side]
entries = []
for identity in sorted(pins):
    url, version = pins[identity]
    revision = hexdigest(identity, version)
    if version.startswith("branch:"):
        state = {"branch": version[7:], "revision": revision}
    else:
        state = {"revision": revision, "version": version}
    entries.append({"identity": identity, "kind": "remoteSourceControl", "location": url, "state": state})
resolved = {"originHash": hexdigest("origin", side, length=64), "pins": entries, "version": 3}
put("ios/Package.resolved", json.dumps(resolved, indent=2, separators=(",", " : ")) + "\n")

# CocoaPods: pod → (version, requirement in the Podfile).
pods = {
    "base": {"Alamofire": ("5.8.0", "~> 5.8"), "KeychainAccess": ("4.2.2", "~> 4.2")},
    "feature": {"Alamofire": ("5.9.0", "~> 5.9"), "KeychainAccess": ("4.2.2", "~> 4.2"), "Reachability": ("3.2", "~> 3.2")},
    "main": {"Alamofire": ("5.8.1", "~> 5.8.1"), "KeychainAccess": ("4.2.2", "~> 4.2"), "Kingfisher": ("7.10.0", "~> 7.10")},
}[side]
names = sorted(pods)
lock = "PODS:\n" + "".join(f"  - {name} ({pods[name][0]})\n" for name in names)
lock += "\nDEPENDENCIES:\n" + "".join(f"  - {name} ({pods[name][1]})\n" for name in names)
lock += "\nSPEC REPOS:\n  trunk:\n" + "".join(f"    - {name}\n" for name in names)
lock += "\nSPEC CHECKSUMS:\n" + "".join(f"  {name}: {hexdigest(name, pods[name][0])}\n" for name in names)
lock += f"\nPODFILE CHECKSUM: {hexdigest('Podfile', side)}\n"
lock += "\nCOCOAPODS: " + {"base": "1.14.3", "feature": "1.15.2", "main": "1.16.2"}[side] + "\n"
put("ios/Podfile.lock", lock)

# Cargo: (name, version, dependencies), in Cargo's order.
crates = {
    "base": [("atlas-core", "0.4.0", ["serde", "tokio", "uniffi"]), ("serde", "1.0.190", ["serde_derive"]),
             ("serde_derive", "1.0.190", ["syn 2.0.39"]), ("syn", "1.0.109", []), ("syn", "2.0.39", []),
             ("tokio", "1.33.0", []), ("uniffi", "0.25.3", ["syn 1.0.109"])],
    "feature": [("atlas-core", "0.5.0", ["serde", "tokio", "uniffi"]), ("serde", "1.0.197", ["serde_derive"]),
                ("serde_derive", "1.0.197", ["syn 2.0.52"]), ("syn", "1.0.109", []), ("syn", "2.0.52", []),
                ("tokio", "1.36.0", []), ("uniffi", "0.26.1", ["syn 1.0.109"])],
    "main": [("atlas-core", "0.4.1", ["serde", "tokio", "uniffi"]), ("serde", "1.0.195", ["serde_derive"]),
             ("serde_derive", "1.0.195", ["syn 2.0.48"]), ("syn", "1.0.109", []), ("syn", "2.0.48", []),
             ("tokio", "1.35.1", ["tokio-macros"]), ("tokio-macros", "2.2.0", ["syn 2.0.48"]),
             ("uniffi", "0.25.3", ["syn 1.0.109"])],
}[side]
cargo = "# This file is automatically @generated by Cargo.\n# It is not intended for manual editing.\nversion = 3\n"
for name, version, dependencies in crates:
    cargo += f'\n[[package]]\nname = "{name}"\nversion = "{version}"\n'
    if name != "atlas-core":
        cargo += 'source = "registry+https://github.com/rust-lang/crates.io-index"\n'
        cargo += f'checksum = "{hexdigest(name, version, length=64)}"\n'
    if dependencies:
        cargo += "dependencies = [\n" + "".join(f' "{dependency}",\n' for dependency in dependencies) + "]\n"
put("core/Cargo.lock", cargo)

# npm: package.json changes on lines far apart, so it merges; its lockfile
# does not.
react = {"base": "18.2.0", "feature": "18.3.1", "main": "18.2.0"}[side]
typescript, vite = {"base": ("5.3.3", "5.0.12"), "feature": ("5.3.3", "5.0.12"), "main": ("5.4.5", "5.2.11")}[side]
manifest = {
    "name": "atlas-web",
    "private": True,
    "dependencies": {"react": f"^{react}", "react-dom": f"^{react}"},
    "scripts": {"build": "tsc && vite build", "dev": "vite", "preview": "vite preview"},
    "devDependencies": {"typescript": f"^{typescript}", "vite": f"^{vite}"},
}
put("web/package.json", json.dumps(manifest, indent=2) + "\n")
modules = {
    "csstype": ({"base": "3.1.1", "feature": "3.1.3", "main": "3.1.2"}[side], True),
    "js-tokens": ("4.0.0", False),
    "loose-envify": ("1.4.0", False),
    "react": (react, False),
    "react-dom": (react, False),
    "scheduler": ({"base": "0.23.0", "feature": "0.23.2", "main": "0.23.0"}[side], False),
    "typescript": (typescript, True),
    "vite": (vite, True),
}
if side == "main":
    modules["vite/node_modules/esbuild"] = ("0.20.2", True)
packages = {"": {"name": "atlas-web", "dependencies": manifest["dependencies"], "devDependencies": manifest["devDependencies"]}}
for path in sorted(modules):
    version, dev = modules[path]
    name = path.rsplit("/", 1)[-1]
    entry = {
        "version": version,
        "resolved": f"https://registry.npmjs.org/{name}/-/{name}-{version}.tgz",
        "integrity": "sha512-" + base64.b64encode(digest(name, version) * 2).decode(),
    }
    if dev:
        entry["dev"] = True
    packages["node_modules/" + path] = entry
put("web/package-lock.json", json.dumps({"name": "atlas-web", "lockfileVersion": 3, "requires": True, "packages": packages}, indent=2) + "\n")

# Go: go.mod changes apart (uuid added at the end, chi bumped at the top);
# go.sum's lines meet.
chi = {"base": "v5.0.10", "feature": "v5.0.10", "main": "v5.0.12"}[side]
modules = [("github.com/go-chi/chi/v5", chi), ("golang.org/x/sync", "v0.5.0")]
go_mod = f"module github.com/atlas/backend\n\ngo 1.22\n\nrequire github.com/go-chi/chi/v5 {chi}\n\nrequire golang.org/x/sync v0.5.0\n"
if side == "feature":
    modules.append(("github.com/google/uuid", "v1.6.0"))
    go_mod += "\nrequire github.com/google/uuid v1.6.0\n"
put("backend/server/go.mod", go_mod)
lines = []
for module, version in modules:
    for suffix in ("", "/go.mod"):
        hash = base64.b64encode(digest(module, version, suffix)).decode()
        lines.append(f"{module} {version}{suffix} h1:{hash}")
put("backend/server/go.sum", "".join(line + "\n" for line in sorted(lines)))

# Bundler: the Gemfile changes on lines far apart.
cocoapods = {"base": "1.14", "feature": "1.14", "main": "1.16"}[side]
fastlane = {"base": "2.219", "feature": "2.220", "main": "2.219"}[side]
put("ios/Gemfile", f'source "https://rubygems.org"\n\ngem "cocoapods", "~> {cocoapods}"\n\n# Release automation (scripts/release.sh)\ngem "fastlane", "~> {fastlane}"\n')
gems = {
    "base": {"activesupport": ("7.1.3", ["concurrent-ruby (~> 1.0)"]), "cocoapods": ("1.14.3", ["activesupport (>= 5.0, < 8)"]),
             "concurrent-ruby": ("1.2.2", []), "fastlane": ("2.219.0", ["xcpretty (~> 0.3.0)"]), "xcpretty": ("0.3.0", [])},
    "feature": {"activesupport": ("7.1.3", ["concurrent-ruby (~> 1.0)"]), "cocoapods": ("1.14.3", ["activesupport (>= 5.0, < 8)"]),
                "concurrent-ruby": ("1.2.3", []), "fastlane": ("2.220.0", ["xcpretty (~> 0.3.0)", "xcpretty-travis-formatter (>= 0.0.3)"]),
                "xcpretty": ("0.3.0", []), "xcpretty-travis-formatter": ("1.0.1", ["xcpretty (~> 0.2, >= 0.0.7)"])},
    "main": {"activesupport": ("7.1.3.4", ["concurrent-ruby (~> 1.0)"]), "cocoapods": ("1.16.2", ["activesupport (>= 5.0, < 8)"]),
             "concurrent-ruby": ("1.3.1", []), "fastlane": ("2.219.0", ["xcpretty (~> 0.3.0)"]), "xcpretty": ("0.3.0", [])},
}[side]
platforms = {"base": ["arm64-darwin-23", "ruby"], "feature": ["arm64-darwin-24", "ruby"], "main": ["arm64-darwin-23", "ruby", "x86_64-darwin-23"]}[side]
gemlock = "GEM\n  remote: https://rubygems.org/\n  specs:\n"
for name in sorted(gems):
    version, requirements = gems[name]
    gemlock += f"    {name} ({version})\n" + "".join(f"      {requirement}\n" for requirement in requirements)
gemlock += "\nPLATFORMS\n" + "".join(f"  {platform}\n" for platform in platforms)
gemlock += f"\nDEPENDENCIES\n  cocoapods (~> {cocoapods})\n  fastlane (~> {fastlane})\n"
gemlock += "\nBUNDLED WITH\n   " + {"base": "2.5.3", "feature": "2.5.9", "main": "2.5.11"}[side] + "\n"
put("ios/Gemfile.lock", gemlock)

# uv: pyproject.toml changes on lines far apart.
fastapi = {"base": "0.110", "feature": "0.110", "main": "0.111"}[side]
dependencies = [f"fastapi>={fastapi}", "pydantic>=2.6", "uvicorn>=0.29"] + (["pillow>=10.3"] if side == "feature" else [])
put("backend/pyproject.toml", '[project]\nname = "atlas-backend"\nversion = "0.4.0"\nrequires-python = ">=3.12"\ndependencies = [\n' + "".join(f'    "{dependency}",\n' for dependency in dependencies) + "]\n")
wheels = {
    "base": {"fastapi": "0.110.0", "pydantic": "2.6.4", "pydantic-core": "2.16.3", "starlette": "0.36.3", "uvicorn": "0.29.0"},
    "feature": {"fastapi": "0.110.0", "pillow": "10.3.0", "pydantic": "2.7.1", "pydantic-core": "2.18.2", "starlette": "0.36.3", "uvicorn": "0.29.0"},
    "main": {"fastapi": "0.111.0", "pydantic": "2.7.0", "pydantic-core": "2.18.1", "starlette": "0.37.2", "uvicorn": "0.29.0"},
}[side]
requires = {"fastapi": ["pydantic", "starlette"], "pydantic": ["pydantic-core"]}
uvlock = 'version = 1\nrequires-python = ">=3.12"\n'
uvlock += '\n[[package]]\nname = "atlas-backend"\nversion = "0.4.0"\nsource = { editable = "." }\ndependencies = [\n'
uvlock += "".join(f'    {{ name = "{dependency.split(">=")[0]}" }},\n' for dependency in sorted(dependencies)) + "]\n"
for name in sorted(wheels):
    version = wheels[name]
    file = name.replace("-", "_") + "-" + version
    uvlock += f'\n[[package]]\nname = "{name}"\nversion = "{version}"\nsource = {{ registry = "https://pypi.org/simple" }}\n'
    if name in requires:
        uvlock += "dependencies = [\n" + "".join(f'    {{ name = "{dependency}" }},\n' for dependency in requires[name]) + "]\n"
    uvlock += f'sdist = {{ url = "https://files.pythonhosted.org/packages/{file}.tar.gz", hash = "sha256:{hexdigest(name, version, length=64)}", size = {len(name) * 9173} }}\n'
    uvlock += f'wheels = [\n    {{ url = "https://files.pythonhosted.org/packages/{file}-py3-none-any.whl", hash = "sha256:{hexdigest(name, version, "whl", length=64)}", size = {len(name) * 4211} }},\n]\n'
put("backend/uv.lock", uvlock)
EOF
}

# build <dir>: the Atlas repository, main and feature/sync-v2 diverged,
# on main.
build() {
dir="$1"
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

/* Begin PBXProject section */
		A9000001 /* Project object */ = {isa = PBXProject; mainGroup = A3000001 /* Sources */; };
/* End PBXProject section */

/* Begin XCBuildConfiguration section */
		A4000001 /* Release */ = {
			isa = XCBuildConfiguration;
			buildSettings = {
				CURRENT_PROJECT_VERSION = 41;
				IPHONEOS_DEPLOYMENT_TARGET = 16.0;
				MARKETING_VERSION = 1.3;
			};
			name = Release;
		};
/* End XCBuildConfiguration section */
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

locks base

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
# A Widgets group that lists ForecastView.swift, which main deletes: git
# merges it without a conflict, yet the project would be broken.
s = s.replace(
    "/* End PBXGroup section */",
    "\t\tA3000002 /* Widgets */ = {\n\t\t\tisa = PBXGroup;\n\t\t\tchildren = (\n\t\t\t\tA2000002 /* ForecastView.swift */,\n\t\t\t);\n\t\t\tpath = Widgets;\n\t\t\tsourceTree = \"<group>\";\n\t\t};\n/* End PBXGroup section */")
s = s.replace("CURRENT_PROJECT_VERSION = 41;", "CURRENT_PROJECT_VERSION = 42;")
s = s.replace("MARKETING_VERSION = 1.3;", "MARKETING_VERSION = 1.4;")
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
# Objective-C file, a binary icon, lockfiles and Swift package pins.
put ios/Podfile <<'EOF'
platform :ios, '17.0'

target 'Atlas' do
  use_frameworks!
  pod 'Alamofire', '~> 5.9'
  pod 'KeychainAccess', '~> 4.2'
  pod 'Reachability', '~> 3.2'
end
EOF
locks feature
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
# ForecastView.swift is replaced by ForecastCache.swift, in the same places.
s = s.replace(
    "\t\tA1000002 /* ForecastView.swift in Sources */ = {isa = PBXBuildFile; fileRef = A2000002 /* ForecastView.swift */; };\n",
    "\t\tA1000004 /* ForecastCache.swift in Sources */ = {isa = PBXBuildFile; fileRef = A2000004 /* ForecastCache.swift */; };\n")
s = s.replace(
    "\t\tA2000002 /* ForecastView.swift */ = {isa = PBXFileReference; lastKnownFileType = sourcecode.swift; path = ForecastView.swift; sourceTree = \"<group>\"; };\n",
    "\t\tA2000004 /* ForecastCache.swift */ = {isa = PBXFileReference; lastKnownFileType = sourcecode.swift; path = ForecastCache.swift; sourceTree = \"<group>\"; };\n")
s = s.replace(
    "\t\t\t\tA2000002 /* ForecastView.swift */,\n\t\t\t);",
    "\t\t\t\tA2000004 /* ForecastCache.swift */,\n\t\t\t);")
s = s.replace("CURRENT_PROJECT_VERSION = 41;", "CURRENT_PROJECT_VERSION = 50;")
s = s.replace("IPHONEOS_DEPLOYMENT_TARGET = 16.0;", "IPHONEOS_DEPLOYMENT_TARGET = 16.4;")
s = s.replace("MARKETING_VERSION = 1.3;", "MARKETING_VERSION = 2.0;")
open(p, "w").write(s)
EOF
commit "Add a 30 s upload timeout; replace ForecastView with a forecast cache; version 2.0 (#412)"

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
locks main
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

git switch --quiet feature/sync-v2
}

# ------------------------------------------------------- interactive rebase

build "$root/interactive-rebase"
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


# ------------------------------------------------------------ simple rebase

# Every commit picked, in order: it stops at the same steps.
build "$root/simple-rebase"
git rebase main > /dev/null 2>&1 || true

# -------------------------------------------------------------------- merge

# main merged into the branch: every conflict at once, in one stop.
build "$root/big-merge"
git merge --no-edit main > /dev/null 2>&1 || true

echo "Atlas repositories, in $root:"
echo "  conflicts/interactive-rebase  stopped on step 1 of 7 (4 steps conflict)"
echo "  conflicts/simple-rebase       stopped on commit 1 of 7 (4 commits conflict)"
echo "  conflicts/big-merge           main merged into feature/sync-v2, every conflict at once"
