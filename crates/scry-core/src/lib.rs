//! Core of Scry.
//!
//! Every front end (the `git-scry` CLI, the MCP server, the SwiftUI app and a
//! later TUI) builds on this crate. It owns all state and logic and must stay
//! free of platform-specific code (vault decision 0003).

pub mod conflict;
pub mod git;
pub mod highlight;
pub mod layout;
pub mod operation;
pub mod resolution;
pub mod session;
pub mod syntax;
