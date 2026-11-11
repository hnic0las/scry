import Foundation
import Observation

/// S2: ours, result and theirs side by side. Turns the core's snapshot into
/// what the editor shows (headers, rows, status, toolbar state) and takes
/// the user's actions. Owns the inspector's and the edit block's view models.
@MainActor
@Observable
final class MergeEditorViewModel {
    /// The inspector beside the editor.
    @ObservationIgnored let inspector: InspectorViewModel
    /// The editor open under a conflict's lens row, while editing by hand.
    private(set) var editBlock: EditBlockViewModel?
    /// Asking before writing a result that does not parse.
    var confirmingInvalidWrite = false
    /// Asking before writing over a file changed on disk since it was
    /// opened.
    var confirmingOverwrite = false

    @ObservationIgnored private let model: MergeModel

    init(model: MergeModel) {
        self.model = model
        inspector = InspectorViewModel(model: model)
        inspector.onEditByHand = { [weak self] conflict in
            self?.startEditing(conflict)
        }
    }

    // MARK: Window

    var title: String { model.header.fileName }
    var subtitle: String { model.header.summary ?? model.header.path }

    // MARK: Headers

    /// A pane's header: the side's label and one line of detail.
    struct PaneHeader: Equatable {
        let label: String
        let detail: String
    }

    var oursHeader: PaneHeader {
        PaneHeader(label: model.header.ours.label, detail: model.header.ours.detail)
    }

    var resultHeader: PaneHeader {
        PaneHeader(
            label: "Result · \(model.header.fileName)",
            detail: "\(resolvedSummary) · written on Mark as Resolved"
        )
    }

    var theirsHeader: PaneHeader {
        PaneHeader(label: model.header.theirs.label, detail: model.header.theirs.detail)
    }

    // MARK: Rows

    /// A row as drawn: a lens row above a conflict, or a line of each pane.
    enum RowState {
        case lens(LensRowState)
        case code(CodeRowState)
    }

    struct LensRowState {
        let conflict: Int
        let title: String
        let focused: Bool
        let oursCommit: LensCommit?
        let theirsCommit: LensCommit?
        let badges: [BadgeState]
        /// The conflict is open for editing under this row.
        let editing: Bool
    }

    struct CodeRowState {
        let ours: Cell
        let result: Cell
        let theirs: Cell
        /// The conflict the line belongs to, if any.
        let conflict: Int?
        /// The chosen resolution does not parse.
        let resultInvalid: Bool
        let resultProvenance: Provenance
    }

    /// A lens row's label for a conflict's state.
    struct BadgeState: Equatable {
        enum Kind: Equatable {
            case resolved(Provenance)
            case doesNotParse
            case unresolved
        }

        let text: String
        let kind: Kind
    }

    /// The rows, built again only when the model changes: SwiftUI reads
    /// them on every evaluation of the editor, and a large file has tens of
    /// thousands.
    var rows: [RowState] {
        let revision = model.revision
        if let cached = rowsCache, cached.revision == revision {
            return cached.rows
        }
        let rows = buildRows()
        rowsCache = (revision, rows)
        return rows
    }

    @ObservationIgnored private var rowsCache: (revision: Int, rows: [RowState])?

    private func buildRows() -> [RowState] {
        // Once per conflict, not once per line.
        let chosen = model.conflicts.map(\.chosenCandidate)
        return model.rows.map { row in
            switch row {
            case .lens(let conflict):
                return .lens(lensRow(Int(conflict), chosen: chosen[Int(conflict)]))
            case .code(let ours, let result, let theirs, let conflict):
                let candidate = conflict.flatMap { chosen[Int($0)] }
                return .code(CodeRowState(
                    ours: ours,
                    result: result,
                    theirs: theirs,
                    conflict: conflict.map { Int($0) },
                    resultInvalid: candidate?.doesNotParse ?? false,
                    resultProvenance: candidate?.provenance ?? .git
                ))
            }
        }
    }

    /// The longest line of any pane, in characters: how far the panes can
    /// scroll sideways.
    var longestLine: Int { model.longestLine }

    /// The focused conflict, from 0.
    var currentConflict: Int { model.current }

    /// The row index of a conflict's lens row, to scroll to it.
    func lensRowIndex(of conflict: Int) -> Int? {
        model.rows.firstIndex { row in
            if case .lens(let index) = row { return Int(index) == conflict }
            return false
        }
    }

    private func lensRow(_ conflict: Int, chosen: CandidateInfo?) -> LensRowState {
        let info = model.conflict(conflict)
        return LensRowState(
            conflict: conflict,
            title: "Conflict \(conflict + 1)",
            focused: model.current == conflict,
            oursCommit: info?.oursCommit,
            theirsCommit: info?.theirsCommit,
            badges: badges(chosen),
            editing: model.editing == conflict
        )
    }

    private func badges(_ chosen: CandidateInfo?) -> [BadgeState] {
        guard let chosen else { return [BadgeState(text: "Unresolved", kind: .unresolved)] }
        var badges = [BadgeState(text: chosen.title, kind: .resolved(chosen.provenance))]
        if chosen.doesNotParse {
            badges.append(BadgeState(text: "Does not parse", kind: .doesNotParse))
        }
        return badges
    }

    // MARK: Toolbar

    var counter: String {
        "Conflict \(min(model.current + 1, model.conflicts.count)) of \(model.conflicts.count)"
    }

    var canGoToPrevious: Bool { model.current > 0 }
    var canGoToNext: Bool { model.current + 1 < model.conflicts.count }
    var isEditing: Bool { editBlock != nil }
    var canUndo: Bool { model.status.canUndo && !isEditing }
    var canRedo: Bool { model.status.canRedo && !isEditing }

    /// Every conflict is resolved, and no edit is pending: ⌘↩ applies the
    /// edit meanwhile.
    var canMarkResolved: Bool {
        model.status.total > 0 && model.status.resolved == model.status.total && !isEditing
    }

    // MARK: Status bar

    enum Verification: Equatable {
        case parses(String)
        case doesNotParse(line: UInt32)
        case notChecked
    }

    var resolvedSummary: String {
        "\(model.status.resolved) of \(model.status.total) conflicts resolved"
    }

    var verification: Verification {
        switch model.status.merged {
        case .parses: .parses(languageName)
        case .doesNotParse(let line): .doesNotParse(line: line)
        case .notChecked: .notChecked
        }
    }

    var language: String? { model.status.language }
    var written: Bool { model.written }

    /// Choices made since the file was opened or last written, or an edit
    /// still open: closing the window would lose them.
    var hasUnwrittenWork: Bool {
        isEditing || (!model.written && model.status.canUndo)
    }

    // MARK: Alerts

    var overwriteMessage: String {
        "\(model.header.fileName) changed on disk since Scry opened it, in another editor or through git. Writing this result replaces those changes."
    }

    var invalidWriteMessage: String {
        guard case .doesNotParse(let line) = model.status.merged else { return "" }
        return "\(model.header.fileName) does not parse as \(languageName) near line \(line)."
    }

    /// The last action the core refused; set to `nil` once shown.
    var failure: String? {
        get { model.failure }
        set { model.failure = newValue }
    }

    // MARK: Intents

    func previous() {
        model.focus(model.current - 1)
    }

    func next() {
        model.focus(model.current + 1)
    }

    func focus(_ conflict: Int) {
        model.focus(conflict)
    }

    /// Takes a candidate for the focused conflict, when it offers it.
    func take(_ candidate: String) {
        guard !isEditing,
              let conflict = model.conflict(model.current),
              conflict.candidates.contains(where: { $0.id == candidate })
        else { return }
        model.choose(candidate, conflict: model.current)
    }

    /// Both sides, ours first; pressed again, theirs first.
    func takeBoth() {
        let both = model.conflict(model.current)?.chosen == "ours-then-theirs"
            ? "theirs-then-ours" : "ours-then-theirs"
        take(both)
    }

    /// Opens a conflict's text for editing by hand, the focused one by
    /// default.
    func startEditing(_ conflict: Int? = nil) {
        let conflict = conflict ?? model.current
        guard !isEditing, model.conflict(conflict) != nil else { return }
        model.startEditing(conflict)
        editBlock = EditBlockViewModel(model: model, conflict: conflict) { [weak self] in
            self?.stopEditing()
        }
    }

    func undo() {
        guard canUndo else { return }
        model.undo()
    }

    func redo() {
        guard canRedo else { return }
        model.redo()
    }

    /// Writes the result, after asking when it does not parse.
    func markResolved() {
        guard canMarkResolved else { return }
        if case .doesNotParse = model.status.merged {
            confirmingInvalidWrite = true
        } else {
            write()
        }
    }

    /// Writes a result that does not parse, once the user confirmed.
    func writeAnyway() {
        write()
    }

    /// Writes over a file changed on disk, once the user confirmed.
    func overwrite() {
        model.save(overwrite: true)
    }

    // MARK: Private

    private var languageName: String { model.status.language ?? "code" }

    private func write() {
        if model.save() == .changedOnDisk {
            confirmingOverwrite = true
        }
    }

    private func stopEditing() {
        model.stopEditing()
        editBlock = nil
    }
}
