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
            repeating: [LensBadge(text: "Unresolved", role: .state(state: .unresolved))],
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

        let badges = editor.lensRows.map { $0.badges.map(\.role) }
        #expect(badges == [[.provenance(provenance: .git)], [.state(state: .unresolved)]])
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

    /// The hint strip lists only the shortcuts without a button that work
    /// now: undo once there is a choice to undo, nothing while editing by
    /// hand, whose own buttons carry their keys.
    @Test func hintStripShowsWhatWorksNow() throws {
        let editor = try ConflictFile().editor()
        #expect(editor.hintStrip.map(\.id) == ["editByHand", "takeBase", "takeNone", "keyboardShortcuts"])
        editor.take("theirs")
        #expect(editor.hintStrip.map(\.id).contains("undo"))
        #expect(!editor.hintStrip.map(\.id).contains("redo"))
        editor.startEditing()
        #expect(editor.hintStrip.isEmpty)
    }

    /// The row under the toolbar names only the toolbar's shortcuts that
    /// work now, in the toolbar's order: no "previous" on the first
    /// conflict, no sides while editing by hand.
    @Test func toolbarHintsFollowTheButtons() throws {
        let editor = try ConflictFile().editor()
        #expect(editor.toolbarHints.map(\.id).first == "nextConflict")
        #expect(!editor.toolbarHints.map(\.id).contains("previousConflict"))
        #expect(editor.toolbarHints.map(\.id).contains("takeOurs"))
        editor.startEditing()
        #expect(!editor.toolbarHints.map(\.id).contains("takeOurs"))
        #expect(editor.toolbarHints.map(\.id).contains("toggleInspector"))
    }

    /// The panes scroll as far as the widest line as drawn: tabs reach
    /// their stop and wide characters count double, or the end of such a
    /// line could not be scrolled to.
    @Test func measuresLinesAsDrawn() throws {
        let editor = try ConflictFile(text: ConflictFile.text + "\t\tlet 名前 = 1\n").editor()

        // Eight spaces, "let ", two wide characters, " = 1".
        #expect(editor.longestLine == 8 + 4 + 4 + 4)
    }

    /// Taking the side already taken changes nothing: one undo takes the
    /// choice back, rather than an undo that seems to do nothing.
    @Test func takingTheSameSideTwiceIsOneStep() throws {
        let editor = try ConflictFile().editor()

        editor.take("theirs")
        editor.take("theirs")
        editor.undo()

        #expect(editor.chosen(0) == nil)
        #expect(!editor.canUndo)
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

    /// The edit block follows the model's conflict being edited, the one
    /// source of truth, and keeps its draft while the edit lasts.
    @Test func editBlockFollowsTheModel() throws {
        let editor = try ConflictFile().editor()
        #expect(editor.editBlock == nil)

        editor.startEditing(1)
        editor.editBlock?.draft = "let c = 9\n"
        #expect(editor.editBlock?.conflict == 1)
        #expect(editor.editBlock?.draft == "let c = 9\n")
        #expect(editor.inspector.canEditByHand == false)

        editor.editBlock?.cancel()
        #expect(editor.editBlock == nil)
        #expect(!editor.isEditing)
        #expect(editor.inspector.canEditByHand)
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
        #expect(editor.lensRows[0].badges == [LensBadge(text: "Edited by you", role: .provenance(provenance: .human))])
        #expect(editor.chosen(0) == "edited")
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
        #expect(editor.lensRows[1].badges.map(\.role) == [.provenance(provenance: .human), .severity(severity: .error)])
        editor.markResolved()
        #expect(editor.confirmingInvalidWrite)
        #expect(!editor.written)
        #expect(file.contents == ConflictFile.text)

        editor.writeAnyway()
        #expect(editor.written)
        #expect(file.contents == "let a = 2\nlet b = 0\nfunc f( {\n")
    }

    /// A file changed on disk after it was opened (resolved in another
    /// editor, an aborted rebase) is not written over silently: Mark as
    /// Resolved asks, and only Overwrite replaces it.
    @Test func changedFileAsksBeforeOverwriting() throws {
        let file = try ConflictFile()
        let editor = try file.editor()
        editor.take("theirs")
        editor.next()
        editor.take("ours")
        try "resolved elsewhere\n".write(to: file.url, atomically: true, encoding: .utf8)

        editor.markResolved()
        #expect(editor.confirmingOverwrite)
        #expect(!editor.written)
        #expect(file.contents == "resolved elsewhere\n")

        editor.overwrite()
        #expect(editor.written)
        #expect(file.contents == "let a = 2\nlet b = 0\nlet c = 1\n")
    }

    /// The rows are kept between renders and built again whenever the
    /// model changes, whichever screen changed it: a choice from the
    /// inspector, undo, focus or editing must all show in the lens rows.
    @Test func rowsFollowEveryChangeOfTheModel() throws {
        let editor = try ConflictFile().editor()
        #expect(editor.lensRows.map(\.focused) == [true, false])

        editor.inspector.choose("theirs")
        #expect(editor.lensRows[0].badges.map(\.role) == [.provenance(provenance: .git)])

        editor.undo()
        #expect(editor.lensRows[0].badges.map(\.role) == [.state(state: .unresolved)])

        editor.next()
        #expect(editor.lensRows.map(\.focused) == [false, true])

        editor.startEditing()
        #expect(editor.lensRows.map(\.editing) == [false, true])
        editor.editBlock?.cancel()
        #expect(editor.lensRows.map(\.editing) == [false, false])
    }
}
