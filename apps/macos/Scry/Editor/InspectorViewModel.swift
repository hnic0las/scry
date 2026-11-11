import Foundation
import Observation

/// S3: the focused conflict, its classification and its candidates.
@MainActor
@Observable
final class InspectorViewModel {
    /// Opens a conflict's editor, which the editor screen owns.
    @ObservationIgnored var onEditByHand: (Int) -> Void = { _ in }

    @ObservationIgnored private let model: MergeModel

    init(model: MergeModel) {
        self.model = model
    }

    /// One candidate's card.
    struct Card: Identifiable {
        let candidate: CandidateInfo
        let selected: Bool
        var id: String { candidate.id }
    }

    var conflict: ConflictInfo? { model.conflict(model.current) }

    var title: String {
        guard let conflict else { return "" }
        return "Conflict \(conflict.number) of \(model.conflicts.count)"
    }

    var line: String {
        conflict.map { "line \($0.startLine)" } ?? ""
    }

    var language: String? { model.status.language }

    var cards: [Card] {
        guard let conflict else { return [] }
        return conflict.candidates.map { Card(candidate: $0, selected: conflict.chosen == $0.id) }
    }

    /// The text written by hand, when it resolves the conflict.
    var editedCard: Card? {
        guard let chosen = conflict?.chosenCandidate, chosen.id == CandidateInfo.editedID else { return nil }
        return Card(candidate: chosen, selected: true)
    }

    /// One conflict is edited at a time.
    var canEditByHand: Bool { conflict != nil && model.editing == nil }

    /// While a conflict is edited by hand, a card would change the
    /// resolution behind the open editor.
    var canChoose: Bool { conflict != nil && model.editing == nil }

    func choose(_ candidate: String) {
        guard let conflict, canChoose else { return }
        model.choose(candidate, conflict: conflict.index)
    }

    func editByHand() {
        guard let conflict, model.editing == nil else { return }
        onEditByHand(conflict.index)
    }
}
