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
        return conflict.candidates.map { Card(candidate: $0, selected: conflict.chosen?.id == $0.id) }
    }

    /// A conflict resolved outside Scry, as the review shows it.
    struct Review: Equatable {
        /// "Resolved by claude-code 2.1", or "Resolved outside Scry".
        let by: String
        let provenance: Provenance
        /// The reason the agent gave, or that none was.
        let reason: String
        let hasReason: Bool
        /// For text of the agent's own: what of either side it left out.
        let dropped: String?
        let reviewed: Bool
        /// "Not reviewed yet", or "Reviewed".
        let status: String
    }

    /// What the review of this conflict shows; `nil` unless it was
    /// resolved outside Scry and the user has not changed it since.
    var review: Review? {
        guard let outside = conflict?.outside else { return nil }
        let reason = outside.reason.map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }
            .flatMap { $0.isEmpty ? nil : $0 }
        return Review(
            by: outside.by.map { "Resolved by \($0)" } ?? "Resolved outside Scry",
            provenance: outside.by == nil ? .human : .agent,
            reason: reason ?? (outside.by == nil
                ? "No reason given (resolved outside Scry)."
                : "No reason given."),
            hasReason: reason != nil,
            dropped: outside.dropped.flatMap { $0.isEmpty ? nil : "Left out: \($0)" },
            reviewed: outside.reviewed,
            status: outside.reviewed ? "Reviewed" : "Not reviewed yet"
        )
    }

    /// The text the conflict holds when it is no candidate's: written by
    /// hand in Scry, or outside it.
    var editedCard: Card? {
        guard let chosen = conflict?.chosen,
              conflict?.outside != nil ? chosen.id == Self.outsideId : chosen.provenance == .human
        else { return nil }
        return Card(candidate: chosen, selected: true)
    }

    /// Marks the resolution reviewed: the file can be staged without
    /// asking. Not while a conflict is edited by hand.
    var canMarkReviewed: Bool { review?.reviewed == false && model.editing == nil }

    func markReviewed() {
        guard let conflict, canMarkReviewed else { return }
        model.markReviewed(conflict.index)
    }

    /// Puts the markers back on a conflict resolved outside Scry, to
    /// resolve it here instead.
    var canReopen: Bool { review != nil && model.editing == nil }

    func reopen() {
        guard let conflict, canReopen else { return }
        model.reopen(conflict.index)
    }

    /// The core's id for text resolved outside Scry (`OUTSIDE_ID`).
    private static let outsideId = "outside"

    /// One conflict is edited at a time.
    var canEditByHand: Bool { conflict != nil && model.editing == nil }

    /// While a conflict is edited by hand, a card would change the
    /// resolution behind the open editor.
    var canChoose: Bool { conflict != nil && model.editing == nil }

    func choose(_ candidate: String) {
        guard let conflict, canChoose else { return }
        model.choose(candidate, conflict: conflict.index)
    }

    /// ↑ / ↓ in the candidate list, a radio group: the previous or next
    /// card, in display order, without wrapping. With none chosen, ↓
    /// takes the first and ↑ the last. No global shortcut cycles them: a
    /// habit press would silently change a resolution (SPEC: keyboard map).
    func chooseAdjacent(_ offset: Int) {
        let ids = cards.map(\.id)
        guard !ids.isEmpty else { return }
        let target: Int
        if let current = cards.firstIndex(where: \.selected) {
            target = min(max(current + offset, 0), ids.count - 1)
            guard target != current else { return }
        } else {
            target = offset > 0 ? 0 : ids.count - 1
        }
        choose(ids[target])
    }

    func editByHand() {
        guard let conflict, model.editing == nil else { return }
        onEditByHand(conflict.index)
    }
}
