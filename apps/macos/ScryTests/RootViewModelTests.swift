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
}
