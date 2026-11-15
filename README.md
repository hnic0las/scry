<p align="center">
  <img src=".github/scry-icon.png" width="128" height="128" alt="Scry app icon">
</p>

<h1 align="center">Scry</h1>

<p align="center">
  <strong>See through your merge conflicts.</strong><br>
  A local-first merge tool for git: a native macOS app and the <code>git scry</code> command.
</p>

Scry (`git-scry`) is a local-first tool, integrated with git, for merging and reviewing changes. It resolves conflicts through `git mergetool`, reviews branches and pull requests, runs built-in checks such as secret scanning, and takes proposals from agents through MCP that a human validates.

Status: the merge tool (the macOS app and the `git-scry` CLI) is close to its first release. MCP, agents, the diff tool and review come next (see [Roadmap](#roadmap)).

## Features

### The whole operation at a glance

- A merge, rebase, cherry-pick or revert opens on its **overview**: the conflicted files and their state, and who is on each side.
- A rebase shows its **plan as a graph**: each pick, reword, fixup or drop, where git stopped, and which steps it ran on its own. A merge shows both branches since their merge base.
- **Drive the operation from the app**: Stage and Unstage per file (Unstage hands the conflict back to git and keeps your resolution), Stage resolved, Continue, Skip and Abort.
- A `reword` step asks for the commit message in the overview, before Continue: git never opens an editor of its own.

### Conflict by conflict

- **One key per choice**: ours, theirs, both in either order, base or none, from the keyboard, the lens row's menu, the › ‹ buttons beside each conflict or the inspector.
- **Edit the result by hand**, conflict by conflict, with undo and redo throughout.
- **Word-level differences** and **syntax highlighting** in the three panes.
- **Show the base** (⌥⌘B) under each conflict, to see what both sides changed from.
- **Two layouts**: three columns, or both sides above the result (⌥⌘L). The second is chosen automatically in a narrow window.
- **Overview ruler**: one mark per conflict, coloured by its state; click one to go to it.
- Synchronised scrolling, horizontal included, and every shortcut listed in Help › Keyboard Shortcuts (⌘/).

### Checked before it is written

- In **19 languages** (Swift, Objective-C, C, C++, Rust, JavaScript, TypeScript, TSX, HTML, CSS, Python, Kotlin, Java, Go, Ruby, Bash, JSON including `.xcstrings`, YAML, TOML), Scry parses each candidate and marks the ones that would break the file, at the line of the error.
- A file that would not parse is written only after you confirm it.
- Merging an **Xcode project**, Scry checks that every reference still points to an object.

### Files a text merge gets wrong

Each of these gets its own view:

- a file **deleted on one side**: keep it or delete it;
- a **binary file**: take one version, with an image preview;
- an **Xcode project** (`.pbxproj`): keep both sides' additions;
- a **String Catalog**: merge key by key;
- **`Package.resolved`**: merge package by package, each pin whole;
- **lockfiles from 18 package managers** (`package-lock.json`, `yarn.lock`, `pnpm-lock.yaml`, `bun.lock`, `deno.lock`, `Cargo.lock`, `Podfile.lock`, `Cartfile.resolved`, `Gemfile.lock`, `poetry.lock`, `uv.lock`, `Pipfile.lock`, `composer.lock`, `pubspec.lock`, `gradle.lockfile`, `flake.lock`, `mix.lock`, `go.sum`): take one side, see the packages each side changed and the command that writes the file again. `go.sum` can also keep both sides' lines.

### In your workflow

- **`git mergetool`**, in the app or in the terminal with the same digits as the app's shortcuts. Scry › Settings sets git up for you.
- **`git scry`** opens the app on the operation in progress, from any folder of the repository.
- `git scry session`, `conflicts` and `highlight` print what the app shows, for scripts and terminals.
- Scry never stages, commits or pushes on its own: each of those steps is a button you press.
- Local first: no account and no server; nothing leaves your Mac.

### Tested against git

Scry's merges are checked against `git merge-file` on generated conflicts, line endings and marker sizes included. Mutation testing checks that the tests catch changes in the code. The hand-written parsers are fuzzed. See [Development](#development).

## Roadmap

- **An MCP server** (`git scry mcp`), so your agent, for example Claude Code, can read the conflicts and propose resolutions that appear live in the app.
- **Explain**, **Suggest** and **Ask** on a conflict, with your own Claude API key, or handed to your agent when there is none. Every proposal keeps its source, is checked by code and is accepted by you; no agent stages, commits, pushes or posts.
- **More files merged well**: storyboards and `.xib` files; migrations both sides added (Django, Alembic, Rails `schema.rb`), which leave two heads without any text conflict; Jupyter notebooks, `.strings` in UTF-16, Android `strings.xml`, `.po`, `.resx`; `CHANGELOG.md`, where both sides add at the top; running `npm install` or `yarn install` directly to repair their lockfiles.
- **A diff tool**: `git difftool` and `git scry diff [ref..ref]`, with a side-by-side view and the list of changed files.
- **Review of branches and pull requests**, with built-in checks such as secret scanning.
- **Editing the whole file**, not only conflicts.
- **Other front ends**: the core holds all the logic and is built and tested on macOS, Linux and Windows, so another app could use it.

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

git diff HEAD > /tmp/scry.diff && scripts/mutants.sh --in-diff /tmp/scry.diff
```

Always run it through `scripts/mutants.sh`, which passes its arguments to `cargo mutants`: it closes standard input, and kills any test process over 2 GB (`SCRY_MUTANTS_MAX_MB`), since a mutant can turn a loop into one that fills memory; several at once crashed the Mac once. The whole workspace takes about two hours (`scripts/mutants.sh`; one file: `-f crates/scry-core/src/conflict.rs`; only the mutants that survived last time: `--iterate`; 4 at a time, `SCRY_MUTANTS_JOBS` to change it). The FFI layer is left out (`.cargo/mutants.toml`): the Swift tests cover it. The report is in `mutants.out/` (`missed.txt` lists the survivors).

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

Fuzzing feeds the hand-written parsers generated input, to find panics and endless loops on files nobody writes by hand. It needs the nightly toolchain and cargo-fuzz (`rustup toolchain install nightly`, `cargo install cargo-fuzz`); the targets are in `fuzz/` (conflict markers, a merge session, Xcode projects, String Catalogs, `Package.resolved`, lockfiles), outside the workspace:

```sh
scripts/fuzz.sh            # every target, 30 minutes each (3 hours)
scripts/fuzz.sh 60 pbxproj # one target, one minute
```

When to run what:

| When | What |
|---|---|
| Each commit | `fmt`, `clippy`, `test` (with the oracle's 150 cases); `scripts/mutants.sh --in-diff` on the diff when Rust code changed |
| Each push to `main` | `scripts/mutants.sh --in-diff` on `git diff origin/main` if a commit skipped it; the app's tests (`xcodebuild … test`) when `apps/macos/` or `crates/scry-ffi/` changed |
| Each tag | `SCRY_ORACLE_CASES=1000` on the oracle, then the CI on macOS, Linux and Windows, all jobs green |
| Each release | `scripts/mutants.sh` on the whole workspace, Rust and app coverage, `SCRY_ORACLE_CASES=5000`, `scripts/fuzz.sh` (30 minutes per parser), and a manual pass through the app and the CLI's prompts |

A bug fix starts with a test that fails without it. A surviving mutant gets a test, or is accepted with its reason when it is equivalent (the same result) or out of reach (it needs the app, a terminal or another OS).

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
