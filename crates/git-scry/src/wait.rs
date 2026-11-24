//! `git scry wait`: prints the asks the user makes in Scry for an agent
//! (Explain on a conflict), so the agent answers them through the MCP
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
    let id = std::process::id().to_string();
    let mut watch = Watch::new(&git_dir);
    let mut last_beat: Option<Instant> = None;
    loop {
        if last_beat.is_none_or(|at| at.elapsed() >= asks::BEAT) {
            let _ = asks::beat(&git_dir, &id);
            last_beat = Some(Instant::now());
        }
        // git failing for a moment (a lock, the operation ending) is not
        // the end of the watch: the next poll reads the stop again.
        let new = watch.poll(&git).unwrap_or_default();
        for open in &new {
            println!("{}", asks::line(open));
        }
        if !follow && !new.is_empty() {
            asks::leave(&git_dir, &id);
            return ExitCode::SUCCESS;
        }
        sleep(POLL);
    }
}
