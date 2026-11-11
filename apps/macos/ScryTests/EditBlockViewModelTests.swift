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

    /// In a file with Windows line endings, the draft breaks lines with
    /// `\n` so it splits into lines (Swift sees `\r\n` as one character)
    /// and Return matches it; applying puts `\r\n` back, so the written
    /// file does not mix line endings.
    @Test func keepsWindowsLineEndings() throws {
        let file = try ConflictFile(text: ConflictFile.crlfText)
        let editor = try file.editor()
        editor.take("theirs")
        editor.startEditing(1)
        let block = try #require(editor.editBlock)

        #expect(block.draft == "let c = 1\n")
        #expect(block.lineEnding == "\r\n")
        block.draft = "let c = 3\nlet d = 4\n"
        #expect(block.visibleLines == 4)
        block.apply()

        #expect(editor.inspector.editedCard?.candidate.preview == ["let c = 3", "let d = 4"])
        editor.markResolved()
        #expect(file.contents == "let a = 2\r\nlet b = 0\r\nlet c = 3\r\nlet d = 4\r\n")
    }
}
