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

    /// Only the focused conflict's candidates are checked by parsing, so a
    /// file with thousands of conflicts opens at once; moving the focus
    /// checks the next one, and moves the focus mark between the two lens
    /// rows without building the rows again.
    @Test func checksTheFocusedConflictOnly() throws {
        let model = try MergeModel(path: ConflictFile().url.path)
        let editor = MergeEditorViewModel(model: model, defaults: .isolated)

        #expect(model.conflicts.map(\.checked) == [true, false])
        #expect(model.conflicts[0].candidates.allSatisfy { $0.parse == .parses })
        let rows = editor.rows.count
        editor.next()
        #expect(model.conflicts.map(\.checked) == [true, true])
        #expect(editor.lensRows.map(\.focused) == [false, true])
        editor.previous()
        #expect(editor.lensRows.map(\.focused) == [true, false])
        #expect(editor.rows.count == rows)
    }

    /// With the same change on both sides there is one card, ours; the
    /// theirs button still resolves the conflict with it, rather than do
    /// nothing.
    @Test func theirsTakesTheSameChange() throws {
        let file = try ConflictFile(text: "<<<<<<< HEAD\nlet a = 1\n||||||| base\nlet a = 0\n=======\nlet a = 1\n>>>>>>> other\n")
        let editor = try file.editor()

        editor.take("theirs")

        #expect(editor.canMarkResolved)
        #expect(editor.lensRows.first?.badges.first?.text == "Ours")
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
        #expect(editor.hintStrip.map(\.id) == ["takeOurs", "takeTheirs", "takeBoth", "editByHand", "takeBase", "takeNone", "keyboardShortcuts"])
        editor.take("theirs")
        #expect(editor.hintStrip.map(\.id).contains("undo"))
        #expect(!editor.hintStrip.map(\.id).contains("redo"))
        editor.startEditing()
        #expect(editor.hintStrip.isEmpty)
    }

    /// The row under the toolbar names only the toolbar's shortcuts that
    /// work now, in the toolbar's order: no "previous" on the first
    /// conflict, and no sides, which have no toolbar button.
    @Test func toolbarHintsFollowTheButtons() throws {
        let editor = try ConflictFile().editor()
        #expect(editor.toolbarHints.map(\.id).first == "nextConflict")
        #expect(!editor.toolbarHints.map(\.id).contains("previousConflict"))
        #expect(!editor.toolbarHints.map(\.id).contains("takeOurs"))
        #expect(editor.toolbarHints.map(\.id).contains("toggleInspector"))
    }

    /// The layout choice is the user's, kept for the next file; a narrow
    /// window stacks the panes without changing it, so widening the window
    /// brings the choice back, and the switch is unavailable meanwhile.
    @Test func layoutIsRememberedAndForcedWhenNarrow() throws {
        let defaults = UserDefaults(suiteName: "ScryTests.\(UUID().uuidString)")!
        let editor = try ConflictFile().editor(defaults: defaults)
        #expect(editor.layout == .columns)
        #expect(editor.toolbarHints.map(\.id).contains("switchLayout"))

        editor.switchLayout()
        #expect(editor.layout == .stacked)
        #expect(try ConflictFile().editor(defaults: defaults).layout == .stacked)
        editor.switchLayout()
        #expect(editor.layout == .columns)

        editor.windowWidthChanged(900)
        #expect(editor.layout == .stacked)
        #expect(!editor.canSwitchLayout)
        #expect(editor.layoutHelp == "Needs a wider window")
        #expect(!editor.toolbarHints.map(\.id).contains("switchLayout"))
        editor.switchLayout()
        #expect(editor.preferredLayout == .columns)

        editor.windowWidthChanged(1440)
        #expect(editor.layout == .columns)
    }

    /// Stacked, a conflict scrolls to 4 rows below the top, so its lines
    /// come with some context in both areas; in columns, to its lens row.
    @Test func stackedScrollsAConflictBelowTheTop() throws {
        let editor = try ConflictFile(text: String(repeating: "let x = 0\n", count: 10) + ConflictFile.text).editor()
        let last = editor.lensRows.count - 1
        let lens = try #require(editor.lensRowIndex(of: last))
        #expect(editor.scrollTarget(for: last) == lens)
        editor.setLayout(.stacked)
        #expect(editor.scrollTarget(for: last) == lens - 4)
    }

    /// The panes scroll as far as the widest line as drawn: tabs reach
    /// their stop and wide characters count double, or the end of such a
    /// line could not be scrolled to.
    @Test func measuresLinesAsDrawn() throws {
        let editor = try ConflictFile(text: ConflictFile.text + "\t\tlet 名前 = 1\n").editor()

        // Eight spaces, "let ", two wide characters, " = 1".
        #expect(editor.longestLine == 8 + 4 + 4 + 4)
    }

    /// A lens row's take button acts on its own conflict, not on the
    /// focused one, and does nothing while a conflict is edited by hand.
    @Test func lensButtonsTakeForTheirConflict() throws {
        let editor = try ConflictFile().editor()
        #expect(editor.currentConflict == 0)

        editor.take("theirs", in: 1)
        #expect(editor.currentConflict == 1)
        #expect(editor.chosen(1) == "theirs")
        #expect(editor.chosen(0) == nil)

        editor.takeBoth(in: 0)
        #expect(editor.chosen(0) == "ours-then-theirs")

        editor.startEditing()
        editor.take("ours", in: 1)
        #expect(editor.chosen(1) == "theirs")
    }

    /// ⌥⌘B adds each conflict's base lines to the rows, above the block
    /// in the result, and hides them again; a file without a base cannot
    /// show one, so the toggle is disabled rather than doing nothing.
    @Test func togglesTheBase() throws {
        let editor = try ConflictFile().editor()
        #expect(editor.canShowBase)
        let count = editor.rows.count
        editor.toggleBase()
        #expect(editor.showsBase)
        #expect(editor.rows.count > count)
        let lens = try #require(editor.lensRowIndex(of: 0))
        guard case .code(let row) = editor.rows[lens + 1] else {
            Issue.record("expected a base row under the lens row")
            return
        }
        #expect(row.result.kind == .base)
        #expect(row.ours.kind == .spacer)
        editor.toggleBase()
        #expect(editor.rows.count == count)

        let noBase = try ConflictFile(text: "a\n<<<<<<< ours\no\n=======\nt\n>>>>>>> theirs\nz\n").editor()
        #expect(!noBase.canShowBase)
        noBase.toggleBase()
        #expect(!noBase.showsBase)
    }

    /// The lens row's ••• menu lists exactly the candidates the core
    /// offers, in its order, with the direct keys of those that have one,
    /// so it never offers a resolution the inspector does not.
    @Test func lensMenuListsTheCoreCandidates() throws {
        let editor = try ConflictFile().editor()
        guard case .lens(let lens) = editor.rows[try #require(editor.lensRowIndex(of: 0))] else {
            Issue.record("expected a lens row")
            return
        }
        #expect(lens.menu.map(\.id) == editor.inspector.cards.map(\.id))
        #expect(lens.menu.first { $0.id == "ours" }?.shortcut == .takeOurs)
        #expect(lens.menu.first { $0.id == "theirs-then-ours" }?.shortcut == nil)
    }

    /// Edit by Hand from a lens row opens that row's conflict, not the
    /// focused one.
    @Test func lensMenuEditsItsOwnConflict() throws {
        let editor = try ConflictFile().editor()
        editor.editByHand(in: 1)
        #expect(editor.editBlock?.conflict == 1)
        #expect(editor.currentConflict == 1)
    }

    /// The › ‹ buttons sit on each conflict's first line, under any base
    /// lines, once per conflict, and are disabled while editing by hand.
    @Test func connectorButtonsStartEachBlock() throws {
        let editor = try ConflictFile().editor()
        editor.toggleBase()
        let starts = editor.rows.compactMap { row -> MergeEditorViewModel.CodeRowState? in
            guard case .code(let code) = row, code.startsBlock else { return nil }
            return code
        }
        #expect(starts.compactMap(\.conflict) == [0, 1])
        #expect(starts.filter { $0.result.kind == .base }.isEmpty)
        #expect(starts.filter { !$0.canTake }.isEmpty)

        editor.startEditing()
        let editing = editor.rows.compactMap { row -> Bool? in
            guard case .code(let code) = row, code.startsBlock else { return nil }
            return code.canTake
        }
        #expect(editing == [false, false])
    }

    /// The ruler has one mark per conflict, in file order, with the rows
    /// above it by height, so the view can place it where its lens row is
    /// drawn; it marks the focused one and follows each conflict's state.
    @Test func rulerMarksFollowTheConflicts() throws {
        let editor = try ConflictFile().editor()
        let ruler = editor.ruler
        var marks = ruler.marks
        #expect(marks.map(\.conflict) == [0, 1])
        #expect(marks.map(\.focused) == [true, false])
        #expect(marks.map(\.lensRowsBefore) == [0, 1])
        #expect(marks[0].codeRowsBefore < marks[1].codeRowsBefore)
        #expect(ruler.lensRows == 2)
        #expect(ruler.codeRows + ruler.lensRows == editor.rows.count)
        #expect(marks[1].label == "Conflict 2 · Unresolved")

        editor.next()
        editor.take("theirs")
        marks = editor.ruler.marks
        #expect(marks.map(\.focused) == [false, true])
        #expect(marks[1].state != .unresolved)
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
