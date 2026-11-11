# Scry

Scry (`git-scry`) is a local-first tool, integrated with git, for merging and reviewing changes. It resolves conflicts through `git mergetool`, reviews branches and pull requests, runs built-in checks such as secret scanning, and takes proposals from agents through MCP that a human validates.

Status: early development. The core, the `git-scry` CLI (merge tool in the terminal) and a first macOS merge editor work; review, MCP and agents are not built yet.

The binary is named `git-scry`, so that git runs it as `git scry`.

## Development

Requirements: Rust through rustup (the toolchain version is pinned in `rust-toolchain.toml`) and, later, Xcode for the Mac app.

On macOS with Homebrew:

```sh
brew install rustup
export PATH="/opt/homebrew/opt/rustup/bin:$PATH"   # rustup is keg-only
rustup default stable
```

Check, lint and test, as the CI does on macOS, Linux and Windows:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Try the CLI:

```sh
cargo run -p git-scry -- conflicts path/to/file
cargo run -p git-scry -- session    # the operation in progress and who is on each side
cargo run -p git-scry -- highlight path/to/file   # the syntax colours the app shows (--spans to list them)
```

Use it as git's merge tool, in the terminal:

```sh
git config --global merge.tool scry
git config --global mergetool.scry.cmd 'git-scry resolve "$BASE" "$LOCAL" "$REMOTE" "$MERGED"'
git config --global mergetool.scry.trustExitCode true
```

`git mergetool` then asks for a candidate per conflict (the same digits as the app's shortcuts: 1 ours, 2 theirs, 3 and 4 both, 5 and 6 both with each side closed, 0 base, 9 none) and writes the file only when every conflict is resolved. For 19 languages (Swift, Objective-C, C, C++, Rust, JavaScript, TypeScript, TSX, HTML, CSS, Python, Kotlin, Java, Go, Ruby, Bash, JSON including `.xcstrings`, YAML, TOML), each candidate is checked by parsing, when both sides' versions of the file parse (✗ when the file would not parse), and a file that does not parse is written only after confirmation, or with `--allow-invalid`.

Or in the Scry app: Scry › Settings › Use Scry as git's Merge Tool sets all three for you, running the `git-scry` inside the app (`Scry.app/Contents/Helpers/git-scry`, put there by the Xcode build), so nothing has to be on the `PATH`. By hand, `git-scry resolve --app` opens the app on each file, in front of the terminal, and waits for it. The app is `$SCRY_APP`, `/Applications/Scry.app` or `~/Applications/Scry.app`, unless given as `--app=/path/to/Scry.app`:

```sh
git config --global mergetool.scry.cmd 'git-scry resolve --app "$BASE" "$LOCAL" "$REMOTE" "$MERGED"'
```

The app shows the base even when the file has none, as the terminal does. Mark as Resolved writes the file and closes the window, and `git mergetool` moves on to the next file. Closing the window without writing leaves the file conflicted.

To try it on real conflicts, `scripts/make-conflicts.sh` rebuilds `conflicts/` (ignored by git) with one repository stopped on a conflict per operation: `rebase`, `merge`, `cherry-pick` and `revert`. It adds three copies of a multi-platform app (`scripts/make-atlas-conflicts.sh`): `interactive-rebase` (pick, reword, fixup, drop) and `simple-rebase`, which stop at 4 of their 7 steps, and `big-merge`, with every conflict at once; on Swift, Objective-C, Kotlin, Rust, TypeScript, TSX, CSS, Python, Go, Ruby, Bash, JSON, YAML, TOML, an Xcode project, a String Catalog, a binary file and a file deleted on one side. In each, `git mergetool` runs this checkout's `git-scry`.

## macOS app

To use the app day to day, without Xcode:

```sh
scripts/install-app.sh      # builds in Release, installs /Applications/Scry.app and links git-scry on the PATH
```

Then, in a repository with a merge, rebase, cherry-pick or revert in progress, from any folder in it, `git scry` (or `git scry open [path]`) opens the app on its overview. When Scry is git's merge tool, the script points it at the installed app; otherwise Scry › Settings › CLI › Install sets it up, and `git mergetool` then opens each file in the app. Run the script again to update.

To develop it:

```sh
scripts/build-ffi.sh        # once before opening Xcode: builds the core and the Swift bindings
open apps/macos/Scry.xcodeproj
```

Xcode runs `scripts/build-ffi.sh` before each build. On a fresh checkout without `scripts/build-ffi.sh` run first, the first build fails on missing types (`Cell`, `Row`…) because Xcode lists the sources before the script writes the bindings; build again. Pass a repository's folder or a conflicted file as the launch argument (scheme › Run › Arguments), or use Open Repository… (⌘O) or Open Conflicted File… (⇧⌘O). A repository opens on the overview of its merge, rebase, cherry-pick or revert: its conflicted files and their state, Stage resolved, Continue, Skip and Abort, and for a rebase its plan as a graph (each pick, reword, fixup or drop, where git stopped, and which steps it ran on its own), or for a merge both branches since their merge base; a file opens from there in the editor (pencil button, double-click or Return), and the files the editor cannot settle open their own view: keep or delete a file deleted on one side, take one version of a binary file (with an image preview), keep both sides' additions to an Xcode project (checked by the core), or merge a string catalog key by key and comes back once written. The view models' unit tests (`apps/macos/ScryTests`, Swift Testing) run with ⌘U, or:

```sh
xcodebuild test -project apps/macos/Scry.xcodeproj -scheme Scry -destination 'platform=macOS'
```

The CI's `app` job does the same on GitHub's `xcode-27` image, after `scripts/build-ffi.sh`, then builds the app in Release.
 Colours come from the design tokens: after a new export in `~/workspace/scry-ui`, run `scripts/generate-theme.py`.

## Layout

- `crates/scry-core`: all state and logic, shared by every front end. No platform-specific code.
- `crates/git-scry`: the `git-scry` command-line binary.
- `crates/scry-ffi`: the core exposed to Swift through UniFFI.
- `apps/macos`: the SwiftUI app (standard Xcode project).

The MCP server will be added with its first code.
