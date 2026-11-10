import Foundation
import Testing
@testable import Scry

@MainActor
struct MergeEditorViewModelTests {
    /// A file just opened shows every conflict unresolved, focuses the
    /// first, and cannot be written yet: the toolbar must not offer to write
    /// a file that still has markers.
    @Test func opensUnresolved() throws {
        let editor = try ConflictFile().editor()

        #expect(editor.title == "Sample.swift")
        #expect(editor.counter == "Conflict 1 of 2")
        #expect(editor.lensRows.map(\.badges) == Array(
            repeating: [.init(text: "Unresolved", kind: .unresolved)],
            count: 2
        ))
        #expect(editor.lensRows.map(\.focused) == [true, false])
        #expect(editor.resultHeader.detail.hasPrefix("0 of 2 conflicts resolved"))
        #expect(!editor.canMarkResolved)
        #expect(!editor.canGoToPrevious)
        #expect(!editor.canUndo)
    }

    /// Next and previous move the focus between conflicts and stop at
    /// both ends, so the toolbar arrows never focus a missing conflict.
    @Test func movesBetweenConflicts() throws {
        let editor = try ConflictFile().editor()

        editor.next()
        #expect(editor.currentConflict == 1)
        #expect(editor.counter == "Conflict 2 of 2")
        #expect(!editor.canGoToNext)
        editor.next()
        #expect(editor.currentConflict == 1)
        editor.previous()
        editor.previous()
        #expect(editor.currentConflict == 0)
    }

    /// Taking a side resolves the focused conflict only, and its lens row
    /// says which side with git's provenance.
    @Test func takeResolvesTheFocusedConflict() throws {
        let editor = try ConflictFile().editor()

        editor.take("theirs")

        let badges = editor.lensRows.map { $0.badges.map(\.kind) }
        #expect(badges == [[.resolved(.git)], [.unresolved]])
        #expect(editor.resultHeader.detail.hasPrefix("1 of 2 conflicts resolved"))
        #expect(editor.canUndo)
    }

    /// Both, pressed again, swaps the order: the shortcut ⌥⌘3 cycles
    /// between ours first and theirs first.
    @Test func takeBothAlternatesTheOrder() throws {
        let editor = try ConflictFile().editor()

        editor.takeBoth()
        #expect(editor.chosen(0) == "ours-then-theirs")
        editor.takeBoth()
        #expect(editor.chosen(0) == "theirs-then-ours")
    }

    /// Undo and redo go through the core's history, so a choice can be
    /// taken back from the editor.
    @Test func undoesAndRedoesChoices() throws {
        let editor = try ConflictFile().editor()

        editor.take("theirs")
        editor.undo()
        #expect(editor.chosen(0) == nil)
        #expect(editor.canRedo)
        editor.redo()
        #expect(editor.chosen(0) == "theirs")
    }

    /// While a conflict is edited by hand, the editor's own shortcuts are
    /// off, so ↩, ⌘Z and ⌘↩ go to the text being typed, and a side cannot
    /// be taken behind the user's back.
    @Test func editingBlocksTheOtherActions() throws {
        let editor = try ConflictFile().editor()
        editor.take("ours")
        editor.next()
        editor.take("ours")

        editor.startEditing()

        #expect(editor.isEditing)
        #expect(editor.editBlock?.conflict == 1)
        #expect(editor.lensRows.map(\.editing) == [false, true])
        #expect(!editor.canUndo)
        #expect(!editor.canMarkResolved)
        editor.take("theirs")
        #expect(editor.chosen(1) == "ours")
    }

    /// Cancelling an edit closes the editor and leaves the resolution as
    /// it was.
    @Test func cancellingAnEditKeepsTheResolution() throws {
        let editor = try ConflictFile().editor()
        editor.take("theirs")

        editor.startEditing()
        editor.editBlock?.draft = "let a = 42\n"
        editor.editBlock?.cancel()

        #expect(editor.editBlock == nil)
        #expect(editor.chosen(0) == "theirs")
    }

    /// Applying an edit resolves the conflict with the user's text, marked
    /// as written by hand, and closes the editor.
    @Test func applyingAnEditResolvesByHand() throws {
        let editor = try ConflictFile().editor()

        editor.startEditing()
        editor.editBlock?.draft = "let a = 42\n"
        editor.editBlock?.apply()

        #expect(editor.editBlock == nil)
        #expect(editor.lensRows[0].badges == [.init(text: "Edited by you", kind: .resolved(.human))])
        #expect(editor.chosen(0) == CandidateInfo.editedID)
    }

    /// Mark as Resolved writes the merged file once every conflict is
    /// resolved, without markers, and the status bar says so.
    @Test func markResolvedWritesTheFile() throws {
        let file = try ConflictFile()
        let editor = try file.editor()
        editor.take("theirs")
        editor.next()
        editor.take("ours")

        #expect(editor.verification == .parses("Swift"))
        editor.markResolved()

        #expect(editor.written)
        #expect(file.contents == "let a = 2\nlet b = 0\nlet c = 1\n")
    }

    /// A result that does not parse is written only once the user
    /// confirms, since it would break the build.
    @Test func invalidResultAsksBeforeWriting() throws {
        let file = try ConflictFile()
        let editor = try file.editor()
        editor.take("theirs")
        editor.startEditing(1)
        editor.editBlock?.draft = "func f( {\n"
        editor.editBlock?.apply()

        guard case .doesNotParse = editor.verification else {
            Issue.record("expected the result not to parse")
            return
        }
        #expect(editor.lensRows[1].badges.map(\.kind) == [.resolved(.human), .doesNotParse])
        editor.markResolved()
        #expect(editor.confirmingInvalidWrite)
        #expect(!editor.written)
        #expect(file.contents == ConflictFile.text)

        editor.writeAnyway()
        #expect(editor.written)
        #expect(file.contents == "let a = 2\nlet b = 0\nfunc f( {\n")
    }
}
