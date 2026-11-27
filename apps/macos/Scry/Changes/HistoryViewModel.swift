import Foundation
import Observation

/// What a file's history reads: the core's `FileHistory`, or a test's own.
protocol HistorySource: Sendable {
    func header() -> HistoryHeader
    /// The history from its newest point: what is not committed, then the
    /// first page of commits.
    func start() throws -> HistoryList
    /// The list with the next page of older commits added.
    func loadOlder() throws -> HistoryList
    /// How many commits touched the file, under all its names.
    func count() throws -> UInt32
    /// The file at the point `index`, as a change of one file: against
    /// the point before it, or with `pin` against the pinned side.
    func changes(index: UInt32, pin: HistoryPin?) throws -> ChangesSource
    /// What differs between the point `index` and `pin`.
    func pinnedCounts(index: UInt32, pin: HistoryPin) throws -> PointCounts
}

extension FileHistory: HistorySource {
    func changes(index: UInt32, pin: HistoryPin?) throws -> ChangesSource {
        try pointChanges(index: index, pin: pin)
    }
}

/// S18, one file's history: the points where the file changed, in the
/// sidebar in place of the changed files, and the file at the selected
/// point. Read only.
@MainActor
@Observable
final class HistoryViewModel {
    /// What a point is compared with.
    enum Mode: String, CaseIterable, Sendable {
        /// The point before it: what the point changed.
        case eachChange
        /// One side held still: everything between the point and the pin.
        case againstPin
    }

    let header: HistoryHeader
    private(set) var points: [HistoryPoint] = []
    /// Older commits are left to load.
    private(set) var more = false
    /// How many commits touched the file, once counted.
    private(set) var total: Int?
    /// The history is being read for the first time.
    private(set) var loading = true
    private(set) var loadingOlder = false
    /// Why the history could not be read.
    private(set) var error: String?
    private(set) var selected: HistoryPoint?
    private(set) var mode: Mode = .eachChange
    /// The side held still, once one was pinned: kept while the mode is
    /// "Each change", to come back to.
    private(set) var pin: HistoryPin?
    /// The file at the selected point.
    private(set) var file: ChangesViewModel?
    /// What differs between each point and the pin, by the point's place.
    private(set) var pinnedCounts: [UInt32: PointCounts] = [:]

    /// Blame is shown, for the whole window.
    let blameSwitch: BlameSwitch
    let inspectorSwitch: InspectorSwitch
    /// The commit to show first, when the history was opened from a line
    /// of a blame.
    @ObservationIgnored private var startAt: String?
    /// How many pages of older commits are read to find a commit.
    static let pagesToFind = 20
    @ObservationIgnored private let source: HistorySource
    @ObservationIgnored private let defaults: UserDefaults
    @ObservationIgnored private let now: () -> Date
    /// Opens the whole commit of a point.
    @ObservationIgnored private let showCommit: (HistoryPoint) -> Void
    /// Back to the changed files.
    @ObservationIgnored private let leave: () -> Void
    /// The latest point asked: a slower, older answer is dropped.
    @ObservationIgnored private var request = 0
    /// The latest pin whose counts were asked.
    @ObservationIgnored private var countsRequest = 0

    init(
        source: HistorySource,
        defaults: UserDefaults = .standard,
        now: @escaping () -> Date = Date.init,
        showCommit: @escaping (HistoryPoint) -> Void = { _ in },
        leave: @escaping () -> Void = {},
        blame: BlameSwitch = BlameSwitch(),
        inspector: InspectorSwitch? = nil,
        startAt: String? = nil
    ) {
        blameSwitch = blame
        inspectorSwitch = inspector ?? InspectorSwitch(defaults: defaults)
        self.startAt = startAt
        self.source = source
        self.defaults = defaults
        self.now = now
        self.showCommit = showCommit
        self.leave = leave
        header = source.header()
    }

    // MARK: The points

    /// Reads the history, again when called again, and keeps the selected
    /// point selected when it is still there; otherwise the newest one.
    func load() async {
        let source = source
        do {
            let list = try await Task.detached { try source.start() }.value
            let pinned = pinnedPoint
            apply(list)
            error = nil
            loading = false
            // Places change when a commit is made: the pin follows its commit.
            if case .point = pin {
                pin = pinned.flatMap { old in points.first { Self.same($0, old) } }.map { .point(index: $0.index) }
                if pin == nil { mode = .eachChange }
            }
            let kept = selected.flatMap { old in points.first { Self.same($0, old) } }
            if let startAt {
                self.startAt = nil
                selected = points.first
                await show(commit: startAt)
            } else {
                await select(kept ?? points.first)
            }
            await count()
            await readPinnedCounts()
        } catch {
            loading = false
            self.error = error.message
        }
    }

    /// Shows what the commit `id` did to the file: its point, in "Each
    /// change", reading older pages until it is found. A commit that is
    /// not in the history leaves the selected point shown.
    func show(commit id: String) async {
        var pages = 0
        while !points.contains(where: { $0.id == id }), more, pages < Self.pagesToFind {
            await loadOlder()
            pages += 1
        }
        let point = points.first { $0.id == id }
        if mode != .eachChange, point != nil { mode = .eachChange }
        await select(point ?? selected)
    }

    /// The file at a point, with the window's blame.
    private func fileModel(_ changes: ChangesSource, empty: String) -> ChangesViewModel {
        ChangesViewModel(
            source: changes, defaults: defaults, emptyMessage: empty, blame: blameSwitch,
            inspector: inspectorSwitch,
            openBlamed: { [weak self] commit, _ in
                Task { await self?.show(commit: commit.id) }
            }
        )
    }

    /// Loads the history once: coming back to it, by Back or Forward,
    /// shows it as it was left.
    func loadOnce() async {
        guard loading else { return }
        await load()
    }

    /// Adds the next page of older commits.
    func loadOlder() async {
        guard more, !loadingOlder else { return }
        loadingOlder = true
        defer { loadingOlder = false }
        let source = source
        guard let list = try? await Task.detached(operation: { try source.loadOlder() }).value else { return }
        apply(list)
        await readPinnedCounts()
    }

    /// Reads the history and the file at the point `place` on the
    /// calling thread, pinned against the newest commit with `pinned`,
    /// for previews, which are drawn once.
    func loadNow(select place: Int = 0, pinned: Bool = false) {
        loading = false
        guard let list = try? source.start() else { return }
        apply(list)
        total = total ?? (try? source.count()).map(Int.init)
        if pinned, let pin = defaultPin {
            self.pin = pin
            mode = .againstPin
            for point in points {
                pinnedCounts[point.index] = try? source.pinnedCounts(index: point.index, pin: pin)
            }
        }
        guard points.indices.contains(place) else { return }
        selected = points[place]
        guard let changes = try? source.changes(index: points[place].index, pin: activePin) else { return }
        let file = fileModel(changes, empty: emptyMessage(for: points[place]))
        file.loadNow()
        self.file = file
    }

    private func apply(_ list: HistoryList) {
        points = list.points
        more = list.more
        if !more { total = commitCount }
    }

    /// Counts the commits when the pages do not hold them all.
    private func count() async {
        guard more else { return }
        let source = source
        if let count = try? await Task.detached(operation: { try source.count() }).value {
            total = Int(count)
        }
    }

    private static func same(_ one: HistoryPoint, _ other: HistoryPoint) -> Bool {
        one.kind == other.kind && one.id == other.id
    }

    /// How many of the loaded points are commits.
    var commitCount: Int { points.count { $0.kind == .commit } }

    /// "142 commits · renames followed".
    var summary: String {
        guard let total else { return loading ? "" : "Counting the commits…" }
        if total == 0 { return "Not committed yet" }
        return "\(total == 1 ? "1 commit" : "\(total) commits") · renames followed"
    }

    /// "50 of 142 loaded", while older commits are left.
    var loadedSummary: String? {
        guard more else { return nil }
        guard let total else { return "\(commitCount) loaded" }
        return "\(commitCount) of \(total) loaded"
    }

    // MARK: The selected point

    /// Shows the file at `point`, read off the main thread. The file
    /// shown stays on screen until the next one is read.
    func select(_ point: HistoryPoint?) async {
        request += 1
        let asked = request
        selected = point
        guard let point else {
            file = nil
            return
        }
        let source = source
        let pin = activePin
        let empty = emptyMessage(for: point)
        do {
            let changes = try await Task.detached { try source.changes(index: point.index, pin: pin) }.value
            let file = fileModel(changes, empty: empty)
            await file.load()
            guard request == asked else { return }
            self.file = file
            error = nil
        } catch {
            guard request == asked else { return }
            file = nil
            self.error = error.message
        }
    }

    /// What the file area says when nothing differs at `point`.
    private func emptyMessage(for point: HistoryPoint) -> String {
        guard mode == .againstPin else { return "This point did not change the file's content." }
        return isPinned(point)
            ? "This point is the pin: there is nothing to compare it with. Select another point to see everything between the two."
            : "The file is the same at this point and at the pin."
    }

    var canGoToOlder: Bool {
        guard let index = selectedIndex else { return false }
        return index < points.count - 1 || more
    }

    var canGoToNewer: Bool { (selectedIndex ?? 0) > 0 }

    private var selectedIndex: Int? {
        selected.flatMap { selected in points.firstIndex { $0.index == selected.index } }
    }

    /// The point before the selected one in time; at the end of the
    /// page, the next page is loaded first.
    func older() async {
        guard let index = selectedIndex else { return }
        if index == points.count - 1 { await loadOlder() }
        if points.indices.contains(index + 1) { await select(points[index + 1]) }
    }

    func newer() async {
        guard let index = selectedIndex, index > 0 else { return }
        await select(points[index - 1])
    }

    // MARK: The pin

    /// The pin the points are compared with now.
    private var activePin: HistoryPin? { mode == .againstPin ? pin : nil }

    /// The commit pinned, when a commit is.
    private var pinnedPoint: HistoryPoint? {
        guard case .point(let index) = pin else { return nil }
        return points.first { $0.index == index }
    }

    /// "Against a pin" needs something to pin: a commit, or the working
    /// copy.
    var canPin: Bool { defaultPin != nil }

    /// The newest commit, which holds what HEAD holds of the file.
    private var defaultPin: HistoryPin? {
        if let commit = points.first(where: { $0.kind == .commit }) { return .point(index: commit.index) }
        return nil
    }

    func setMode(_ mode: Mode) async {
        guard mode != self.mode else { return }
        if mode == .againstPin {
            if pin == nil { pin = defaultPin }
            guard pin != nil else { return }
        }
        self.mode = mode
        await select(selected)
        await readPinnedCounts()
    }

    /// Whether `point` can be pinned: a commit, or the working copy.
    func canPin(_ point: HistoryPoint) -> Bool { point.kind != .staged }

    var canPinSelected: Bool { selected.map(canPin) ?? false }

    func isPinned(_ point: HistoryPoint) -> Bool {
        guard mode == .againstPin else { return false }
        switch pin {
        case .point(let index): return point.index == index
        case .workingCopy: return point.kind == .notCommitted
        case nil: return false
        }
    }

    /// Pins `point`: every point is then compared with it. On the pin
    /// itself, back to "Each change".
    func togglePin(_ point: HistoryPoint) async {
        guard canPin(point) else { return }
        if isPinned(point) { return await setMode(.eachChange) }
        pin = point.kind == .notCommitted ? .workingCopy : .point(index: point.index)
        mode = .againstPin
        await select(selected)
        await readPinnedCounts()
    }

    func togglePinOnSelected() async {
        if let selected { await togglePin(selected) }
    }

    /// The pin's name: "HEAD", a commit's short id, or the working copy.
    var pinName: String {
        switch pin {
        case .workingCopy: return "the working copy"
        case .point:
            guard let point = pinnedPoint else { return "the pin" }
            return point.head ? "HEAD" : point.shortId
        case nil: return "the pin"
        }
    }

    /// The sentence under the two modes.
    var modeSentence: String {
        mode == .eachChange
            ? "Each point shows what it changed: a commit against its parent."
            : "Each point shows everything between it and the pin, \(pinName)."
    }

    /// Reads, point after point, what differs between each and the pin.
    private func readPinnedCounts() async {
        countsRequest += 1
        let asked = countsRequest
        guard let pin = activePin else {
            pinnedCounts = [:]
            return
        }
        let source = source
        var counts: [UInt32: PointCounts] = [:]
        pinnedCounts = [:]
        for point in points {
            let index = point.index
            let read = try? await Task.detached(operation: { try source.pinnedCounts(index: index, pin: pin) }).value
            guard countsRequest == asked else { return }
            counts[index] = read
            pinnedCounts = counts
        }
    }

    /// The counts on a point's row: its own in "Each change", against
    /// the pin otherwise, once read.
    func counts(_ point: HistoryPoint) -> (added: UInt32?, removed: UInt32?) {
        guard mode == .againstPin else { return (point.added, point.removed) }
        guard let counts = pinnedCounts[point.index], !counts.same else { return (nil, nil) }
        return (counts.added, counts.removed)
    }

    // MARK: Leaving

    /// Back to the changed files.
    func showChangedFiles() { leave() }

    var canShowWholeCommit: Bool { selected?.kind == .commit }

    /// The Changes window for the selected commit.
    func showWholeCommit() {
        if let selected { showWholeCommit(selected) }
    }

    func showWholeCommit(_ point: HistoryPoint) {
        guard point.kind == .commit else { return }
        showCommit(point)
    }

    // MARK: Shortcut hints

    var toolbarHints: [Shortcut] {
        var shortcuts = file?.toolbarHints ?? []
        shortcuts.append(.history)
        return shortcuts
    }

    var hintStrip: [Shortcut] {
        var shortcuts: [Shortcut] = []
        if canGoToOlder { shortcuts.append(.olderPoint) }
        if canGoToNewer { shortcuts.append(.newerPoint) }
        if canPinSelected { shortcuts.append(.pinPoint) }
        shortcuts.append(contentsOf: [.openRepository, .keyboardShortcuts])
        return shortcuts
    }

    // MARK: Words

    /// A point's first line: the commit's subject, or what it stands for.
    static func title(_ point: HistoryPoint) -> String {
        switch point.kind {
        case .notCommitted: "Not committed"
        case .staged: "Staged"
        case .commit: point.subject.isEmpty ? "(no message)" : point.subject
        }
    }

    /// A point's second line: "9f8e7d6 · You · 2 d".
    func detail(_ point: HistoryPoint) -> String {
        switch point.kind {
        case .notCommitted: return "The working copy against the index"
        case .staged: return "The index against HEAD"
        case .commit:
            let author = !header.me.isEmpty && point.author == header.me ? "You" : point.author
            return "\(point.shortId) · \(author) · \(Self.age(of: point.time, now: now()))"
        }
    }

    /// How long ago, in the largest unit: "now", "5 min", "3 h", "8 d",
    /// "2 mo", "4 y".
    nonisolated static func age(of time: Int64, now: Date) -> String {
        let seconds = max(Int64(now.timeIntervalSince1970) - time, 0)
        let (minute, hour, day): (Int64, Int64, Int64) = (60, 3_600, 86_400)
        switch seconds {
        case ..<minute: return "now"
        case ..<hour: return "\(seconds / minute) min"
        case ..<day: return "\(seconds / hour) h"
        case ..<(30 * day): return "\(seconds / day) d"
        case ..<(365 * day): return "\(seconds / (30 * day)) mo"
        default: return "\(seconds / (365 * day)) y"
        }
    }

    /// The commit's exact date, for the row's tooltip.
    static func date(_ point: HistoryPoint) -> String {
        Date(timeIntervalSince1970: TimeInterval(point.time)).formatted(date: .abbreviated, time: .shortened)
    }
}
