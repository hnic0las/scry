import Foundation
import Observation

/// Explain's asks and answers for the conflicts of one file: the core,
/// reading the repository, in the app; a fake in tests and previews.
@MainActor
protocol Explainer: AnyObject {
    func explainState(_ index: Int) -> ExplainState
    func askExplain(_ index: Int) throws
    func cancelExplain(_ index: Int) throws
}

extension MergeModel: Explainer {}

/// S3: the focused conflict, its classification and its candidates.
@MainActor
@Observable
final class InspectorViewModel {
    /// Opens a conflict's editor, which the editor screen owns.
    @ObservationIgnored var onEditByHand: (Int) -> Void = { _ in }

    @ObservationIgnored private let model: MergeModel
    @ObservationIgnored private let explainer: Explainer
    @ObservationIgnored private let defaults: UserDefaults

    /// `explainer` answers Explain, the model by default; `defaults` holds
    /// whether Explain is on (Settings › Agent).
    init(model: MergeModel, explainer: Explainer? = nil, defaults: UserDefaults = .standard) {
        self.model = model
        self.explainer = explainer ?? model
        self.defaults = defaults
    }

    // MARK: Explain

    /// Settings › Agent's switch: off, there is no Explain anywhere.
    static let explainKey = "explainConflicts"

    static func explainEnabled(in defaults: UserDefaults) -> Bool {
        defaults.object(forKey: explainKey) as? Bool ?? true
    }

    /// How long an ask waits, with a session watching, before Scry says no
    /// answer came.
    static let answerPatience: TimeInterval = 60

    /// Where Explain stands on the focused conflict, as last read.
    private(set) var explainState: ExplainState?
    /// The conflict `explainState` is about.
    private var explainedIndex: Int?
    /// When `explainState` was read: the waiting time counts from it.
    private(set) var now = Date()
    /// Why the last ask or cancel failed.
    private(set) var explainError: String?
    /// The long answers the user opened, by conflict id, for the session.
    private(set) var expanded: Set<String> = []

    /// What the Explain block shows.
    enum Explain: Equatable {
        /// Nothing asked: the Explain button, and whether a session watches.
        case notAsked(watching: Bool)
        /// Asked, a session watching: answered within seconds. `previous`
        /// stays shown below while asked again.
        case waiting(seconds: Int, previous: Explanation?)
        /// Asked, nobody watching: the ask waits for a session.
        case noSession(previous: Explanation?)
        /// Asked a minute ago, a session watching, and nothing came back.
        case noAnswer(previous: Explanation?)
        case answered(Explanation, watching: Bool)
        /// The answer is about text written by hand that the user changed
        /// since: kept, marked as about an earlier version.
        case stale(Explanation)
        /// Claude Code could not explain: the problem it met.
        case failed(Explanation)
    }

    /// The Explain block; `nil` when Explain is off, the file is in no
    /// repository, or nothing was read yet.
    var explain: Explain? {
        guard Self.explainEnabled(in: defaults),
              let state = explainState, state.available,
              explainedIndex == conflict?.index
        else { return nil }
        guard let asked = state.askedAt else {
            guard let answer = state.answer else { return .notAsked(watching: state.watching) }
            if answer.failed { return .failed(answer) }
            if state.stale { return .stale(answer) }
            return .answered(answer, watching: state.watching)
        }
        // Asked again: the last explanation stays below, a failure does not.
        let previous = state.answer.flatMap { $0.failed ? nil : $0 }
        guard state.watching else { return .noSession(previous: previous) }
        let waited = max(now.timeIntervalSince1970 - Double(asked) / 1000, 0)
        if waited >= Self.answerPatience { return .noAnswer(previous: previous) }
        return .waiting(seconds: Int(waited), previous: previous)
    }

    /// Whether a Claude Code session watches the repository, for the
    /// status bar; `nil` outside a repository, or once Settings › Agent
    /// found Scry's plugin off. Explain off, it stays: the agent can still
    /// resolve (SPEC).
    var watching: Bool? {
        guard ClaudeCodePlugin.isOn(in: defaults), let state = explainState, state.available else { return nil }
        return state.watching
    }

    /// Reads where Explain stands on the focused conflict again, and
    /// whether a session watches: the editor does it every second.
    func refreshExplain() {
        guard let conflict else {
            explainState = nil
            return
        }
        now = Date()
        explainState = explainer.explainState(conflict.index)
        explainedIndex = conflict.index
    }

    /// ⌃⌘E works on a conflict while Explain is on, even with the
    /// inspector hidden (it shows it).
    var explainOn: Bool { conflict != nil && Self.explainEnabled(in: defaults) }

    /// ⌃⌘E and the block's button: asks, or asks again.
    var canExplain: Bool { explain != nil }

    func askExplain() {
        guard let conflict, canExplain else { return }
        run { try self.explainer.askExplain(conflict.index) }
    }

    /// Takes back a waiting ask; the last answer stays.
    func cancelExplain() {
        guard let conflict else { return }
        run { try self.explainer.cancelExplain(conflict.index) }
    }

    private func run(_ action: () throws -> Void) {
        do {
            try action()
            explainError = nil
        } catch {
            explainError = error.message
        }
        refreshExplain()
    }

    /// Whether the focused conflict's long answer is open.
    var isExpanded: Bool { conflict.map { expanded.contains($0.id) } ?? false }

    func toggleExpanded() {
        guard let id = conflict?.id else { return }
        if expanded.contains(id) { expanded.remove(id) } else { expanded.insert(id) }
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
        /// "Resolved by Claude Code", or "Resolved outside Scry".
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
