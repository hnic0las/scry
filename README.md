<p align="center">
  <img src=".github/scry-icon.png" width="128" height="128" alt="Scry app icon">
</p>

<h1 align="center">Scry</h1>

<p align="center">
  <strong>See through your merge conflicts.</strong><br>
  A local-first merge tool for git: a native macOS app and the <code>git scry</code> command.
</p>

Scry (`git-scry`) is a local-first tool, integrated with git, for merging and reviewing changes. It resolves conflicts through `git mergetool`, reviews branches and pull requests, runs built-in checks such as secret scanning, and takes proposals from agents through MCP that a human validates.

What it does today:

- **Understands the whole operation.** A merge, rebase, cherry-pick or revert opens on its overview: the conflicted files, who is on each side, and a rebase's plan as a graph, with where git stopped.
- **Resolves conflict by conflict.** Pick ours, theirs, both or base with one key, or edit the result by hand, with undo and redo, word-level differences and syntax highlighting.
- **Checks every candidate.** In 19 languages, Scry parses each resolution and marks the ones that would break the file before you write it.
- **Handles the files text merges get wrong.** Xcode projects, String Catalogs, `Package.resolved`, lockfiles from 18 package managers, binary files and files deleted on one side each get their own view.
- **Stays in your workflow.** It plugs into `git mergetool`, works in the terminal as well as in the app, and never stages, commits or pushes on its own.

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

Check, lint and test before each commit, as the CI does on macOS, Linux and Windows. The CI runs only when a tag is pushed (`git tag v0.1.0 && git push origin v0.1.0`):

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Before committing a change to the core, also check that its tests would catch a bug in the changed lines: mutation testing changes the code (a `+` becomes a `-`, a function returns a default value) and expects a test to fail. Each mutant that survives is a missing test, or is recorded as accepted in the vault task note on test quality:

```sh
brew install cargo-mutants cargo-llvm-cov
rustup component add llvm-tools          # for cargo-llvm-cov

git diff HEAD > /tmp/scry.diff && cargo mutants --in-diff /tmp/scry.diff
```

The whole workspace takes about an hour (`cargo mutants -j 6`; one file: `-f crates/scry-core/src/conflict.rs`; only the mutants that survived last time: `--iterate`). The FFI layer is left out (`.cargo/mutants.toml`): the Swift tests cover it. The report is in `mutants.out/` (`missed.txt` lists the survivors).

Mutation testing tells whether the tests react, not whether the results are right. For that, `tests/merge_file_oracle.rs` checks Scry against `git merge-file` on generated merges: the same conflicts, and taking ours, theirs or both everywhere gives what `--ours`, `--theirs` and `--union` give. It runs 150 cases with the tests; run more before a release:

```sh
SCRY_ORACLE_CASES=5000 cargo test -p scry-core --test merge_file_oracle   # about 5 minutes
```

Coverage shows the code no test runs:

```sh
cargo llvm-cov --workspace --html        # target/llvm-cov/html/index.html
cargo llvm-cov --workspace --summary-only
```

For the app, the Scry scheme collects coverage of the app target: in Xcode, Report navigator › the test run › Coverage; from the command line, `xcrun xccov view --report` on the `.xcresult` that `xcodebuild test` writes.

Try the CLI:

```sh
cargo run -p git-scry -- conflicts path/to/file
cargo run -p git-scry -- session    # the operation in progress and who is on each side
cargo run -p git-scry -- highlight path/to/file   # the syntax colours the app shows (--spans to list them)
```

Use it as git's merge tool, in the terminal:

```sh
git config --global merge.tool scry
git config --global mergetool.scry.cmd 'git-scry resolve -- "$BASE" "$LOCAL" "$REMOTE" "$MERGED"'
git config --global mergetool.scry.trustExitCode true
```

`git mergetool` then asks for a candidate per conflict (the same digits as the app's shortcuts: 1 ours, 2 theirs, 3 and 4 both, 5 and 6 both with each side closed, 0 base, 9 none) and writes the file only when every conflict is resolved. For 19 languages (Swift, Objective-C, C, C++, Rust, JavaScript, TypeScript, TSX, HTML, CSS, Python, Kotlin, Java, Go, Ruby, Bash, JSON including `.xcstrings`, YAML, TOML), each candidate is checked by parsing, when both sides' versions of the file parse (✗ when the file would not parse), and a file that does not parse is written only after confirmation, or with `--allow-invalid`.

Or in the Scry app: Scry › Settings › Use Scry as git's Merge Tool sets all three for you, running the `git-scry` inside the app (`Scry.app/Contents/Helpers/git-scry`, put there by the Xcode build), so nothing has to be on the `PATH`. By hand, `git-scry resolve --app` opens the app on each file, in front of the terminal, and waits for it. The app is `$SCRY_APP`, `/Applications/Scry.app` or `~/Applications/Scry.app`, unless given as `--app=/path/to/Scry.app`:

```sh
git config --global mergetool.scry.cmd 'git-scry resolve --app -- "$BASE" "$LOCAL" "$REMOTE" "$MERGED"'
```

The app shows the base even when the file has none, as the terminal does. Mark as Resolved writes the file and closes the window, and `git mergetool` moves on to the next file. Closing the window without writing leaves the file conflicted.

To try it on real conflicts, `scripts/make-conflicts.sh` rebuilds `conflicts/` (ignored by git) with one repository stopped on a conflict per operation: `rebase`, `merge`, `cherry-pick` and `revert`. It adds three copies of a multi-platform app (`scripts/make-atlas-conflicts.sh`): `interactive-rebase` (pick, reword, fixup, drop) and `simple-rebase`, which stop at 4 of their 7 steps, and `big-merge`, with every conflict at once; on Swift, Objective-C, Kotlin, Rust, TypeScript, TSX, CSS, Python, Go, Ruby, Bash, JSON, YAML, TOML, an Xcode project, a String Catalog, a binary file and a file deleted on one side. In each, `git mergetool` runs this checkout's `git-scry`.

## macOS app

To use the app day to day, without Xcode:

```sh
scripts/install-app.sh      # builds in Release, installs /Applications/Scry.app and links git-scry on the PATH
```

Then, in a repository with a merge, rebase, cherry-pick or revert in progress, from any folder in it, `git scry` (or `git scry open [path]`) opens the app on its overview. When Scry is git's merge tool, the script points it at the installed app; otherwise Scry › Settings › Use Scry as git's Merge Tool sets it up, and `git mergetool` then opens each file in the app. Run the script again to update.

To develop it:

```sh
scripts/build-ffi.sh        # once before opening Xcode: builds the core and the Swift bindings
open apps/macos/Scry.xcodeproj
```

Xcode runs `scripts/build-ffi.sh` before each build. On a fresh checkout without `scripts/build-ffi.sh` run first, the first build fails on missing types (`Cell`, `Row`…) because Xcode lists the sources before the script writes the bindings; build again. Pass a repository's folder or a conflicted file as the launch argument (scheme › Run › Arguments), or use Open Repository… (⌘O) or Open Conflicted File… (⇧⌘O). A repository opens on the overview of its merge, rebase, cherry-pick or revert: its conflicted files and their state, Stage and Unstage per file (Unstage gives git the conflict back and keeps the file as resolved), Stage resolved, Continue, Skip and Abort, and for a rebase its plan as a graph (each pick, reword, fixup or drop, where git stopped, and which steps it ran on its own); a `reword` step stops for the commit's message, written in the overview before Continue (git never opens an editor of its own), or for a merge both branches since their merge base; a file opens from there in the editor (pencil button, double-click or Return), and the files the editor cannot settle open their own view: keep or delete a file deleted on one side, take one version of a binary file (with an image preview), keep both sides' additions to an Xcode project (checked by the core), merge a string catalog key by key or a `Package.resolved` package by package (each pin whole), or take one side of a lockfile (`package-lock.json`, `yarn.lock`, `pnpm-lock.yaml`, `bun.lock`, `deno.lock`, `Cargo.lock`, `Podfile.lock`, `Cartfile.resolved`, `Gemfile.lock`, `poetry.lock`, `uv.lock`, `Pipfile.lock`, `composer.lock`, `pubspec.lock`, `gradle.lockfile`, `flake.lock`, `mix.lock`, `go.sum`), unstaged, with the packages each side changed and the command that writes it again (`go.sum` can also keep both sides' lines), and comes back once written. The view models' unit tests (`apps/macos/ScryTests`, Swift Testing) run with ⌘U, or:

```sh
xcodebuild test -project apps/macos/Scry.xcodeproj -scheme Scry -destination 'platform=macOS'
```

The CI's `app` job does the same on GitHub's `xcode-27` image, after `scripts/build-ffi.sh`, then builds the app in Release.
 Keyboard shortcuts are listed once, in `apps/macos/Scry/Shortcuts/Shortcut.swift`: menus, buttons, tooltips and Help › Keyboard Shortcuts (⌘/) read that list. Colours come from the design tokens: after a new export in `~/workspace/scry-ui`, run `scripts/generate-theme.py`.

## Layout

- `crates/scry-core`: all state and logic, shared by every front end. No platform-specific code.
- `crates/git-scry`: the `git-scry` command-line binary.
- `crates/scry-ffi`: the core exposed to Swift through UniFFI.
- `apps/macos`: the SwiftUI app (standard Xcode project).

The MCP server will be added with its first code.
