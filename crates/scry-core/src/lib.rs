//! Core of Scry.
//!
//! Every front end (the `git-scry` CLI, the MCP server, the SwiftUI app and a
//! later TUI) builds on this crate. It owns all state and logic and must stay
//! free of platform-specific code (vault decision 0003).

pub mod blame;
pub mod catalog;
pub mod command;
pub mod commit_changes;
pub mod commit_message;
pub mod conflict;
pub mod git;
pub mod highlight;
pub mod indent;
pub mod layout;
pub mod lockfile;
pub mod merge_graph;
pub mod mergetool;
pub mod operation;
pub mod overview;
pub mod pbxproj;
pub mod presentation;
pub mod rebase_plan;
pub mod resolution;
pub mod resolved;
pub mod session;
pub mod syntax;
pub mod theme;
pub mod words;
