//! The contract between the core and every UI (portability rule 2).
//!
//! A UI changes a merge only by sending a `Command` to
//! `MergeSession::apply`, and learns what changed from the `Event`s it
//! returns. Both are plain data that serialise to JSON, so the CLI, the
//! Mac app and the MCP server share one contract, and a new action is a
//! new variant rather than a new method in every binding. Reading the
//! merge (rows, conflicts, colours and indentation while typing) stays a
//! query: it changes nothing.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::records::Record;
use crate::resolution::CandidateKind;
use crate::session::{ChooseError, SaveError};

/// Something a UI asks the session to do. Conflicts are counted from 0.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "kebab-case")]
pub enum Command {
    /// Resolves a conflict with one of the candidates it offers.
    Choose {
        conflict: usize,
        candidate: CandidateKind,
    },
    /// Resolves a conflict with text written by hand, checked like a
    /// candidate.
    Edit {
        conflict: usize,
        text: String,
    },
    /// Resolves a conflict with what an agent proposed when the user asked
    /// in Scry, which the user accepts: kept with who proposed it and why.
    AcceptProposal {
        conflict: usize,
        proposal: Record,
    },
    /// Chooses the rule's suggestion for every open conflict that has one,
    /// as one step of the history.
    ApplySuggestions,
    /// Marks a conflict resolved outside Scry as reviewed: the user looked
    /// at what the file holds and keeps it.
    MarkReviewed {
        conflict: usize,
    },
    /// Takes a conflict's choice away: it is open again.
    Reopen {
        conflict: usize,
    },
    Undo,
    Redo,
    /// Writes the merged file where it was read from. Refuses a file
    /// changed on disk since then, unless `overwrite`.
    Save {
        overwrite: bool,
    },
}

/// What a command changed. A command that changes nothing returns no
/// event: choosing the candidate already chosen, undoing with nothing to
/// undo.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "kebab-case")]
pub enum Event {
    /// These conflicts are resolved differently, or no longer resolved:
    /// the rows, the conflicts and the merged file's check changed.
    ChoicesChanged { conflicts: Vec<usize> },
    /// The step this command made was undone.
    Undone { command: Command },
    /// The step this command made was done again.
    Redone { command: Command },
    /// What undo and redo can do now.
    HistoryChanged { can_undo: bool, can_redo: bool },
    /// The merged file was written.
    Saved,
}

/// Why a command was refused; the session is left as it was.
#[derive(Debug)]
pub enum CommandError {
    Choose(ChooseError),
    Save(SaveError),
}

impl From<ChooseError> for CommandError {
    fn from(error: ChooseError) -> CommandError {
        CommandError::Choose(error)
    }
}

impl From<SaveError> for CommandError {
    fn from(error: SaveError) -> CommandError {
        CommandError::Save(error)
    }
}

impl fmt::Display for CommandError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CommandError::Choose(error) => error.fmt(f),
            CommandError::Save(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for CommandError {}

#[cfg(test)]
mod tests {
    use super::*;

    /// A refused command reads as the reason itself, which the CLI and
    /// the app show as they are.
    #[test]
    fn errors_read_as_their_reason() {
        let choose = CommandError::from(ChooseError::NoSuchConflict(4));
        assert_eq!(choose.to_string(), "no conflict 4");
        let save = CommandError::from(SaveError::Unresolved(2));
        assert_eq!(save.to_string(), "2 conflict(s) left");
    }

    /// Commands and events are the contract another UI or the MCP server
    /// speaks: they must read and write as stable JSON, with candidates
    /// named by the same IDs as the CLI.
    #[test]
    fn commands_and_events_round_trip_through_json() {
        let command = Command::Choose {
            conflict: 2,
            candidate: CandidateKind::TheirsThenOurs,
        };
        let json = serde_json::to_string(&command).unwrap();
        assert_eq!(
            json,
            r#"{"command":"choose","conflict":2,"candidate":"theirs-then-ours"}"#
        );
        assert_eq!(serde_json::from_str::<Command>(&json).unwrap(), command);

        let event = Event::Undone { command };
        let json = serde_json::to_string(&event).unwrap();
        assert_eq!(serde_json::from_str::<Event>(&json).unwrap(), event);

        let unknown = r#"{"command":"choose","conflict":0,"candidate":"mine"}"#;
        assert!(serde_json::from_str::<Command>(unknown).is_err());
    }
}
