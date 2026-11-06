//! Core of Scry.
//!
//! Every front end (the `git-scry` CLI, the MCP server, the SwiftUI app and a
//! later TUI) builds on this crate. It owns all state and logic and must stay
//! free of platform-specific code (vault decision 0003).

pub mod conflict;
