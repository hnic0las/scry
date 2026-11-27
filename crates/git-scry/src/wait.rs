//! `git scry wait`: prints the asks the user makes in Scry for an agent
//! (Explain on a conflict, or on lines selected in the Changes window,
//! with or without an operation in progress), so the agent answers them through the MCP
//! server. Scry's Claude Code plugin runs `wait --follow` as a monitor:
//! each line it prints reaches the user's session.

use std::path::PathBuf;
use std::process::ExitCode;
use std::thread::sleep;
use std::time::{Duration, Instant};

use scry_core::asks::{self, Watch};
use scry_core::git::GitCli;

/// How often the asks files are looked at: an ask is printed within this
/// time of being made.
const POLL: Duration = Duration::from_millis(500);

pub fn run(follow: bool) -> ExitCode {
    let git = GitCli::discover(PathBuf::from("."));
    let git_dir = match asks::git_dir(&git) {
        Ok(dir) => dir,
        // The plugin's monitor starts in every folder Claude Code opens:
        // outside a repository there is nothing to watch, and nothing to
        // say, so it stays quiet until the session ends.
        Err(_) if follow => loop {
            sleep(Duration::from_secs(3600));
        },
        Err(error) => {
            eprintln!("git-scry: {error}");
            return ExitCode::from(2);
        }
    };
    // Held while the watch runs, the system releasing its lock when the
    // process ends, even killed with its session: Scry then sees nobody
    // watching at once. Without it Scry says nobody watches, and the
    // asks are printed all the same.
    let presence = asks::Presence::start(&git_dir, &std::process::id().to_string()).ok();
    let mut watch = Watch::new(&git_dir);
    let mut last_beat = Instant::now();
    loop {
        if last_beat.elapsed() >= asks::BEAT {
            if let Some(presence) = &presence {
                let _ = presence.beat();
            }
            last_beat = Instant::now();
        }
        // git failing for a moment (a lock, the operation ending) is not
        // the end of the watch: the next poll reads the stop again.
        let new = watch.poll(&git).unwrap_or_default();
        for open in &new {
            println!("{}", asks::line(open));
        }
        if !follow && !new.is_empty() {
            drop(presence);
            return ExitCode::SUCCESS;
        }
        sleep(POLL);
    }
}
