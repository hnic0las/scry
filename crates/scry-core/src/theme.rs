//! The theme as roles (portability rule 5): the core says which role each
//! thing plays, and each UI maps a role to its own colour and symbol. The
//! syntax roles (`highlight::SyntaxRole`) and the cell kinds
//! (`layout::CellKind`) are roles too; these are the rest of the design's
//! colour tokens that the core assigns.

/// Where a resolution comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provenance {
    /// A side as git produced it.
    Git,
    /// Made by a format rule.
    Rule,
    /// Made by a structural merge.
    Structural,
    /// Made by a code analyzer.
    Analyzer,
    /// Proposed by an agent and accepted by the user.
    Agent,
    /// Written by the user.
    Human,
}

/// Where a conflict or a file stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Unresolved,
    /// Resolved by something other than the user, to look at before
    /// writing.
    NeedsReview,
    Resolved,
    /// Written and staged in git.
    Staged,
}

/// How serious a check's finding is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
    Info,
}

/// The role that colours a badge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Provenance(Provenance),
    State(State),
    Severity(Severity),
}

/// A short label with its role, such as a lens row's "Theirs" in git's
/// colour or "Does not parse" as an error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Badge {
    pub text: String,
    pub role: Role,
}

impl Badge {
    pub fn new(text: impl Into<String>, role: Role) -> Badge {
        Badge {
            text: text.into(),
            role,
        }
    }
}
