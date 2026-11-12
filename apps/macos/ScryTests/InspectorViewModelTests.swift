import Testing
@testable import Scry

@MainActor
struct InspectorViewModelTests {
    /// The inspector follows the focused conflict and marks its chosen
    /// candidate, so the cards always match the lens row.
    @Test func showsTheFocusedConflict() throws {
        let editor = try ConflictFile().editor()
        let inspector = editor.inspector

        #expect(inspector.title == "Conflict 1 of 2")
        #expect(inspector.cards.allSatisfy { !$0.selected })
        inspector.choose("theirs")
        #expect(inspector.cards.filter(\.selected).map(\.id) == ["theirs"])

        editor.next()
        #expect(inspector.title == "Conflict 2 of 2")
        #expect(inspector.cards.allSatisfy { !$0.selected })
    }

    /// Edit by Hand opens the editor under the inspector's conflict, owned
    /// by the editor screen, and is unavailable while an edit is open.
    @Test func editByHandOpensTheEditor() throws {
        let editor = try ConflictFile().editor()
        editor.next()

        editor.inspector.editByHand()

        #expect(editor.editBlock?.conflict == 1)
        #expect(!editor.inspector.canEditByHand)
    }

    /// A conflict resolved by hand shows its text as a selected card with
    /// the user's provenance, so it can be reopened from the inspector.
    @Test func showsTheEditedText() throws {
        let editor = try ConflictFile().editor()
        #expect(editor.inspector.editedCard == nil)

        editor.startEditing()
        editor.editBlock?.draft = "let a = 42\n"
        editor.editBlock?.apply()

        let edited = try #require(editor.inspector.editedCard)
        #expect(edited.selected)
        #expect(edited.candidate.provenance == .human)
        #expect(edited.candidate.preview == ["let a = 42"])
    }

    /// While a conflict is edited by hand, the cards cannot choose: a
    /// click would change the resolution behind the open editor, and
    /// Apply would then overwrite it.
    @Test func cannotChooseWhileEditing() throws {
        let editor = try ConflictFile().editor()
        editor.take("ours")

        editor.startEditing()
        #expect(!editor.inspector.canChoose)
        editor.inspector.choose("theirs")
        #expect(editor.chosen(0) == "ours")

        editor.editBlock?.cancel()
        #expect(editor.inspector.canChoose)
    }

    /// The candidates are a radio group: ↑ and ↓ move the choice one card
    /// at a time and stop at the ends, so a held key never wraps round to
    /// a resolution the user did not see; nothing moves while editing.
    @Test func arrowsMoveTheChoiceThroughTheCards() throws {
        let editor = try ConflictFile().editor()
        let inspector = editor.inspector
        let ids = inspector.cards.map(\.id)
        #expect(ids.count > 1)

        inspector.chooseAdjacent(1)
        #expect(editor.chosen(0) == ids.first)
        inspector.chooseAdjacent(-1)
        #expect(editor.chosen(0) == ids.first)
        inspector.chooseAdjacent(1)
        #expect(editor.chosen(0) == ids[1])

        editor.take(ids.last ?? "")
        inspector.chooseAdjacent(1)
        #expect(editor.chosen(0) == ids.last)

        editor.startEditing()
        inspector.chooseAdjacent(-1)
        #expect(editor.chosen(0) == ids.last)
    }
}
