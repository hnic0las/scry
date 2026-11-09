# Scry (git-scry)

## Project knowledge

Specifications, decisions and investigations live in the Workshop Obsidian vault, not in this repository: `~/Documents/Workshop/Projects/git-scry/Home.md`. Read it before starting work and follow its links.

Accepted decisions so far:

- Start with the merge tool (vault decision 0001).
- Agent routes (vault decision 0002): an MCP server from the start; the user's Claude API key by default for the app's own model calls; a hand-off to the user's agent when there is no key; a provider interface in the core. The app never launches the `claude` CLI and never uses claude.ai credentials.
- Rust core and SwiftUI Mac app through UniFFI (vault decision 0003). Syntax highlighting is computed in the core with tree-sitter.

Layout and commands: `README.md`. Add `crates/scry-mcp`, `crates/scry-ffi` and `apps/macos` with their first code, not as empty placeholders.

## Working mode

- Claude writes both the Rust core and the SwiftUI app; Nicolas validates and orchestrates. Nicolas is new to Rust and an experienced SwiftUI developer.
- Write plain Rust: no complex lifetime annotations, clever generics, custom macros, or `async` in the core until needed.
- After each Rust change, explain briefly in the conversation what is new language-wise; code comments explain why, not Rust basics.
- Work in small increments that can be shown, each with what to run or look at to validate it.

## Design prototype

The UI design lives in its own repository, `~/workspace/scry-ui` (images in Git LFS), so this one keeps only code. Its `prototype/` folder is exported by Claude Design; never edit it. Start from `prototype/README.md` and `prototype/SPEC.md`. Raise design problems with Nicolas, who passes them to Claude Design; each new export lands as a commit there, and the app follows its diff.

## Portability rules (decision 0003)

1. The core owns all state and logic (merge session, undo and redo, line alignment, conflict blocks, word-level diff, highlight spans, folding, provenance). The SwiftUI app renders and forwards input only.
2. The core talks to UIs through serialisable commands and events, so another UI can use the same contract.
3. No macOS-specific code in the core except behind traits (secrets, file watching, spawning `git`, line endings, file-system case, paths).
4. CI runs on macOS, Linux and Windows from the first code commit.
5. The theme is defined in the core as roles; each UI maps roles to colours.

## Design rules

- Deterministic first: code computes every fact; a model only handles what needs judgement.
- Agent output is a proposal with its provenance, verified by code and accepted by a human. No code path stages, commits, pushes or posts on behalf of an agent.

## Naming

- Technical name `git-scry` (binary, crate, Homebrew formula), so that `git scry` works.
- Brand "Scry", or "Scry for Git". Never "GitScry" or another word built on "Git" (Git trademark policy).
