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

    /// A file resolved outside Scry opens for review: a conflict with no
    /// record says nobody gave a reason; the agent's own text shows who
    /// resolved it, why and what it left out, as a card of its own.
    /// Marking it reviewed keeps the file as it is, so leaving asks
    /// nothing; reopening brings the markers back, and undo the resolution.
    @Test func reviewsAResolutionMadeOutsideScry() throws {
        let editor = try #require(ReviewSample.editor())
        let inspector = editor.inspector

        let first = try #require(inspector.review)
        #expect(first.by == "Resolved outside Scry")
        #expect(first.reason == "No reason given (resolved outside Scry).")
        #expect(!first.hasReason && !first.reviewed)
        #expect(inspector.cards.filter(\.selected).map(\.id) == ["theirs"])
        #expect(inspector.editedCard == nil)

        editor.next()
        let agent = try #require(inspector.review)
        #expect(agent.by == "Resolved by claude-code 2.1")
        #expect(agent.provenance == .agent)
        #expect(agent.reason == ReviewSample.reason)
        #expect(agent.dropped == "Left out: nothing")
        #expect(inspector.editedCard?.id == "outside")
        #expect(editor.rows.contains { row in
            if case .code(let code) = row { return code.conflict == 1 && code.resultUnreviewed }
            return false
        })

        inspector.markReviewed()
        #expect(inspector.review?.reviewed == true)
        #expect(inspector.review?.status == "Reviewed")
        #expect(!inspector.canMarkReviewed)
        #expect(!editor.hasUnwrittenWork)

        inspector.reopen()
        #expect(inspector.review == nil)
        #expect(inspector.conflict?.chosen == nil)
        #expect(editor.hasUnwrittenWork)
        editor.undo()
        #expect(inspector.review?.reviewed == true)
    }
}
