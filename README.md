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

Use it as git's merge tool (terminal only until the app exists):

```sh
git config --global merge.tool scry
git config --global mergetool.scry.cmd 'git-scry resolve "$BASE" "$LOCAL" "$REMOTE" "$MERGED"'
git config --global mergetool.scry.trustExitCode true
```

`git mergetool` then asks for a candidate per conflict (the same digits as the app's shortcuts: 1 ours, 2 theirs, 3 and 4 both, 5 and 6 both with each side closed, 0 base, 9 none) and writes the file only when every conflict is resolved. For 19 languages (Swift, Objective-C, C, C++, Rust, JavaScript, TypeScript, TSX, HTML, CSS, Python, Kotlin, Java, Go, Ruby, Bash, JSON including `.xcstrings`, YAML, TOML), each candidate is checked by parsing, when both sides' versions of the file parse (✗ when the file would not parse), and a file that does not parse is written only after confirmation, or with `--allow-invalid`.

To try it on real conflicts, `scripts/make-conflicts.sh` rebuilds `conflicts/` (ignored by git) with one repository stopped on a conflict per operation: `rebase`, `merge`, `cherry-pick` and `revert`. In each, `git mergetool` runs this checkout's `git-scry`.

## macOS app

```sh
scripts/build-ffi.sh        # once before opening Xcode: builds the core and the Swift bindings
open apps/macos/Scry.xcodeproj
```

Xcode runs `scripts/build-ffi.sh` before each build. On a fresh checkout without `scripts/build-ffi.sh` run first, the first build fails on missing types (`Cell`, `Row`…) because Xcode lists the sources before the script writes the bindings; build again. Pass a conflicted file as the launch argument (scheme › Run › Arguments), or use Open Conflicted File… (⌘O). Colours come from the design tokens: after a new export in `~/workspace/scry-ui`, run `scripts/generate-theme.py`.

## Layout

- `crates/scry-core`: all state and logic, shared by every front end. No platform-specific code.
- `crates/git-scry`: the `git-scry` command-line binary.
- `crates/scry-ffi`: the core exposed to Swift through UniFFI.
- `apps/macos`: the SwiftUI app (standard Xcode project).

The MCP server will be added with its first code.
