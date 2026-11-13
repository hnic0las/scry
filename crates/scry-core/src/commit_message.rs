//! The rules of a commit message the overview checks while the user
//! writes a reworded commit's message.

use serde::Serialize;

/// The subject's length that `git log`, GitHub and the rebase plan show
/// whole.
pub const SUBJECT_LIMIT: usize = 72;

/// What is wrong, or not, with a message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MessageCheck {
    /// Nothing but blanks: git refuses it.
    pub empty: bool,
    /// The first line's length, in characters.
    pub subject_length: usize,
    pub subject_too_long: bool,
    /// Text on line 2: git takes everything before the first blank line
    /// as the subject.
    pub second_line_is_text: bool,
}

pub fn check(message: &str) -> MessageCheck {
    let mut lines = message.split('\n');
    let subject_length = lines.next().unwrap_or_default().chars().count();
    MessageCheck {
        empty: message.trim().is_empty(),
        subject_length,
        subject_too_long: subject_length > SUBJECT_LIMIT,
        second_line_is_text: lines.next().is_some_and(|line| !line.trim().is_empty()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The overview warns about what git and forges do to a message: an
    /// empty one is refused, a long subject is cut, and text on line 2
    /// joins the subject.
    #[test]
    fn a_message_is_checked_as_git_reads_it() {
        assert!(check(" \n\t\n").empty);
        let long = "x".repeat(73);
        let checked = check(&long);
        assert_eq!(checked.subject_length, 73);
        assert!(checked.subject_too_long);
        assert!(!check(&"é".repeat(72)).subject_too_long);
        assert!(check("Subject\nmore subject").second_line_is_text);
        assert!(!check("Subject\n\nBody").second_line_is_text);
        assert!(!check("Subject").second_line_is_text);
    }
}
