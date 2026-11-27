//! `git-scry log <path>`: opens the app's Changes window on one file's
//! history, or with `--print` lists its points in the terminal: what is
//! not committed, then the commits that touched the file, followed
//! through its renames.

use std::ffi::OsString;
use std::path::PathBuf;
use std::process::ExitCode;

use scry_core::git::{Git, GitCli};
use scry_core::history::{self, Point, PointKind};

pub struct Options {
    pub path: String,
    pub print: bool,
    pub app: Option<PathBuf>,
}

/// The app's option for a file's history: the repository's root follows
/// it, then the file from the root after `--path`.
pub const HISTORY: &str = "--history";

pub fn run(options: Options) -> ExitCode {
    match read(&options) {
        Ok(read) if options.print => {
            print(&read);
            ExitCode::SUCCESS
        }
        Ok(read) => {
            let arguments: Vec<OsString> = vec![
                HISTORY.into(),
                read.root.clone().into_os_string(),
                "--path".into(),
                read.path.into(),
            ];
            crate::open::open_app(options.app, &read.root, arguments)
        }
        Err(message) => {
            eprintln!("git-scry: {message}");
            ExitCode::from(2)
        }
    }
}

/// The file's history, read before the app is opened: a file with none
/// is reported in the terminal.
struct Read {
    root: PathBuf,
    /// The file, from the repository's root.
    path: String,
    points: Vec<Point>,
}

fn read(options: &Options) -> Result<Read, String> {
    let here = GitCli::discover(PathBuf::from("."));
    let text = |error: scry_core::git::GitError| error.to_string();
    let root = PathBuf::from(
        here.run(&["rev-parse", "--show-toplevel"])
            .map_err(text)?
            .trim_end_matches(['\n', '\r']),
    );
    // The path is given from the current folder, and git lists files
    // from the root.
    let prefix = here.run(&["rev-parse", "--show-prefix"]).map_err(text)?;
    let path = format!("{}{}", prefix.trim_end_matches(['\n', '\r']), options.path);
    let git = here.in_dir(root.clone());
    let mut points = history::uncommitted(&git, &root, &path).map_err(text)?;
    let followed = history::committed_path(&points, &path);
    // The app reads its own pages: the first one tells whether there is
    // a history to open.
    loop {
        let commits = points
            .iter()
            .filter(|point| point.kind == PointKind::Commit)
            .count();
        let page = history::commits(&git, None, &followed, commits, history::PAGE).map_err(text)?;
        points.extend(page.points);
        if !page.more || !options.print {
            break;
        }
    }
    if points.is_empty() {
        return Err(format!(
            "{path} has no history: no commit touched it and nothing of it is not committed"
        ));
    }
    Ok(Read { root, path, points })
}

fn print(read: &Read) {
    let commits = read
        .points
        .iter()
        .filter(|point| point.kind == PointKind::Commit)
        .count();
    let noun = if commits == 1 { "commit" } else { "commits" };
    println!(
        "History of {} · {commits} {noun} · renames followed",
        read.path
    );
    for point in &read.points {
        let counts = match (point.added, point.removed) {
            (Some(added), Some(removed)) => format!("+{added} −{removed}"),
            _ => String::from("not counted"),
        };
        match point.kind {
            PointKind::NotCommitted => println!("  Not committed  {counts}"),
            PointKind::Staged => println!("  Staged  {counts}"),
            PointKind::Commit => {
                let head = if point.head { "  HEAD" } else { "" };
                println!(
                    "  {}  {}  {}  {}  {counts}{head}",
                    point.short_id(),
                    date(point.time),
                    point.author,
                    point.subject
                );
            }
        }
        if let Some(old_path) = &point.old_path {
            println!("    renamed from {old_path}");
        }
    }
}

/// The day of `seconds` since 1970, in UTC: "2026-11-27".
fn date(seconds: i64) -> String {
    // Howard Hinnant's days-to-civil.
    let days = seconds.div_euclid(86_400) + 719_468;
    let era = days.div_euclid(146_097);
    let day_of_era = days.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * shifted_month + 2) / 5 + 1;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

#[cfg(test)]
mod tests {
    use super::date;

    #[test]
    fn a_time_is_printed_as_its_day() {
        assert_eq!(date(0), "1970-01-01");
        assert_eq!(date(951_782_400), "2000-02-29");
        assert_eq!(date(1_795_816_800), "2026-11-27");
        assert_eq!(date(1_704_067_199), "2023-12-31");
    }
}
