# Scry (git-scry)

## Project knowledge

Specifications, decisions and investigations live in the Bigsool Obsidian vault, not in this repository: `~/Documents/Bigsool/Projects/git-scry/Home.md`. Read it before starting work and follow its links.

Accepted decisions so far:

- Start with the merge tool (vault decision 0001).
- Agent routes (vault decision 0002): an MCP server from the start; the user's Claude API key by default for the app's own model calls; a hand-off to the user's agent when there is no key; a provider interface in the core. The app never launches the `claude` CLI and never uses claude.ai credentials.

Proposed but not decided: a Rust core with a SwiftUI app through UniFFI. Do not scaffold a language or framework before it is decided.

## Design rules

- Deterministic first: code computes every fact; a model only handles what needs judgement.
- Agent output is a proposal with its provenance, verified by code and accepted by a human. No code path stages, commits, pushes or posts on behalf of an agent.

## Naming

- Technical name `git-scry` (binary, crate, Homebrew formula), so that `git scry` works.
- Brand "Scry", or "Scry for Git". Never "GitScry" or another word built on "Git" (Git trademark policy).
