//! Scry's MCP server, run as `git-scry mcp` by the user's agent (Claude
//! Code, through the `git-scry` plugin): the operation in progress and
//! its conflicts as Scry computes them, so the agent reads facts instead of
//! whole files and `git log`.
//!
//! Over standard input and output only: no port, no socket. The server and
//! the app share no connection; the repository is their only channel.

mod protocol;
mod tools;

use std::io::{self, BufRead, Write};

use scry_core::git::GitCli;

pub use protocol::Server;

/// Answers the messages read from `input` on `output`, one per line, until
/// `input` ends: the agent closed the connection.
pub fn serve(input: impl BufRead, mut output: impl Write, git: GitCli) -> io::Result<()> {
    let mut server = Server::new(git);
    for line in input.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        if let Some(answer) = server.answer(&line) {
            // JSON escapes line breaks inside strings: one message, one line.
            writeln!(output, "{answer}")?;
            output.flush()?;
        }
    }
    Ok(())
}
