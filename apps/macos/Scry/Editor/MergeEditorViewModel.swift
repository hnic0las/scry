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
    /// The editor under the conflict being edited by hand. Which conflict
    /// is the model's (`MergeModel.editing`), the one source of truth;
    /// the editor and its draft are kept here until the edit ends.
    var editBlock: EditBlockViewModel? {
        guard let editing = model.editing else { return nil }
        if let block = editBlockCache, block.conflict == editing { return block }
        let block = EditBlockViewModel(model: model, conflict: editing) { [weak self] in
            self?.stopEditing()
        }
        editBlockCache = block
        return block
    }

    @ObservationIgnored private var editBlockCache: EditBlockViewModel?
    /// Asking before writing a result that does not parse.
    var confirmingInvalidWrite = false
    /// Asking before writing over a file changed on disk since it was
    /// opened.
    var confirmingOverwrite = false

    @ObservationIgnored private let model: MergeModel
    /// Where the layout choice is remembered, per user, not per repository.
    @ObservationIgnored private let defaults: UserDefaults

    /// `defaults` keeps the layout choice; tests pass their own suite.
    init(model: MergeModel, defaults: UserDefaults = .standard) {
        self.model = model
        self.defaults = defaults
        preferredLayout = defaults.string(forKey: Self.layoutKey).flatMap(EditorLayout.init(rawValue:)) ?? .columns
        inspector = InspectorViewModel(model: model)
        inspector.onEditByHand = { [weak self] conflict in
            self?.startEditing(conflict)
        }
    }

    // MARK: Window

    var title: String { model.header.fileName }
    var subtitle: String { model.header.summary ?? model.header.path }

    // MARK: Layout

    /// Ours, result and theirs side by side, or the two sides above the
    /// result (SPEC: sides above, result below).
    enum EditorLayout: String, CaseIterable {
        case columns
        case stacked
    }

    static let layoutKey = "mergeEditorLayout"
    /// Below this window width the sides are too narrow beside the result,
    /// so the editor stacks them whatever the choice.
    static let narrowWidth = 1100.0

    /// The user's choice, kept for the next file; three columns at first.
    private(set) var preferredLayout: EditorLayout
    /// The window is narrower than `narrowWidth`.
    private(set) var isNarrow = false

    /// The layout shown: stacked in a narrow window, without changing the
    /// choice, so widening the window brings it back.
    var layout: EditorLayout { isNarrow ? .stacked : preferredLayout }
    var canSwitchLayout: Bool { !isNarrow }

    /// The layout control's tooltip.
    var layoutHelp: String {
        isNarrow ? "Needs a wider window" : Shortcut.switchLayout.help()
    }

    func setLayout(_ layout: EditorLayout) {
        guard canSwitchLayout, layout != preferredLayout else { return }
        preferredLayout = layout
        defaults.set(layout.rawValue, forKey: Self.layoutKey)
    }

    /// ⌥⌘L.
    func switchLayout() {
        setLayout(preferredLayout == .columns ? .stacked : .columns)
    }

    // MARK: Base

    /// Some conflict has a base to show: git wrote diff3 markers, or a
    /// merge tool passed the base file.
    var canShowBase: Bool { model.conflicts.contains(where: \.hasBase) }
    var showsBase: Bool { model.showsBase }

    var baseHelp: String {
        canShowBase
            ? Shortcut.toggleBase.help(showsBase ? "Hide base" : "Show base")
            : "No base: git wrote this file without diff3 markers"
    }

    /// ⌥⌘B.
    func toggleBase() {
        guard canShowBase else { return }
        model.setShowsBase(!model.showsBase)
    }

    /// Reads the choice again: Settings may have changed it.
    func reloadLayout() {
        let stored = defaults.string(forKey: Self.layoutKey).flatMap(EditorLayout.init(rawValue:)) ?? .columns
        if stored != preferredLayout { preferredLayout = stored }
    }

    func windowWidthChanged(_ width: Double) {
        let narrow = width < Self.narrowWidth
        if narrow != isNarrow { isNarrow = narrow }
    }

    /// The row to scroll to the top for a conflict: its lens row, or in
    /// the stacked layout the row 4 above it, so the conflict sits a little
    /// below the top of both areas with some context.
    func scrollTarget(for conflict: Int) -> Int? {
        guard let lens = lensRowIndex(of: conflict) else { return nil }
        return layout == .stacked ? max(lens - 4, 0) : lens
    }

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
        /// From the core, each with its theme role.
        let badges: [LensBadge]
        /// The conflict is open for editing under this row.
        let editing: Bool
        /// The take buttons work: no conflict is open for editing.
        let canTake: Bool
        /// The ••• menu: the candidates the core offers, in its order.
        let menu: [LensMenuItem]
    }

    /// A candidate in the lens row's ••• menu, with its direct key when it
    /// has one.
    struct LensMenuItem: Equatable {
        let id: String
        let title: String
        let shortcut: Shortcut?
    }

    /// The keys that take a candidate directly, by its ID.
    private static func shortcut(for candidate: String) -> Shortcut? {
        switch candidate {
        case "ours": .takeOurs
        case "theirs": .takeTheirs
        case "ours-then-theirs": .takeBoth
        case "neither": .takeNone
        case "base": .takeBase
        default: nil
        }
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
        let chosen = model.conflicts.map(\.chosen)
        return model.rows.map { row in
            switch row {
            case .lens(let conflict):
                return .lens(lensRow(Int(conflict)))
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

    /// The widest line of any pane, in columns: how far the panes can
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

    private func lensRow(_ conflict: Int) -> LensRowState {
        let info = model.conflict(conflict)
        return LensRowState(
            conflict: conflict,
            title: "Conflict \(conflict + 1)",
            focused: model.current == conflict,
            oursCommit: info?.oursCommit,
            theirsCommit: info?.theirsCommit,
            badges: info?.badges ?? [],
            editing: model.editing == conflict,
            canTake: model.editing == nil,
            menu: (info?.candidates ?? []).map {
                LensMenuItem(id: $0.id, title: $0.title, shortcut: Self.shortcut(for: $0.id))
            }
        )
    }

    // MARK: Toolbar

    var counter: String {
        "Conflict \(min(model.current + 1, model.conflicts.count)) of \(model.conflicts.count)"
    }

    var canGoToPrevious: Bool { model.current > 0 }
    var canGoToNext: Bool { model.current + 1 < model.conflicts.count }
    var isEditing: Bool { model.editing != nil }

    /// The toolbar's shortcuts that work now, in the toolbar's order, for
    /// the hint row under it while ⌘ is held.
    var toolbarHints: [Shortcut] {
        var shortcuts: [Shortcut] = []
        if canGoToPrevious { shortcuts.append(.previousConflict) }
        if canGoToNext { shortcuts.append(.nextConflict) }
        if !isEditing { shortcuts.append(contentsOf: [Shortcut.takeOurs, .takeBoth, .takeTheirs]) }
        if canShowBase { shortcuts.append(.toggleBase) }
        if canSwitchLayout { shortcuts.append(.switchLayout) }
        shortcuts.append(.toggleInspector)
        if canMarkResolved { shortcuts.append(.markAsResolved) }
        return shortcuts
    }

    /// The shortcuts with no visible control, for the hint strip while ⌘
    /// is held: only those that work now. Editing by hand shows its own
    /// buttons instead.
    var hintStrip: [Shortcut] {
        guard !isEditing else { return [] }
        var shortcuts: [Shortcut] = [.editByHand, .takeBase, .takeNone]
        if canUndo { shortcuts.append(.undo) }
        if canRedo { shortcuts.append(.redo) }
        shortcuts.append(.keyboardShortcuts)
        return shortcuts
    }
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

    /// Back to the overview the file was opened from; `nil` when it was
    /// opened on its own.
    @ObservationIgnored var onBack: (() -> Void)?
    var canGoBack: Bool { onBack != nil }
    /// Asking before going back with choices not written.
    var confirmingLeave = false

    /// Goes back to the overview, after asking when choices are not
    /// written.
    func back() {
        guard let onBack else { return }
        if hasUnwrittenWork {
            confirmingLeave = true
        } else {
            onBack()
        }
    }

    /// Goes back, losing the choices not written, once the user agreed.
    func leaveWithoutWriting() {
        onBack?()
    }
    /// Written at least once since the file was opened.
    var everWritten: Bool { model.everWritten }

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

    /// A lens row's button: focuses its conflict, then takes the
    /// candidate, so the button acts on the conflict it sits on.
    func take(_ candidate: String, in conflict: Int) {
        guard !isEditing else { return }
        model.focus(conflict)
        take(candidate)
    }

    /// The ••• menu's Edit by Hand: opens this conflict, not the focused
    /// one.
    func editByHand(in conflict: Int) {
        guard !isEditing else { return }
        model.focus(conflict)
        startEditing(conflict)
    }

    func takeBoth(in conflict: Int) {
        guard !isEditing else { return }
        model.focus(conflict)
        takeBoth()
    }

    /// Both sides, ours first; pressed again, theirs first.
    func takeBoth() {
        let both = model.conflict(model.current)?.chosen?.id == "ours-then-theirs"
            ? "theirs-then-ours" : "ours-then-theirs"
        take(both)
    }

    /// Opens a conflict's text for editing by hand, the focused one by
    /// default.
    func startEditing(_ conflict: Int? = nil) {
        let conflict = conflict ?? model.current
        guard !isEditing, model.conflict(conflict) != nil else { return }
        model.startEditing(conflict)
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
        editBlockCache = nil
    }
}
