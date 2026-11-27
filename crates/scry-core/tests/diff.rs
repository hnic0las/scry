//! Reading a change with the `git` executable, on real repositories in
//! temporary directories: which files differ, in which group, and what
//! each file's rows are.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

use scry_core::diff::{
    self, ChangedFile, Comparison, FileDiff, FileState, Group, LineKind, Marks, RowKind, TextDiff,
};
use scry_core::git::GitCli;
use tempfile::TempDir;

/// A throwaway repository whose git commands ignore the user's own
/// configuration (signing, hooks, default branch).
struct Repo {
    root: TempDir,
}

impl Repo {
    fn new() -> Repo {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("gitconfig"), "").unwrap();
        fs::create_dir(root.path().join("repo")).unwrap();
        let repo = Repo { root };
        repo.git(&["init", "--quiet", "--initial-branch=main"]);
        repo
    }

    fn path(&self) -> PathBuf {
        self.root.path().join("repo")
    }

    /// Runs git, which must succeed.
    fn git(&self, args: &[&str]) {
        assert!(self.try_git(args), "git {args:?}");
    }

    fn try_git(&self, args: &[&str]) -> bool {
        Command::new("git")
            .arg("-C")
            .arg(self.path())
            .args(args)
            .env("GIT_CONFIG_GLOBAL", self.root.path().join("gitconfig"))
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_AUTHOR_NAME", "Scry Test")
            .env("GIT_AUTHOR_EMAIL", "test@example.com")
            .env("GIT_COMMITTER_NAME", "Scry Test")
            .env("GIT_COMMITTER_EMAIL", "test@example.com")
            .env("GIT_EDITOR", "true")
            .output()
            .unwrap()
            .status
            .success()
    }

    fn write(&self, path: &str, content: impl AsRef<[u8]>) {
        let path = self.path().join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }

    /// Writes the file and stages it.
    fn stage(&self, path: &str, content: impl AsRef<[u8]>) {
        self.write(path, content);
        self.git(&["add", "--", path]);
    }

    fn commit(&self, message: &str) {
        self.git(&["commit", "--quiet", "--allow-empty", "-m", message]);
    }

    /// Git as the app runs it, with the user's configuration left out.
    fn scry_git(&self) -> GitCli {
        GitCli::new(self.path())
            .with_env(
                "GIT_CONFIG_GLOBAL",
                &self.root.path().join("gitconfig").display().to_string(),
            )
            .with_env("GIT_CONFIG_NOSYSTEM", "1")
    }

    fn files(&self, comparison: &Comparison) -> Vec<ChangedFile> {
        diff::changed_files(&self.scry_git(), &self.path(), comparison, &[]).unwrap()
    }

    /// Everything not committed.
    fn uncommitted(&self) -> (Comparison, Vec<ChangedFile>) {
        let comparison = Comparison::uncommitted(&self.scry_git()).unwrap();
        let files = self.files(&comparison);
        (comparison, files)
    }

    fn diff(&self, comparison: &Comparison, file: &ChangedFile) -> FileDiff {
        diff::file_diff(
            &self.scry_git(),
            &self.path(),
            comparison,
            file,
            Marks::Auto,
        )
        .unwrap()
    }

    /// The text diff of the one file not committed.
    fn only_text_diff(&self) -> TextDiff {
        let (comparison, files) = self.uncommitted();
        assert_eq!(files.len(), 1, "{files:?}");
        text(self.diff(&comparison, &files[0]))
    }
}

fn text(diff: FileDiff) -> TextDiff {
    match diff {
        FileDiff::Text(text) => text,
        other => panic!("not a text diff: {other:?}"),
    }
}

fn named<'a>(files: &'a [ChangedFile], path: &str) -> &'a ChangedFile {
    files
        .iter()
        .find(|file| file.path == path)
        .unwrap_or_else(|| panic!("{path} is not listed: {files:?}"))
}

/// Each file as "path state group +added -removed", in the list's order.
fn summary(files: &[ChangedFile]) -> Vec<String> {
    files
        .iter()
        .map(|file| {
            format!(
                "{} {:?} {:?} +{:?} -{:?}",
                file.path, file.state, file.group, file.added, file.removed
            )
        })
        .collect()
}

/// The rows as "old|new kind", to read a diff's shape at a glance.
fn shape(text: &TextDiff) -> Vec<String> {
    text.rows
        .iter()
        .map(|row| {
            let number = |number: Option<usize>| match number {
                Some(number) => number.to_string(),
                None => String::from("-"),
            };
            format!("{}|{} {:?}", number(row.old), number(row.new), row.kind)
        })
        .collect()
}

/// What is not committed comes in three groups, staged, not staged and
/// untracked, each file with its state and its counts against HEAD; an
/// ignored file is left out, as it is from `git status`.
#[test]
fn what_is_not_committed_is_grouped_by_where_it_stands() {
    let repo = Repo::new();
    repo.stage(".gitignore", "build/\n");
    repo.stage("staged.txt", "a\nb\n");
    repo.stage("edited.txt", "a\nb\n");
    repo.stage("removed.txt", "a\nb\n");
    repo.commit("base");

    repo.stage("staged.txt", "a\nB\nc\n");
    repo.stage("new.txt", "one\n");
    repo.write("edited.txt", "a\n");
    fs::remove_file(repo.path().join("removed.txt")).unwrap();
    repo.write("docs/untracked.md", "one\ntwo\nthree");
    repo.write("build/output.o", "ignored\n");

    let (_, files) = repo.uncommitted();
    assert_eq!(
        summary(&files),
        [
            "new.txt Added Some(Staged) +Some(1) -Some(0)",
            "staged.txt Modified Some(Staged) +Some(2) -Some(1)",
            "edited.txt Modified Some(NotStaged) +Some(0) -Some(1)",
            "removed.txt Deleted Some(NotStaged) +Some(0) -Some(2)",
            "docs/untracked.md Untracked Some(Untracked) +Some(3) -Some(0)",
        ]
    );
    let untracked = named(&files, "docs/untracked.md");
    assert_eq!(
        (untracked.name.as_str(), untracked.directory.as_str()),
        ("untracked.md", "docs/")
    );
}

/// A file with some changes staged and others not is listed once, under
/// not staged and flagged partly staged, and its diff is the whole
/// difference with HEAD: the reader sees the file once, entirely.
#[test]
fn a_partly_staged_file_is_one_entry_with_its_whole_diff() {
    let repo = Repo::new();
    repo.stage("a.txt", "one\ntwo\nthree\n");
    repo.commit("base");
    repo.stage("a.txt", "ONE\ntwo\nthree\n");
    repo.write("a.txt", "ONE\ntwo\nTHREE\n");

    let (comparison, files) = repo.uncommitted();
    assert_eq!(files.len(), 1, "{files:?}");
    let file = &files[0];
    assert_eq!(file.group, Some(Group::NotStaged));
    assert!(file.partly_staged);
    assert_eq!((file.added, file.removed), (Some(2), Some(2)));
    let text = text(repo.diff(&comparison, file));
    assert_eq!(text.new_lines, ["ONE", "two", "THREE"]);
    assert_eq!(text.counts(), (2, 2));

    // Wholly staged, it moves to the staged group and loses the flag.
    repo.git(&["add", "a.txt"]);
    let (_, files) = repo.uncommitted();
    assert_eq!(files[0].group, Some(Group::Staged));
    assert!(!files[0].partly_staged);
}

/// A change staged and then undone by hand leaves the file equal to HEAD
/// while the index still holds the change: the file stays listed, with no
/// line changed, so what the next commit would take is not hidden.
#[test]
fn a_staged_change_undone_in_the_working_copy_is_still_listed() {
    let repo = Repo::new();
    repo.stage("a.txt", "one\n");
    repo.commit("base");
    repo.stage("a.txt", "two\n");
    repo.write("a.txt", "one\n");

    let (comparison, files) = repo.uncommitted();
    assert_eq!(
        summary(&files),
        ["a.txt Modified Some(NotStaged) +Some(0) -Some(0)"]
    );
    assert!(files[0].partly_staged);
    let text = text(repo.diff(&comparison, &files[0]));
    assert_eq!(shape(&text), ["1|1 Unchanged"]);
}

/// The file is shown whole, not as hunks: every line of both sides is in
/// a row, once and in order, changed lines sit beside what replaces them
/// with their changed words marked, and "next change" has its blocks.
#[test]
fn a_file_is_shown_whole_with_its_changed_rows() {
    let repo = Repo::new();
    let old: String = (1..=30).map(|number| format!("line {number}\n")).collect();
    repo.stage("a.txt", &old);
    repo.commit("base");
    let new = old
        .replace("line 2\n", "line two\n")
        .replace("line 20\n", "")
        .replace("line 29\n", "line 29\nadded a\nadded b\n");
    repo.write("a.txt", &new);

    let text = repo.only_text_diff();
    assert_eq!(text.old_lines.len(), 30);
    assert_eq!(text.new_lines.len(), 31);
    assert_eq!(text.rows.len(), 32);
    assert_eq!(text.rows[0].kind, RowKind::Unchanged);

    let changed = &text.rows[1];
    assert_eq!(
        (changed.kind, changed.old, changed.new),
        (RowKind::Changed, Some(2), Some(2))
    );
    assert_eq!(&text.old_lines[1][changed.old_words[0].clone()], "2");
    assert_eq!(&text.new_lines[1][changed.new_words[0].clone()], "two");

    let removed = &text.rows[19];
    assert_eq!(
        (removed.kind, removed.old, removed.new),
        (RowKind::Removed, Some(20), None)
    );
    let added: Vec<_> = text
        .rows
        .iter()
        .filter(|row| row.kind == RowKind::Added)
        .map(|row| row.new)
        .collect();
    assert_eq!(added, [Some(29), Some(30)]);

    let changes: Vec<_> = text
        .changes()
        .iter()
        .map(|change| {
            (
                change.first_row,
                change.row_count,
                change.added,
                change.removed,
            )
        })
        .collect();
    assert_eq!(changes, [(1, 1, 1, 1), (19, 1, 0, 1), (29, 2, 2, 0)]);
    assert_eq!(text.counts(), (3, 2));
}

/// In a block, lines that are alike share a row with their words marked,
/// and the others sit side by side as far as they go, so the split view
/// is no taller than the longer side; the unified view gives the same
/// block as its removed lines, then its added ones.
#[test]
fn a_block_pairs_like_lines_and_the_unified_view_groups_it() {
    let repo = Repo::new();
    repo.stage(
        "a.swift",
        "top\nlet timeout = 30\nfunc cancelAll() {\nbottom\n",
    );
    repo.commit("base");
    repo.write(
        "a.swift",
        "top\n// retries\nlet timeout = 60\nvar pending = 0\nbottom\n",
    );

    let text = repo.only_text_diff();
    assert_eq!(
        shape(&text),
        [
            "1|1 Unchanged",
            "-|2 Added",
            "2|3 Changed",
            "3|4 Changed",
            "4|5 Unchanged"
        ]
    );
    // "let timeout" was edited: its number is marked on both sides.
    assert_eq!(&text.old_lines[1][text.rows[2].old_words[0].clone()], "30");
    assert_eq!(&text.new_lines[2][text.rows[2].new_words[0].clone()], "60");
    // The last two lines have nothing in common: side by side, unmarked.
    assert!(text.rows[3].old_words.is_empty() && text.rows[3].new_words.is_empty());

    let unified: Vec<_> = text
        .unified()
        .iter()
        .map(|line| (line.kind, line.old, line.new))
        .collect();
    assert_eq!(
        unified,
        [
            (LineKind::Unchanged, Some(1), Some(1)),
            (LineKind::Removed, Some(2), None),
            (LineKind::Removed, Some(3), None),
            (LineKind::Added, None, Some(2)),
            (LineKind::Added, None, Some(3)),
            (LineKind::Added, None, Some(4)),
            (LineKind::Unchanged, Some(4), Some(5)),
        ]
    );
    assert_eq!(text.unified()[1].words, text.rows[2].old_words);
}

/// A renamed file is one entry with its old path, not a deletion and an
/// addition, and its diff compares the old path's content with the new
/// one's; renamed without a change, it is shown plain.
#[test]
fn a_renamed_file_is_one_entry_compared_with_its_old_path() {
    let repo = Repo::new();
    let content: String = (1..=12).map(|number| format!("line {number}\n")).collect();
    repo.stage("old/name.txt", &content);
    repo.stage("plain.txt", content.replace("line", "row"));
    repo.commit("base");
    repo.git(&["mv", "old/name.txt", "new-name.txt"]);
    repo.stage("new-name.txt", content.replace("line 3\n", "line three\n"));
    repo.git(&["mv", "plain.txt", "moved.txt"]);

    let (comparison, files) = repo.uncommitted();
    assert_eq!(files.len(), 2, "{files:?}");
    let renamed = named(&files, "new-name.txt");
    assert_eq!(renamed.state, FileState::Renamed);
    assert_eq!(renamed.old_path.as_deref(), Some("old/name.txt"));
    assert_eq!((renamed.added, renamed.removed), (Some(1), Some(1)));
    let text_diff = text(repo.diff(&comparison, renamed));
    assert_eq!(text_diff.rows.len(), 12);
    assert_eq!(text_diff.counts(), (1, 1));

    let moved = named(&files, "moved.txt");
    assert_eq!(moved.old_path.as_deref(), Some("plain.txt"));
    assert_eq!((moved.added, moved.removed), (Some(0), Some(0)));
    let plain = text(repo.diff(&comparison, moved));
    assert_eq!(plain.rows.len(), 12);
    assert!(plain.rows.iter().all(|row| row.kind == RowKind::Unchanged));
    assert_eq!(plain.new_lines[0], "row 1");
}

/// A new file has every line added and a deleted one every line removed,
/// with the content it had: the reader sees what appears and what goes.
#[test]
fn added_and_deleted_files_show_all_their_lines() {
    let repo = Repo::new();
    repo.stage("gone.txt", "one\ntwo\n");
    repo.commit("base");
    repo.git(&["rm", "--quiet", "gone.txt"]);
    repo.stage("new.txt", "a\nb\nc\n");
    repo.write("untracked.txt", "x\ny\n");

    let (comparison, files) = repo.uncommitted();
    let gone = text(repo.diff(&comparison, named(&files, "gone.txt")));
    assert_eq!(shape(&gone), ["1|- Removed", "2|- Removed"]);
    assert_eq!(gone.old_lines, ["one", "two"]);
    assert!(gone.new_lines.is_empty());

    let new = text(repo.diff(&comparison, named(&files, "new.txt")));
    assert_eq!(shape(&new), ["-|1 Added", "-|2 Added", "-|3 Added"]);

    let untracked = text(repo.diff(&comparison, named(&files, "untracked.txt")));
    assert_eq!(shape(&untracked), ["-|1 Added", "-|2 Added"]);
    assert_eq!(untracked.new_lines, ["x", "y"]);
}

/// A binary file has no rows: the list flags it without counts and its
/// diff gives the size on each side, tracked or not.
#[test]
fn a_binary_file_gives_its_sizes_instead_of_rows() {
    let repo = Repo::new();
    repo.stage("icon.png", [0u8, 1, 2, 3]);
    repo.commit("base");
    repo.write("icon.png", [0u8, 1, 2, 3, 4, 5, 6]);
    repo.write("new.bin", [9u8, 0, 9]);

    let (comparison, files) = repo.uncommitted();
    let icon = named(&files, "icon.png");
    assert!(icon.binary);
    assert_eq!((icon.added, icon.removed), (None, None));
    assert_eq!(
        repo.diff(&comparison, icon),
        FileDiff::Binary {
            old_size: Some(4),
            new_size: Some(7)
        }
    );
    let new = named(&files, "new.bin");
    assert!(new.binary);
    assert_eq!(
        repo.diff(&comparison, new),
        FileDiff::Binary {
            old_size: None,
            new_size: Some(3)
        }
    );
}

/// During a merge, a file left with conflicts is listed once, as
/// conflicted and without counts, and has no diff: a diff of conflict
/// markers means nothing, the merge editor resolves the file.
#[test]
fn a_conflicted_file_is_listed_once_without_a_diff() {
    let repo = Repo::new();
    repo.stage("a.txt", "base\n");
    repo.stage("b.txt", "base\n");
    repo.commit("base");
    repo.git(&["switch", "--quiet", "-c", "feature"]);
    repo.stage("a.txt", "feature\n");
    repo.stage("b.txt", "feature\n");
    repo.commit("feature");
    repo.git(&["switch", "--quiet", "main"]);
    repo.stage("a.txt", "main\n");
    repo.commit("main");
    assert!(!repo.try_git(&["merge", "--quiet", "feature"]));

    let (comparison, files) = repo.uncommitted();
    assert_eq!(
        summary(&files),
        [
            "b.txt Modified Some(Staged) +Some(1) -Some(1)",
            "a.txt Conflicted Some(NotStaged) +None -None",
        ]
    );
    assert!(!named(&files, "a.txt").partly_staged);
    assert_eq!(
        repo.diff(&comparison, named(&files, "a.txt")),
        FileDiff::Conflicted
    );
}

/// Before the first commit there is no HEAD to compare with: everything
/// is new, staged or untracked, instead of an error.
#[test]
fn a_repository_without_a_commit_shows_everything_as_new() {
    let repo = Repo::new();
    repo.stage("staged.txt", "a\n");
    repo.write("untracked.txt", "b\n");

    let (comparison, files) = repo.uncommitted();
    assert_eq!(
        summary(&files),
        [
            "staged.txt Added Some(Staged) +Some(1) -Some(0)",
            "untracked.txt Untracked Some(Untracked) +Some(1) -Some(0)",
        ]
    );
    let staged = text(repo.diff(&comparison, &files[0]));
    assert_eq!(shape(&staged), ["-|1 Added"]);
}

/// A range compares two commits and leaves the working copy out, with no
/// group; `a...b` compares b with its merge base with a, so main's own
/// later commits do not show as removed by the branch.
#[test]
fn a_range_compares_commits_and_three_dots_start_at_the_merge_base() {
    let repo = Repo::new();
    repo.stage("a.txt", "one\n");
    repo.commit("base");
    repo.git(&["switch", "--quiet", "-c", "feature"]);
    repo.stage("feature.txt", "feature\n");
    repo.commit("feature");
    repo.git(&["switch", "--quiet", "main"]);
    repo.stage("main.txt", "main\n");
    repo.commit("main");
    repo.write("untracked.txt", "not in a range\n");

    let two_dots = Comparison::Commits {
        old: String::from("main"),
        new: String::from("feature"),
    };
    assert_eq!(
        summary(&repo.files(&two_dots)),
        [
            "feature.txt Added None +Some(1) -Some(0)",
            "main.txt Deleted None +Some(0) -Some(1)",
        ]
    );
    let three_dots = Comparison::since_merge_base(&repo.scry_git(), "main", "feature").unwrap();
    let files = repo.files(&three_dots);
    assert_eq!(
        summary(&files),
        ["feature.txt Added None +Some(1) -Some(0)"]
    );
    assert_eq!(
        text(repo.diff(&three_dots, &files[0])).new_lines,
        ["feature"]
    );
}

/// `--staged` compares the index with HEAD: a later edit of the working
/// copy and an untracked file are not part of it, in the list or in the
/// file's rows.
#[test]
fn staged_shows_the_index_not_the_working_copy() {
    let repo = Repo::new();
    repo.stage("a.txt", "one\n");
    repo.commit("base");
    repo.stage("a.txt", "one\nstaged\n");
    repo.write("a.txt", "one\nstaged\nnot staged\n");
    repo.write("untracked.txt", "x\n");

    let staged = Comparison::staged(&repo.scry_git()).unwrap();
    let files = repo.files(&staged);
    assert_eq!(summary(&files), ["a.txt Modified None +Some(1) -Some(0)"]);
    assert_eq!(
        text(repo.diff(&staged, &files[0])).new_lines,
        ["one", "staged"]
    );

    let unstaged = repo.files(&Comparison::Unstaged);
    assert_eq!(
        text(repo.diff(&Comparison::Unstaged, &unstaged[0])).old_lines,
        ["one", "staged"]
    );
}

/// Paths limit the list to files and folders, tracked or not, and are
/// names, never patterns: `[id].tsx` does not also select `i.tsx`.
#[test]
fn paths_limit_the_list_and_are_taken_literally() {
    let repo = Repo::new();
    repo.stage("src/[id].tsx", "a\n");
    repo.stage("src/i.tsx", "a\n");
    repo.stage("docs/a.md", "a\n");
    repo.commit("base");
    repo.write("src/[id].tsx", "b\n");
    repo.write("src/i.tsx", "b\n");
    repo.write("docs/a.md", "b\n");
    repo.write("docs/new file é.md", "b\n");

    let comparison = Comparison::uncommitted(&repo.scry_git()).unwrap();
    let list = |paths: &[&str]| -> Vec<String> {
        let paths: Vec<String> = paths.iter().map(|path| path.to_string()).collect();
        diff::changed_files(&repo.scry_git(), &repo.path(), &comparison, &paths)
            .unwrap()
            .into_iter()
            .map(|file| file.path)
            .collect()
    };
    assert_eq!(list(&["src/[id].tsx"]), ["src/[id].tsx"]);
    assert_eq!(list(&["docs"]), ["docs/a.md", "docs/new file é.md"]);

    // A name git would quote in its usual output is read as it is.
    let (comparison, files) = repo.uncommitted();
    let odd = named(&files, "docs/new file é.md");
    assert_eq!(text(repo.diff(&comparison, odd)).new_lines, ["b"]);
    let bracket = named(&files, "src/[id].tsx");
    assert_eq!(text(repo.diff(&comparison, bracket)).counts(), (1, 1));
}

/// The user's git configuration may colour diffs, drop the a/ and b/
/// prefixes, run an external diff tool, turn renames off or shorten the
/// context: none of it changes what Scry reads.
#[test]
fn the_users_diff_configuration_does_not_change_what_is_read() {
    let repo = Repo::new();
    let content: String = (1..=12).map(|number| format!("line {number}\n")).collect();
    repo.stage("a.txt", &content);
    repo.stage("old.txt", content.replace("line", "row"));
    repo.commit("base");
    for (key, value) in [
        ("color.ui", "always"),
        ("color.diff", "always"),
        ("diff.noprefix", "true"),
        ("diff.mnemonicPrefix", "true"),
        ("diff.external", "false"),
        ("diff.renames", "false"),
        ("diff.context", "0"),
        ("diff.relative", "true"),
        ("core.quotePath", "true"),
    ] {
        repo.git(&["config", key, value]);
    }
    repo.write("a.txt", content.replace("line 6\n", "line six\n"));
    repo.git(&["mv", "old.txt", "new.txt"]);

    let (comparison, files) = repo.uncommitted();
    assert_eq!(
        summary(&files),
        [
            "new.txt Renamed Some(Staged) +Some(0) -Some(0)",
            "a.txt Modified Some(NotStaged) +Some(1) -Some(1)",
        ]
    );
    let text = text(repo.diff(&comparison, named(&files, "a.txt")));
    assert_eq!(text.rows.len(), 12);
    assert_eq!(text.new_lines[5], "line six");
    assert_eq!(text.old_lines[11], "line 12");
}

/// A side whose last line has no line ending is flagged, and when only
/// one side has none git counts that line as changed: the rows agree
/// with `git diff` instead of hiding the difference.
#[test]
fn a_missing_final_line_ending_is_reported() {
    let repo = Repo::new();
    repo.stage("a.txt", "one\ntwo\n");
    repo.commit("base");
    repo.write("a.txt", "one\ntwo");

    let text = repo.only_text_diff();
    assert!(!text.old_missing_final_newline);
    assert!(text.new_missing_final_newline);
    assert_eq!(shape(&text), ["1|1 Unchanged", "2|2 Changed"]);
    assert_eq!(text.counts(), (1, 1));

    // An untracked file says so too.
    repo.write("new.txt", "x");
    let (comparison, files) = repo.uncommitted();
    let new = self::text(repo.diff(&comparison, named(&files, "new.txt")));
    assert!(new.new_missing_final_newline);
    assert_eq!(new.new_lines, ["x"]);
}

/// With Windows line endings in the working copy and git set to convert
/// them, an unchanged line stays unchanged and no carriage return ends a
/// line Scry shows.
#[test]
fn converted_line_endings_are_not_a_change() {
    let repo = Repo::new();
    repo.git(&["config", "core.autocrlf", "true"]);
    repo.stage("a.txt", "one\ntwo\nthree\n");
    repo.commit("base");
    repo.write("a.txt", "one\r\nTWO\r\nthree\r\n");

    let text = repo.only_text_diff();
    assert_eq!(
        shape(&text),
        ["1|1 Unchanged", "2|2 Changed", "3|3 Unchanged"]
    );
    assert_eq!(text.new_lines, ["one", "TWO", "three"]);
}

/// A folder holding a repository of its own is one entry, not its files,
/// and has no rows.
#[test]
fn a_nested_repository_is_one_entry() {
    let repo = Repo::new();
    repo.stage("a.txt", "one\n");
    repo.commit("base");
    fs::create_dir(repo.path().join("vendor")).unwrap();
    let nested = repo.path().join("vendor");
    assert!(
        Command::new("git")
            .arg("-C")
            .arg(&nested)
            .args(["init", "--quiet"])
            .status()
            .unwrap()
            .success()
    );
    fs::write(nested.join("inner.txt"), "x\n").unwrap();

    let (comparison, files) = repo.uncommitted();
    assert_eq!(files.len(), 1, "{files:?}");
    assert_eq!(files[0].path, "vendor");
    assert!(files[0].submodule);
    assert_eq!(repo.diff(&comparison, &files[0]), FileDiff::Submodule);
}

/// A large file opens without pairing its lines or marking words, and
/// says so, unless the user asks for the detail on that file.
#[test]
fn a_large_file_skips_the_marks_unless_asked() {
    let repo = Repo::new();
    let lines = diff::LARGE_FILE_LINES / 2 + 1;
    let old: String = (0..lines)
        .map(|number| format!("line {number}\n"))
        .collect();
    repo.stage("big.txt", &old);
    repo.commit("base");
    repo.write("big.txt", old.replace("line 7\n", "line seven\n"));

    let (comparison, files) = repo.uncommitted();
    let auto = text(repo.diff(&comparison, &files[0]));
    assert!(auto.marks_skipped);
    assert_eq!(auto.rows[7].kind, RowKind::Changed);
    assert!(auto.rows[7].new_words.is_empty());

    let always = text(
        diff::file_diff(
            &repo.scry_git(),
            &repo.path(),
            &comparison,
            &files[0],
            Marks::Always,
        )
        .unwrap(),
    );
    assert!(!always.marks_skipped);
    assert_eq!(
        &always.new_lines[7][always.rows[7].new_words[0].clone()],
        "seven"
    );
}

/// Two files outside any repository, as `git difftool` hands them over:
/// their rows when they differ, the file plain when they are the same,
/// sizes when they are binary, and an error for a missing file.
#[test]
fn two_plain_files_are_compared_without_a_repository() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("gitconfig"), "").unwrap();
    let git = GitCli::new(dir.path().to_path_buf())
        .with_env(
            "GIT_CONFIG_GLOBAL",
            &dir.path().join("gitconfig").display().to_string(),
        )
        .with_env("GIT_CONFIG_NOSYSTEM", "1");
    let file = |name: &str, content: &[u8]| {
        let path = dir.path().join(name);
        fs::write(&path, content).unwrap();
        path
    };
    let old = file("old.txt", b"one\ntwo\n");
    let new = file("new.txt", b"one\n2\nthree\n");
    let differing = text(diff::plain_files(&git, &old, &new, Marks::Auto).unwrap());
    assert_eq!(
        shape(&differing),
        ["1|1 Unchanged", "2|2 Changed", "-|3 Added"]
    );

    let same = text(diff::plain_files(&git, &old, &old, Marks::Auto).unwrap());
    assert_eq!(shape(&same), ["1|1 Unchanged", "2|2 Unchanged"]);

    let binary = file("a.bin", &[0, 1, 2]);
    assert_eq!(
        diff::plain_files(&git, &binary, &new, Marks::Auto).unwrap(),
        FileDiff::Binary {
            old_size: Some(3),
            new_size: Some(12)
        }
    );
    assert!(diff::plain_files(&git, &old, &dir.path().join("missing"), Marks::Auto).is_err());
}

/// Between two commits, a file only renamed and a binary file are read
/// from the commits, not from the working copy, which may have moved on
/// since.
#[test]
fn a_range_reads_its_files_from_the_commits() {
    let repo = Repo::new();
    repo.stage("old.txt", "one\ntwo\n");
    repo.stage("icon.png", [0u8, 1, 2]);
    repo.commit("base");
    repo.git(&["mv", "old.txt", "new.txt"]);
    repo.stage("icon.png", [0u8, 1, 2, 3, 4]);
    repo.commit("rename");
    repo.write("new.txt", "changed since\n");
    repo.write("icon.png", [0u8; 40]);

    let range = Comparison::Commits {
        old: String::from("HEAD~1"),
        new: String::from("HEAD"),
    };
    let files = repo.files(&range);
    let renamed = text(repo.diff(&range, named(&files, "new.txt")));
    assert_eq!(renamed.new_lines, ["one", "two"]);
    assert_eq!(
        repo.diff(&range, named(&files, "icon.png")),
        FileDiff::Binary {
            old_size: Some(3),
            new_size: Some(5)
        }
    );

    // The same from the index, with `--staged`.
    repo.git(&["mv", "new.txt", "newer.txt"]);
    repo.write("newer.txt", "changed again\n");
    let staged = Comparison::staged(&repo.scry_git()).unwrap();
    let files = repo.files(&staged);
    let renamed = text(repo.diff(&staged, named(&files, "newer.txt")));
    assert_eq!(renamed.new_lines, ["one", "two"]);
}

/// An untracked file too large to read for the list is listed without
/// counts, so a forgotten build product does not slow the list down; its
/// rows are still there when it is opened.
#[test]
fn a_large_untracked_file_is_listed_without_counts() {
    let repo = Repo::new();
    repo.stage("a.txt", "one\n");
    repo.commit("base");
    repo.write("big.log", "0123456789\n".repeat(200_000));
    repo.write("small.log", "x\n");

    let (comparison, files) = repo.uncommitted();
    let big = named(&files, "big.log");
    assert_eq!((big.added, big.removed, big.binary), (None, None, false));
    assert_eq!(named(&files, "small.log").added, Some(1));
    assert_eq!(text(repo.diff(&comparison, big)).rows.len(), 200_000);
}

/// What is asked is named for the window: what is read, what it is
/// compared with, and each side's label for the bar above the file; the
/// branch is given while the working copy or the index is read.
#[test]
fn a_scope_names_what_is_compared() {
    let repo = Repo::new();
    repo.stage("a.txt", "one\n");
    repo.commit("base");
    repo.git(&["switch", "--quiet", "-c", "feature"]);
    repo.stage("b.txt", "two\n");
    repo.commit("feature");
    let git = repo.scry_git();
    let words = |staged: bool, revisions: Option<&str>| {
        let scope = diff::Scope::of(&git, staged, revisions).unwrap();
        (
            scope.sentence(),
            scope.old_label.clone(),
            scope.new_label.clone(),
            scope.branch.clone(),
        )
    };
    let text = |a: &str, b: &str, c: &str, branch: Option<&str>| {
        (
            a.to_string(),
            b.to_string(),
            c.to_string(),
            branch.map(str::to_string),
        )
    };

    assert_eq!(
        words(false, None),
        text(
            "Not committed, against HEAD",
            "HEAD",
            "Working copy",
            Some("feature")
        )
    );
    assert_eq!(
        words(true, None),
        text("Staged, against HEAD", "HEAD", "Index", Some("feature"))
    );
    assert_eq!(
        words(false, Some("main")),
        text(
            "Working copy, against main",
            "main",
            "Working copy",
            Some("feature")
        )
    );
    assert_eq!(
        words(true, Some("main")),
        text("Staged, against main", "main", "Index", Some("feature"))
    );
    assert_eq!(
        words(false, Some("main..feature")),
        text("feature, against main", "main", "feature", None)
    );
    // A missing end of a range is HEAD.
    assert_eq!(
        words(false, Some("main..")),
        text("HEAD, against main", "main", "HEAD", None)
    );

    // Three dots: the merge base, by its short id.
    let scope = diff::Scope::of(&git, false, Some("main...feature")).unwrap();
    let base = scope.old_label.clone();
    let main = Command::new("git")
        .arg("-C")
        .arg(repo.path())
        .args(["rev-parse", "--short", "main"])
        .output()
        .unwrap();
    assert_eq!(base, String::from_utf8(main.stdout).unwrap().trim());
    assert_eq!(
        scope.sentence(),
        format!("feature, against its merge base with main ({base})")
    );
    assert_eq!(
        summary(&repo.files(&scope.comparison)),
        ["b.txt Added None +Some(1) -Some(0)"]
    );

    // The index has one commit to be compared with.
    for range in ["main..feature", "main...feature"] {
        let refused = diff::Scope::of(&git, true, Some(range)).unwrap_err();
        assert_eq!(
            refused.to_string(),
            "--staged takes one commit, not a range"
        );
    }
    assert!(diff::Scope::of(&git, false, Some("no-such...feature")).is_err());

    // Detached, there is no branch to name.
    repo.git(&["switch", "--quiet", "--detach", "HEAD"]);
    assert_eq!(words(false, None).3, None);
}

/// Before the first commit, what is not committed is compared with
/// nothing, and the window says so rather than naming a HEAD that does
/// not exist.
#[test]
fn a_scope_without_a_commit_is_compared_with_nothing() {
    let repo = Repo::new();
    repo.write("a.txt", "one\n");
    let scope = diff::Scope::of(&repo.scry_git(), false, None).unwrap();
    assert_eq!(
        scope.sentence(),
        "Not committed, against nothing: no commit yet"
    );
    assert_eq!(scope.old_label, "nothing: no commit yet");
    assert_eq!(repo.files(&scope.comparison).len(), 1);
}

/// A git that runs outside any repository, in `dir`, with the user's
/// configuration left out.
fn plain_git(dir: &std::path::Path) -> GitCli {
    fs::write(dir.join("gitconfig"), "").unwrap();
    GitCli::new(dir.to_path_buf())
        .with_env(
            "GIT_CONFIG_GLOBAL",
            &dir.join("gitconfig").display().to_string(),
        )
        .with_env("GIT_CONFIG_NOSYSTEM", "1")
}

/// Two folders, as `git difftool --dir-diff` hands them over: the files
/// that differ are listed by their path with their counts, a file in one
/// folder only is added or deleted, a file that is the same is left out,
/// and each one's rows are read from the two folders.
#[test]
fn two_folders_list_the_files_that_differ() {
    let dir = tempfile::tempdir().unwrap();
    let git = plain_git(dir.path());
    let (left, right) = (dir.path().join("left"), dir.path().join("right"));
    let write = |folder: &std::path::Path, path: &str, content: &[u8]| {
        let path = folder.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    };
    write(&left, "src/a.txt", b"one\ntwo\n");
    write(&right, "src/a.txt", b"one\n2\nthree\n");
    write(&left, "same.txt", b"same\n");
    write(&right, "same.txt", b"same\n");
    write(&left, "gone.txt", b"x\ny\n");
    write(&right, "docs/new.md", b"new\n");
    write(&left, "icon.png", &[0, 1]);
    write(&right, "icon.png", &[0, 1, 2]);
    // Binary on one side is enough: there are no lines to count.
    write(&left, "became.bin", b"text\n");
    write(&right, "became.bin", &[7, 0, 7]);
    write(&left, "was.bin", &[7, 0, 7]);
    write(&right, "was.bin", b"text\n");

    let files = diff::folder_files(&git, &left, &right).unwrap();
    assert_eq!(
        summary(&files),
        [
            "became.bin Modified None +None -None",
            "docs/new.md Added None +Some(1) -Some(0)",
            "gone.txt Deleted None +Some(0) -Some(2)",
            "icon.png Modified None +None -None",
            "src/a.txt Modified None +Some(2) -Some(1)",
            "was.bin Modified None +None -None",
        ]
    );
    for path in ["icon.png", "became.bin", "was.bin"] {
        assert!(named(&files, path).binary, "{path}");
    }

    let rows = |path: &str| {
        diff::folder_file_diff(&git, &left, &right, named(&files, path), Marks::Auto).unwrap()
    };
    assert_eq!(
        shape(&text(rows("src/a.txt"))),
        ["1|1 Unchanged", "2|2 Changed", "-|3 Added"]
    );
    assert_eq!(shape(&text(rows("docs/new.md"))), ["-|1 Added"]);
    assert_eq!(
        shape(&text(rows("gone.txt"))),
        ["1|- Removed", "2|- Removed"]
    );
    assert_eq!(
        rows("icon.png"),
        FileDiff::Binary {
            old_size: Some(2),
            new_size: Some(3)
        }
    );
    assert!(diff::folder_files(&git, &left, &dir.path().join("missing")).is_err());
}

/// With `--dir-diff`, git puts links to the working copy's files in the
/// right-hand folder: the file a link points to is compared, not the
/// link's own text, in the list and in the rows.
#[cfg(unix)]
#[test]
fn links_in_a_folder_are_followed_to_their_files() {
    let dir = tempfile::tempdir().unwrap();
    let git = plain_git(dir.path());
    let (left, right) = (dir.path().join("left"), dir.path().join("right"));
    let working = dir.path().join("working");
    for folder in [&left, &right, &working] {
        fs::create_dir_all(folder.join("sub")).unwrap();
    }
    fs::write(left.join("sub/a.txt"), "one\n").unwrap();
    fs::write(working.join("sub/a.txt"), "one\ntwo\n").unwrap();
    std::os::unix::fs::symlink(working.join("sub/a.txt"), right.join("sub/a.txt")).unwrap();
    // A link whose file is gone is not a file to compare.
    std::os::unix::fs::symlink(working.join("gone.txt"), right.join("gone.txt")).unwrap();

    let files = diff::folder_files(&git, &left, &right).unwrap();
    assert_eq!(
        summary(&files),
        ["sub/a.txt Modified None +Some(1) -Some(0)"]
    );
    let rows = diff::folder_file_diff(&git, &left, &right, &files[0], Marks::Auto).unwrap();
    assert_eq!(text(rows).new_lines, ["one", "two"]);

    let linked = diff::plain_files(
        &git,
        &left.join("sub/a.txt"),
        &right.join("sub/a.txt"),
        Marks::Auto,
    );
    assert_eq!(text(linked.unwrap()).counts(), (1, 0));
}
