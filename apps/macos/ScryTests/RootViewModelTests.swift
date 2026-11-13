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
}
