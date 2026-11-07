# Scry

Scry (`git-scry`) is a local-first tool, integrated with git, for merging and reviewing changes. It resolves conflicts through `git mergetool`, reviews branches and pull requests, runs built-in checks such as secret scanning, and takes proposals from agents through MCP that a human validates.

Status: design phase. No code yet.

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
```

To try it on real conflicts, `scripts/make-conflicts.sh` rebuilds `conflicts/` (ignored by git) with one repository stopped on a conflict per operation: `rebase`, `merge`, `cherry-pick` and `revert`.

## Layout

- `crates/scry-core`: all state and logic, shared by every front end. No platform-specific code.
- `crates/git-scry`: the `git-scry` command-line binary.

The MCP server, the UniFFI bindings and the SwiftUI app (`apps/macos`) will be added with their first code.
