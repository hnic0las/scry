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
}
