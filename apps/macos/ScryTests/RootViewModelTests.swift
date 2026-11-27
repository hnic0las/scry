import Foundation
import Testing
@testable import Scry

@MainActor
struct RootViewModelTests {
    /// The file to open is the first argument naming an existing file:
    /// Xcode's own flags and their values are skipped.
    @Test func findsTheLaunchFile() throws {
        let file = try ConflictFile()
        let arguments = ["Scry", "-NSDocumentRevisionsDebugMode", "YES", file.url.path]

        #expect(RootViewModel.launchPath(in: arguments) == file.url.path)
        #expect(RootViewModel.launchPath(in: ["Scry", "/no/such/file"]) == nil)
    }

    /// Opening a file shows the editor; a file that cannot be opened keeps
    /// the empty state with the reason.
    @Test func opensAFileOrSaysWhy() async throws {
        let root = RootViewModel()

        await root.open("/no/such/file.swift")
        #expect(root.editor == nil)
        #expect(root.openError != nil)
        #expect(root.opening == nil)

        await root.open(try ConflictFile().url.path)
        #expect(root.editor != nil)
        #expect(root.openError == nil)
        #expect(root.opening == nil)
    }

    /// Closing the window with choices not written asks first, so a click
    /// on the close button does not lose a merge; once written, or with
    /// nothing chosen, it closes straight away.
    @Test func asksBeforeClosingOnUnwrittenChoices() async throws {
        let root = RootViewModel()
        #expect(root.shouldClose())

        await root.open(try ConflictFile().url.path)
        let editor = try #require(root.editor)
        #expect(root.shouldClose())

        editor.take("theirs")
        #expect(!root.shouldClose())
        #expect(root.confirmingClose)
        #expect(root.closeMessage.hasPrefix("1 of 2 conflicts resolved"))

        editor.next()
        editor.take("ours")
        editor.markResolved()
        #expect(root.shouldClose())
    }

    /// Once the user agreed to lose the choices, the window closes.
    @Test func closesWithoutWritingOnceConfirmed() async throws {
        let root = RootViewModel()
        await root.open(try ConflictFile().url.path)
        root.editor?.take("theirs")
        #expect(!root.shouldClose())

        root.closeWithoutWriting()
        #expect(root.shouldClose())
    }

    /// `git-scry resolve --app` passes each of the four files after its
    /// option; without all four, it is not the merge tool.
    @Test func findsTheMergetoolFiles() {
        let arguments = [
            "Scry", "--base", "/r/f_BASE", "--local", "/r/f_LOCAL",
            "--remote", "/r/f_REMOTE", "--merged", "/r/f",
        ]

        #expect(RootViewModel.mergetoolFiles(in: arguments) == MergetoolFiles(
            base: "/r/f_BASE", local: "/r/f_LOCAL", remote: "/r/f_REMOTE", merged: "/r/f"
        ))
        #expect(RootViewModel.mergetoolFiles(in: ["Scry", "--merged", "/r/f"]) == nil)
        #expect(RootViewModel.resultPath(in: arguments + ["--result", "/tmp/answer"]) == "/tmp/answer")
        #expect(RootViewModel.resultPath(in: arguments) == nil)
        #expect(RootViewModel.mergetoolFiles(in: ["Scry", "/r/f"]) == nil)
    }

    /// Opened by `git mergetool`, the file shows its base, rebuilt as the
    /// CLI does; the app answers 1 until the file is written, then 0, and
    /// closes so git moves on to the next file.
    @Test func answersGitMergetool() async throws {
        let files = try ConflictFile.mergetool()
        let root = RootViewModel()

        await root.open(mergetool: files)
        let editor = try #require(root.editor)
        #expect(root.isMergetool)
        #expect(editor.inspector.cards.contains { $0.id == "base" })
        #expect(root.exitStatus == 1)
        #expect(!root.finished)

        editor.take("theirs")
        editor.markResolved()

        #expect(root.finished)
        #expect(root.exitStatus == 0)
        #expect(try String(contentsOfFile: files.merged, encoding: .utf8) == "let a = 2\n")
    }

    /// Outside the merge tool, the app always exits 0 and stays open after
    /// writing.
    @Test func outsideTheMergetoolNothingChanges() async throws {
        let root = RootViewModel()
        await root.open(try ConflictFile().url.path)
        let editor = try #require(root.editor)
        editor.take("theirs")
        editor.next()
        editor.take("ours")
        editor.markResolved()

        #expect(!root.finished)
        #expect(root.exitStatus == 0)
    }

    /// A file opened from the overview has a way back; once written it goes
    /// back to the list, read again, so the file shows as resolved.
    @Test func filesOpenFromTheOverviewAndComeBack() async throws {
        let conflicted = try ConflictFile()
        let source = FakeOverview(files: [file("Sample.swift", .unresolved, absolutePath: conflicted.url.path)])
        let root = RootViewModel()
        await root.open(overview: source)
        let overview = try #require(root.overview)
        let readsBefore = source.reads

        overview.open("Sample.swift")
        // Opening goes through a task: wait for the editor.
        for _ in 0..<100 where root.editor == nil {
            try await Task.sleep(for: .milliseconds(20))
        }
        let editor = try #require(root.editor)
        #expect(editor.canGoBack)
        #expect(!root.returnsToOverview)

        editor.take("theirs")
        editor.next()
        editor.take("ours")
        editor.markResolved()
        #expect(root.returnsToOverview)

        root.leaveEditor()
        #expect(root.editor == nil)
        await overview.idle()
        #expect(source.reads > readsBefore)
    }

    /// Under `git mergetool`, a file that cannot be opened leaves no way to
    /// open another, and nothing else written ever answers 0: git would
    /// stage its file still conflicted.
    @Test func aMergetoolFileThatFailsNeverAnswersZero() async throws {
        let missing = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString).path
        let root = RootViewModel()
        await root.open(mergetool: MergetoolFiles(base: missing, local: missing, remote: missing, merged: missing))
        #expect(root.editor == nil)
        #expect(root.openError != nil)
        #expect(!root.canOpen)
        #expect(root.emptyMessage.hasPrefix("git mergetool gave Scry a file it could not open"))

        // Another file written, as the empty window's buttons once allowed.
        await root.open(try ConflictFile().url.path)
        let editor = try #require(root.editor)
        editor.take("theirs")
        editor.next()
        editor.take("ours")
        editor.markResolved()
        #expect(root.exitStatus == 1)
    }

    /// Under `git mergetool`, a file whose conflicts were all removed by
    /// hand before Scry opened it can still be written, so git hears it
    /// is resolved instead of always reading 1.
    @Test func aMergetoolFileWithNoConflictLeftAnswersZero() async throws {
        let files = try ConflictFile.mergetool(merged: "let a = 3\n")
        let root = RootViewModel()
        await root.open(mergetool: files)
        let editor = try #require(root.editor)
        #expect(editor.canMarkResolved)

        editor.markResolved()
        #expect(root.finished)
        #expect(root.exitStatus == 0)
        #expect(try String(contentsOfFile: files.merged, encoding: .utf8) == "let a = 3\n")
    }

    /// A file of the overview that cannot be opened says why over the
    /// list: the empty window, which shows the reason otherwise, is not
    /// on display.
    @Test func aFileThatFailsToOpenFromTheOverviewSaysWhy() async throws {
        let missing = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString).path
        let source = FakeOverview(files: [file("Gone.swift", .unresolved, absolutePath: missing)])
        let root = RootViewModel()
        await root.open(overview: source)
        let overview = try #require(root.overview)
        #expect(!root.failedOverOverview)

        overview.open("Gone.swift")
        for _ in 0..<100 where root.openError == nil {
            try await Task.sleep(for: .milliseconds(20))
        }
        #expect(root.editor == nil)
        #expect(root.failedOverOverview)

        root.dismissOpenError()
        #expect(!root.failedOverOverview)
    }

    /// File > Open works from the empty window and the overview, but never
    /// over a file in the editor, whose choices may not be written, nor
    /// under `git mergetool`, which owns the window's file.
    @Test func fileOpenOnlyWhereNothingIsLost() async throws {
        let root = RootViewModel()
        #expect(root.canOpen)

        let conflicted = try ConflictFile()
        await root.open(overview: FakeOverview(files: [file("Sample.swift", .unresolved, absolutePath: conflicted.url.path)]))
        #expect(root.canOpen)

        await root.open(conflicted.url.path)
        #expect(!root.canOpen)

        let mergetool = RootViewModel()
        await mergetool.open(mergetool: try ConflictFile.mergetool())
        #expect(!mergetool.canOpen)
    }

    /// `git scry diff` passes what to compare after `--diff`: the
    /// repository, then its options, each path after `--path`. Without
    /// `--diff` the window is not asked for changes.
    @Test func readsWhatGitScryDiffAsks() {
        let all = [
            "Scry", "--diff", "/r", "--staged", "--revisions", "main...topic", "--path", "src/a b.swift",
            "--path", "docs",
        ]
        #expect(RootViewModel.changesRequest(in: all) == RootViewModel.ChangesRequest(
            root: "/r", staged: true, revisions: "main...topic", paths: ["src/a b.swift", "docs"]
        ))
        #expect(RootViewModel.changesRequest(in: ["Scry", "--diff", "/r"]) == RootViewModel.ChangesRequest(root: "/r"))
        #expect(RootViewModel.changesRequest(in: ["Scry", "/r"]) == nil)
        #expect(RootViewModel.changesRequest(in: ["Scry", "--diff"]) == nil)
    }

    /// The window shows the changes it is given, and a folder that is not
    /// a repository leaves it empty with the reason.
    @Test func opensTheChangesOrSaysWhy() async {
        let root = RootViewModel()
        await root.openChanges(RootViewModel.ChangesRequest(root: "/no/such/folder"))
        #expect(root.changes == nil)
        #expect(root.openError != nil)

        root.open(changes: FakeChanges(files: []))
        #expect(root.changes != nil)
        #expect(root.editor == nil)
        #expect(root.overview == nil)
    }

    /// `git scry log` asks for a file's history with `--history` and
    /// `--path`.
    @Test func readsTheHistoryRequest() {
        let arguments = ["Scry", "--history", "/r", "--path", "src/a.swift"]
        #expect(RootViewModel.historyRequest(in: arguments) == RootViewModel.HistoryRequest(root: "/r", path: "src/a.swift"))
        #expect(RootViewModel.historyRequest(in: ["Scry", "--history", "/r"]) == nil)
        #expect(RootViewModel.historyRequest(in: ["Scry", "--diff", "/r", "--path", "a"]) == nil)
    }

    /// Entering a file's history and opening a whole commit are places
    /// the window goes back from and forward to, each as it was left; a
    /// new place drops those Forward led to.
    @Test func backAndForwardMoveBetweenThePlacesShown() async {
        let root = RootViewModel()
        root.open(changes: FakeChanges(files: [FakeChanges.file(0, "a.swift")]))
        let changes = try? #require(root.changes)
        #expect(!root.canGoBack && !root.canGoForward)
        #expect(root.backHelp == "Back")

        root.open(history: FakeHistory(points: [FakeHistory.commit(0, "Start", head: true)]))
        let history = try? #require(root.history)
        #expect(root.changes == nil)
        #expect(root.canGoBack && !root.canGoForward)
        #expect(root.backHelp == "Back to Not committed")

        let commit = FakeChanges(files: [])
        commit.title = "Commit 9f8e7d6"
        root.open(commit: commit)
        #expect(root.changes?.header.title == "Commit 9f8e7d6")
        #expect(root.history == nil)
        #expect(root.backHelp == "Back to the history of SyncEngine.swift")

        root.goBack()
        #expect(root.history === history)
        #expect(root.forwardHelp == "Forward to Commit 9f8e7d6")
        root.goBack()
        #expect(root.changes === changes)
        root.goBack()
        #expect(root.changes === changes)
        root.goForward()
        #expect(root.history === history)
        // History's "Changed files" is Back.
        history?.showChangedFiles()
        #expect(root.changes === changes)
        // A new place from here: the history and the commit are dropped.
        root.open(history: FakeHistory(points: []))
        #expect(root.trail.count == 2)
        #expect(!root.canGoForward)
        // Another change replaces every place.
        root.open(changes: FakeChanges(files: []))
        #expect(root.trail.count == 1 && !root.canGoBack)
    }

    /// The History key asks for the selected file's history; a window on
    /// git difftool's copies, or on a history's own file, has none.
    @Test func theSelectedFilesHistoryIsAsked() async {
        var asked: [String] = []
        let source = FakeChanges(files: [FakeChanges.file(0, "a.swift"), FakeChanges.file(1, "b.swift")])
        let changes = ChangesViewModel(source: source, defaults: .isolated, openHistory: { asked.append($0.path) })
        #expect(!changes.canShowHistory)
        await changes.load()
        #expect(changes.canShowHistory)
        #expect(changes.toolbarHints.contains(.history))
        await changes.nextFile()
        changes.showHistory()
        #expect(asked == ["b.swift"])

        let reduced = FakeChanges(files: [FakeChanges.file(0, "a.swift")])
        reduced.hasHistory = false
        let none = ChangesViewModel(source: reduced, defaults: .isolated, openHistory: { asked.append($0.path) })
        await none.load()
        #expect(!none.canShowHistory)
        none.showHistory()
        #expect(asked == ["b.swift"])
    }

    /// `git difftool` hands two files, or two folders, each after its
    /// option, with the file's name in the repository when there is one;
    /// the window then shows them, with nothing to resolve or to go back
    /// to.
    @Test func readsGitDifftoolsFiles() async throws {
        let arguments = ["Scry", "--difftool", "--old", "/tmp/a", "--new", "/r/src/a.swift", "--name", "src/a.swift"]
        #expect(RootViewModel.difftoolFiles(in: arguments) == RootViewModel.DifftoolFiles(
            old: "/tmp/a", new: "/r/src/a.swift", name: "src/a.swift"
        ))
        #expect(RootViewModel.difftoolFiles(in: ["Scry", "--difftool", "--old", "/l/", "--new", "/r/"])?.name == "")
        #expect(RootViewModel.difftoolFiles(in: ["Scry", "--old", "/a", "--new", "/b"]) == nil)
        #expect(RootViewModel.difftoolFiles(in: ["Scry", "--difftool", "--old", "/a"]) == nil)
        // Its place among the files git opens in turn, when it gives one.
        let queued = ["Scry", "--difftool", "--old", "/a", "--new", "/b", "--place", "2", "--of", "13"]
        #expect(RootViewModel.difftoolFiles(in: queued)?.queue == ChangesViewModel.Queue(place: 2, total: 13))
        for (place, total) in [("1", "1"), ("0", "3"), ("4", "3"), ("x", "3")] {
            let arguments = ["Scry", "--difftool", "--old", "/a", "--new", "/b", "--place", place, "--of", total]
            #expect(RootViewModel.difftoolFiles(in: arguments)?.queue == nil)
        }

        let directory = FileManager.default.temporaryDirectory.appending(path: "ScryTests-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        let old = directory.appending(path: "old.swift")
        let new = directory.appending(path: "new.swift")
        try "let a = 1\n".write(to: old, atomically: true, encoding: .utf8)
        try "let a = 2\n".write(to: new, atomically: true, encoding: .utf8)

        let root = RootViewModel()
        await root.openLaunchFile(arguments: [
            "Scry", "--difftool", "--old", old.path, "--new", new.path, "--name", "src/a.swift",
        ])
        let changes = try #require(root.changes)
        #expect(changes.header.reduced)
        #expect(changes.header.singleFile)
        await changes.load()
        #expect(changes.selected?.path == "src/a.swift")
        #expect(changes.status == "1 change · +1 −1")
        #expect(root.exitStatus == 0)
        #expect(changes.queue == nil)
        #expect(!root.finished)
    }

    /// Among several files, Next File closes the window and answers 0, so
    /// git opens the next one; Stop answers 1, and git opens none.
    @Test func leavesGitDifftoolsFilesByNextOrStop() async throws {
        let directory = FileManager.default.temporaryDirectory.appending(path: UUID().uuidString)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        let old = directory.appending(path: "old.swift")
        let new = directory.appending(path: "new.swift")
        try "let a = 1\n".write(to: old, atomically: true, encoding: .utf8)
        try "let a = 2\n".write(to: new, atomically: true, encoding: .utf8)
        let arguments = [
            "Scry", "--difftool", "--old", old.path, "--new", new.path, "--name", "a.swift", "--place", "1", "--of", "3",
        ]

        let next = RootViewModel()
        await next.openLaunchFile(arguments: arguments)
        #expect(next.changes?.queueLabel == "File 1 of 3")
        next.changes?.nextInQueue()
        #expect(next.finished)
        #expect(next.exitStatus == 0)

        let stop = RootViewModel()
        await stop.openLaunchFile(arguments: arguments)
        stop.changes?.stopQueue()
        #expect(stop.finished)
        #expect(stop.exitStatus == 1)
    }

    /// A repository opens on what waits in it: the overview while
    /// conflicts are left, otherwise what is not committed; and it is
    /// remembered among the recent ones, most recent first.
    @Test func opensARepositoryOnWhatWaitsInIt() async throws {
        let directory = FileManager.default.temporaryDirectory.appending(path: "ScryTests-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        let git = Process()
        git.executableURL = URL(fileURLWithPath: "/usr/bin/env")
        git.arguments = ["git", "-C", directory.path, "init", "--quiet"]
        try git.run()
        git.waitUntilExit()
        try #require(git.terminationStatus == 0)
        try "x\n".write(to: directory.appending(path: "a.txt"), atomically: true, encoding: .utf8)

        let defaults = UserDefaults.isolated
        let changes = RootViewModel(defaults: defaults, waiting: { _ in .notCommitted(files: 1) })
        await changes.openRepository(directory.path)
        #expect(changes.changes != nil)
        #expect(changes.overview == nil)
        await changes.changes?.load()
        #expect(changes.changes?.files.map(\.path) == ["a.txt"])

        let resolving = RootViewModel(
            defaults: defaults, waiting: { _ in .resolving(headline: "Merging", unresolved: 1) }
        )
        await resolving.openRepository(directory.path)
        #expect(resolving.overview != nil)
        #expect(resolving.changes == nil)

        let recent = RecentRepositories(defaults: defaults).paths
        #expect(recent.count == 1)
        #expect(recent.first.map { URL(fileURLWithPath: $0).lastPathComponent } == directory.lastPathComponent)
    }
}
