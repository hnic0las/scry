import Testing
@testable import Scry

@MainActor
struct EditBlockViewModelTests {
    /// An unresolved conflict opens with ours, and a resolved one with its
    /// resolution, so the user edits from what the result shows.
    @Test func startsFromTheCurrentText() throws {
        let editor = try ConflictFile().editor()

        editor.startEditing(0)
        #expect(editor.editBlock?.draft == "let a = 1\n")
        editor.editBlock?.cancel()

        editor.take("theirs")
        editor.startEditing(0)
        #expect(editor.editBlock?.draft == "let a = 2\n")
    }

    /// The editor is at least three rows high and at most twenty, with a
    /// spare row to type into.
    @Test func sizesToItsText() throws {
        let editor = try ConflictFile().editor()
        editor.startEditing(0)
        let block = try #require(editor.editBlock)

        #expect(block.visibleLines == 3)
        block.draft = Array(repeating: "x", count: 5).joined(separator: "\n")
        #expect(block.visibleLines == 6)
        block.draft = Array(repeating: "x", count: 40).joined(separator: "\n")
        #expect(block.visibleLines == 20)
    }

    /// Return and the colours come from the core with the conflict's file
    /// around the draft: a line break after `{` indents one unit deeper.
    @Test func asksTheCoreForIndentationAndColours() throws {
        let editor = try ConflictFile().editor()
        editor.startEditing(0)
        let block = try #require(editor.editBlock)

        let newline = block.newline(before: "func f() {", after: "")
        #expect(newline.text.hasPrefix("func f() {\n    "))
        #expect(!block.highlight("let a = 1\n").joined().isEmpty)
    }
}
