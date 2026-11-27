import Foundation
import Observation

/// What the Changes window reads: the core's `ChangeSet`, or a test's own.
protocol ChangesSource: Sendable {
    func header() -> ChangesHeader
    /// The files that differ, read again.
    func files() throws -> [ChangesFile]
    /// The diff of the file at `index` of the last list.
    func fileDiff(index: UInt32, allMarks: Bool) throws -> FileDiffInfo
    /// Marks the file at `index` of the last list viewed, or not.
    func setViewed(index: UInt32, viewed: Bool) throws
}

extension ChangeSet: ChangesSource {}

/// S17, the Changes window: the files that differ between two versions of
/// a repository, and the selected one shown whole, unified or split. Read
/// only: nothing here writes to the repository.
@MainActor
@Observable
final class ChangesViewModel {
    /// How the file is shown.
    enum Layout: String, CaseIterable, Sendable {
        /// One column: in each block the removed lines, then the added ones.
        case unified
        /// The old side beside the new one.
        case split
    }

    /// What the file area shows.
    enum Content: Equatable {
        /// No file: the list is empty or still loading.
        case none
        case loading
        case text(TextDiffInfo)
        /// Sizes in bytes; `nil` for a side the file is not on.
        case binary(old: UInt64?, new: UInt64?)
        /// A repository inside the repository.
        case submodule
        /// Left with conflicts by a merge: the merge editor resolves it.
        case conflicted
        case failed(String)
    }

    /// A group of the list: staged, not staged, untracked; or the whole
    /// list, untitled, when the change has no groups.
    struct Section: Identifiable, Equatable {
        let title: String?
        /// `nil` for the whole list, which cannot be collapsed.
        let group: ChangeGroup?
        let files: [ChangesFile]
        /// Its files are hidden under its title.
        let collapsed: Bool
        var id: String { title ?? "" }
    }

    /// Under `git difftool`, which opens its files one after the other:
    /// this window's file is number `place` of `total`.
    struct Queue: Equatable, Sendable {
        let place: Int
        let total: Int

        var isLast: Bool { place >= total }
    }

    /// A block of the file on the change map: where it starts and how
    /// tall it is, as shares of the file, and how much of it is removed.
    struct MapMark: Identifiable, Equatable {
        let id: Int
        let start: Double
        let length: Double
        /// The share of the block's lines that are removed, from 0 to 1.
        let removedShare: Double
        let current: Bool
    }

    /// Why a row of the list has a dot: what happened to its file while
    /// the window was open, until the file is shown.
    enum Arrival: Equatable {
        /// The file was not in the list.
        case new
        case changed
    }

    let header: ChangesHeader
    /// The files that arrived or changed while the window was open, by
    /// path.
    private(set) var arrived: [String: Arrival] = [:]
    /// When the list last changed by itself.
    private(set) var updatedAt: Date?
    /// Why the last "Viewed" mark could not be written.
    private(set) var viewedError: String?
    /// Goes up when the file area must move to `placeTarget`, after the
    /// open file changed on disk.
    private(set) var placeRequest = 0
    /// Where the top of the file area must be, in rows from the file's
    /// first one.
    private(set) var placeTarget: Double = 0
    /// Where the top of the file area is, in rows: the view says so as it
    /// scrolls.
    @ObservationIgnored var topRow: Double = 0
    private(set) var files: [ChangesFile] = []
    /// The list is being read for the first time.
    private(set) var loadingFiles = true
    /// Why the list could not be read.
    private(set) var listError: String?
    private(set) var selected: ChangesFile?
    private(set) var content: Content = .none
    /// The block "next change" last went to, from 0.
    private(set) var currentChange = 0
    /// Goes up when the file area must scroll to the current block.
    private(set) var scrollRequest = 0
    /// "Last change · press again for the next file": the next press of
    /// next change leaves the file.
    private(set) var atFileEdge: FileEdge?
    /// The groups whose files are hidden under their title.
    private(set) var collapsed: Set<ChangeGroup> = []
    /// The file's place among those `git difftool` opens in turn.
    let queue: Queue?
    var layout: Layout {
        didSet {
            guard layout != oldValue else { return }
            defaults.set(layout.rawValue, forKey: Self.layoutKey)
            scrollRequest += 1
        }
    }

    enum FileEdge: Equatable {
        case first, last
    }

    static let layoutKey = "changesLayout"

    @ObservationIgnored private let source: ChangesSource
    @ObservationIgnored private let defaults: UserDefaults
    /// Opens a conflicted file in the merge editor, by its path from the
    /// repository's root.
    @ObservationIgnored private let openConflicted: (String) -> Void
    /// Closes the window for git's next file, or with `true` for none.
    @ObservationIgnored private let leaveQueue: (Bool) -> Void
    /// The latest file asked: a slower, older answer is dropped.
    @ObservationIgnored private var request = 0
    /// The repository's changes on disk, as they settle.
    @ObservationIgnored private let watch: ((ChangesHeader) -> AsyncStream<Void>)?
    /// The open file was asked with its colours and word marks, though
    /// large.
    @ObservationIgnored private var allMarks = false
    @ObservationIgnored private let now: () -> Date

    init(
        source: ChangesSource,
        defaults: UserDefaults = .standard,
        queue: Queue? = nil,
        watch: ((ChangesHeader) -> AsyncStream<Void>)? = nil,
        now: @escaping () -> Date = Date.init,
        leaveQueue: @escaping (Bool) -> Void = { _ in },
        openConflicted: @escaping (String) -> Void = { _ in }
    ) {
        self.watch = watch
        self.now = now
        self.source = source
        self.defaults = defaults
        self.queue = queue
        self.leaveQueue = leaveQueue
        self.openConflicted = openConflicted
        header = source.header()
        layout = defaults.string(forKey: Self.layoutKey).flatMap(Layout.init) ?? .unified
    }

    // MARK: The list

    /// Reads the files, again when called again, and keeps the selected
    /// file selected when it is still there; otherwise the first one.
    func load() async {
        let source = source
        do {
            let files = try await Task.detached { try source.files() }.value
            self.files = files
            listError = nil
            arrived = [:]
            let kept = selected.flatMap { old in
                files.first { $0.path == old.path && $0.group == old.group }
            }
            loadingFiles = false
            await select(kept ?? files.first)
        } catch {
            loadingFiles = false
            listError = error.message
        }
    }

    // MARK: Live refresh

    /// Reads the list again each time the repository changes on disk,
    /// until the window closes. Only the working copy and the index
    /// change under a window.
    func watch() async {
        guard header.live, let watch else { return }
        for await _ in watch(header) {
            await refresh()
        }
    }

    /// Reads the list again after a change on disk. The selected file
    /// stays selected and, when its content changed, is read again with
    /// the view kept on the same line. The other files that changed or
    /// arrived get a dot in the list.
    func refresh() async {
        let source = source
        guard let fresh = try? await Task.detached(operation: { try source.files() }).value else { return }
        guard fresh != files else { return }
        let before = Dictionary(files.map { ($0.path, $0.contentId) }, uniquingKeysWith: { first, _ in first })
        var arrived = arrived.filter { path, _ in fresh.contains { $0.path == path } }
        for file in fresh where before[file.path] != file.contentId {
            // A file that arrived stays "new" when it changes again.
            arrived[file.path] = before[file.path] == nil ? .new : arrived[file.path] ?? .changed
        }
        let old = selected
        let oldIndex = old.flatMap { files.firstIndex(of: $0) }
        files = fresh
        listError = nil
        loadingFiles = false
        updatedAt = now()
        if let old, let same = fresh.first(where: { $0.path == old.path }) {
            // The file on screen is being looked at: no dot.
            arrived[same.path] = nil
            self.arrived = arrived
            selected = same
            if same.contentId != old.contentId { await readAgain(same) }
        } else {
            self.arrived = arrived
            // The file no longer differs: the one that took its place.
            let next = oldIndex.map { min($0, fresh.count - 1) }.flatMap { fresh.indices.contains($0) ? fresh[$0] : nil }
            await select(next ?? fresh.first)
        }
    }

    /// Reads the selected file again without the loading state and
    /// without going to a block: the view stays where the user reads.
    private func readAgain(_ file: ChangesFile) async {
        request += 1
        let asked = request
        let source = source
        let allMarks = allMarks
        let result = await Task.detached { Self.read(file, from: source, allMarks: allMarks) }.value
        guard request == asked else { return }
        let before = diff
        let top = topRow
        content = result
        atFileEdge = nil
        guard case .text(let new) = result else {
            currentChange = 0
            return
        }
        currentChange = min(currentChange, max(new.changes.count - 1, 0))
        if let before, let target = Self.place(of: top, from: before, to: new, layout: layout), target != top {
            placeTarget = target
            placeRequest += 1
        }
    }

    /// Where the row at the top of the view, `top` rows into `old`, is in
    /// `new`: the same line of the old side, which does not change while
    /// the working copy does. `nil` when the view has no reason to move.
    nonisolated static func place(
        of top: Double, from old: TextDiffInfo, to new: TextDiffInfo, layout: Layout
    ) -> Double? {
        let oldNumbers = layout == .unified ? old.unified.map(\.old) : old.rows.map(\.old)
        let newNumbers = layout == .unified ? new.unified.map(\.old) : new.rows.map(\.old)
        let row = Int(top)
        guard top >= 0, oldNumbers.indices.contains(row) else { return nil }
        // The first row at or under the top that has an old line.
        guard let anchor = oldNumbers[row...].firstIndex(where: { $0 != nil }),
              let found = newNumbers.firstIndex(of: oldNumbers[anchor])
        else { return nil }
        return max(Double(found - (anchor - row)) + top - Double(row), 0)
    }

    // MARK: Viewed

    /// Files can be marked viewed: not the copies git difftool hands over.
    var canMarkViewed: Bool { !header.reduced && !header.gitDir.isEmpty }

    var viewedCount: Int { files.count { $0.viewed == .viewed } }

    /// "7 of 13 viewed".
    var viewedSummary: String { "\(viewedCount) of \(files.count) viewed" }

    /// The share of the files that are viewed, from 0 to 1.
    var viewedShare: Double {
        files.isEmpty ? 0 : Double(viewedCount) / Double(files.count)
    }

    /// Marks `file` viewed with the content it has in the list, or
    /// removes its mark.
    func setViewed(_ file: ChangesFile, _ viewed: Bool) {
        guard canMarkViewed, let index = files.firstIndex(of: file) else { return }
        do {
            try source.setViewed(index: file.index, viewed: viewed)
            viewedError = nil
        } catch {
            viewedError = error.message
            return
        }
        files[index].viewed = viewed ? .viewed : .notViewed
        if selected == file { selected = files[index] }
    }

    /// The box of a row or of the file header: viewed, or not.
    func toggleViewed(_ file: ChangesFile) {
        setViewed(file, file.viewed != .viewed)
    }

    /// The key: marks the selected file viewed and goes to the next one
    /// that is not; on a file already viewed, removes the mark.
    func markViewedAndGoOn() async {
        guard let selected, canMarkViewed else { return }
        guard selected.viewed != .viewed else { return setViewed(selected, false) }
        setViewed(selected, true)
        guard viewedError == nil, let index = files.firstIndex(where: { $0.path == selected.path }) else { return }
        // After this file first, then from the top.
        let order = files[(index + 1)...] + files[..<index]
        if let next = order.first(where: { $0.viewed != .viewed }) {
            await select(next)
        }
    }

    var sections: [Section] {
        guard header.grouped else {
            return files.isEmpty ? [] : [Section(title: nil, group: nil, files: files, collapsed: false)]
        }
        let groups: [(ChangeGroup, String)] = [
            (.staged, "Staged"), (.notStaged, "Not staged"), (.untracked, "Untracked"),
        ]
        return groups.compactMap { group, title in
            let members = files.filter { $0.group == group }
            return members.isEmpty
                ? nil
                : Section(title: title, group: group, files: members, collapsed: collapsed.contains(group))
        }
    }

    /// Hides a group's files under its title, or shows them again. The
    /// selected file stays shown in the file area either way.
    func toggle(_ group: ChangeGroup) {
        if collapsed.contains(group) {
            collapsed.remove(group)
        } else {
            collapsed.insert(group)
        }
    }

    // MARK: git difftool's files

    /// "File 2 of 13".
    var queueLabel: String? {
        queue.map { "File \($0.place) of \($0.total)" }
    }

    /// Closes the window: git opens its next file.
    func nextInQueue() {
        leaveQueue(false)
    }

    /// Closes the window and tells git to open no other file.
    func stopQueue() {
        leaveQueue(true)
    }

    // MARK: Shortcut hints

    /// The toolbar's shortcuts, in its order, for the hints while ⌘ is held.
    var toolbarHints: [Shortcut] {
        var shortcuts: [Shortcut] = []
        if canGoToPreviousChange { shortcuts.append(.previousChange) }
        if canGoToNextChange { shortcuts.append(.nextChange) }
        shortcuts.append(.changesLayout)
        if !header.reduced { shortcuts.append(.refreshChanges) }
        return shortcuts
    }

    /// The shortcuts whose control is too small for a badge, or that have
    /// none, for the hint strip while ⌘ is held.
    var hintStrip: [Shortcut] {
        var shortcuts: [Shortcut] = []
        if canGoToPreviousFile { shortcuts.append(.previousFile) }
        if canGoToNextFile { shortcuts.append(.nextFile) }
        if canMarkViewed, selected != nil { shortcuts.append(.viewed) }
        if !header.reduced { shortcuts.append(.openRepository) }
        shortcuts.append(.keyboardShortcuts)
        return shortcuts
    }

    /// "Against HEAD · branch feature/offline-sync".
    var scopeDetail: String {
        guard let branch = header.branch else { return header.detail }
        return "\(header.detail) · branch \(branch)"
    }

    /// "12 files · +96 −31"; binary files count for no line.
    var listSummary: String {
        let added = files.compactMap(\.added).reduce(0, +)
        let removed = files.compactMap(\.removed).reduce(0, +)
        return "\(Self.count(files.count, "file")) · +\(added) −\(removed)"
    }

    /// "4 of 12", the selected file's place in the list.
    var filePosition: String? {
        guard let selected, let index = files.firstIndex(of: selected) else { return nil }
        return "\(index + 1) of \(files.count)"
    }

    var canGoToPreviousFile: Bool { neighbour(-1) != nil }
    var canGoToNextFile: Bool { neighbour(1) != nil }

    func previousFile() async {
        if let file = neighbour(-1) { await select(file) }
    }

    func nextFile() async {
        if let file = neighbour(1) { await select(file) }
    }

    private func neighbour(_ step: Int) -> ChangesFile? {
        guard let selected, let index = files.firstIndex(of: selected) else { return nil }
        let target = index + step
        return files.indices.contains(target) ? files[target] : nil
    }

    // MARK: The file

    /// Shows `file`, read off the main thread. A large file comes without
    /// colours and word marks unless `allMarks` asks for them.
    func select(_ file: ChangesFile?, allMarks: Bool = false, at change: Int = 0) async {
        request += 1
        let asked = request
        selected = file
        currentChange = 0
        atFileEdge = nil
        self.allMarks = allMarks
        if let file { arrived[file.path] = nil }
        // A file reached from the keyboard is shown in the list too.
        if let group = file?.group { collapsed.remove(group) }
        guard let file else {
            content = .none
            return
        }
        // The last file stays on screen while the next one is read, unless
        // reading takes long enough to be noticed.
        let source = source
        let read = Task.detached { Self.read(file, from: source, allMarks: allMarks) }
        let slow = Task { [weak self] in
            try? await Task.sleep(for: .milliseconds(150))
            guard !Task.isCancelled, let self, self.request == asked else { return }
            self.content = .loading
        }
        let result = await read.value
        slow.cancel()
        guard request == asked else { return }
        content = result
        if case .text(let diff) = result, !diff.changes.isEmpty {
            currentChange = min(max(change, 0), diff.changes.count - 1)
        }
        scrollRequest += 1
    }

    /// What the file area shows for `file`.
    private nonisolated static func read(_ file: ChangesFile, from source: ChangesSource, allMarks: Bool) -> Content {
        do {
            switch try source.fileDiff(index: file.index, allMarks: allMarks) {
            case .text(let diff): return .text(diff)
            case .binary(let old, let new): return .binary(old: old, new: new)
            case .submodule: return .submodule
            case .conflicted: return .conflicted
            }
        } catch {
            return .failed(error.message)
        }
    }

    /// Reads the list and the file at `path` (the first one without) on
    /// the calling thread, for previews, which are drawn once.
    func loadNow(select path: String? = nil) {
        loadingFiles = false
        do {
            files = try source.files()
        } catch {
            listError = error.message
            return
        }
        selected = files.first { $0.path == path } ?? files.first
        if let selected {
            content = Self.read(selected, from: source, allMarks: false)
        }
    }

    /// The design's "Turn On for This File": colours and word marks on a
    /// large file.
    func turnOnMarks() async {
        await select(selected, allMarks: true, at: currentChange)
    }

    /// Opens the conflicted file in the merge editor.
    func openInMergeEditor() {
        guard let selected else { return }
        openConflicted(selected.path)
    }

    var diff: TextDiffInfo? {
        if case .text(let diff) = content { return diff }
        return nil
    }

    /// How many rows the file area draws in the current layout.
    var rowCount: Int {
        guard let diff else { return 0 }
        return layout == .unified ? diff.unified.count : diff.rows.count
    }

    /// The widest line number of either side, in digits: the gutter's width.
    var numberDigits: Int {
        guard let diff else { return 1 }
        return max(String(max(diff.oldLines.count, diff.newLines.count)).count, 3)
    }

    // MARK: Changes

    var changeCount: Int { diff?.changes.count ?? 0 }

    /// "Change 2 of 6", or "No change" for a file only renamed.
    var changeCounter: String {
        guard diff != nil else { return "" }
        return changeCount == 0 ? "No change" : "Change \(currentChange + 1) of \(changeCount)"
    }

    var canGoToPreviousChange: Bool { currentChange > 0 || canGoToPreviousFile }
    var canGoToNextChange: Bool { currentChange < changeCount - 1 || canGoToNextFile }

    /// The next block; past the last one, a first press says so and a
    /// second goes to the next file, so the file does not change under a
    /// key held a moment too long.
    ///
    /// A file with no block to stop on (binary, only renamed, a
    /// repository of its own) is passed at the first press.
    func nextChange() async {
        guard changeCount > 0 else { return await nextFile() }
        if currentChange < changeCount - 1 {
            go(to: currentChange + 1)
        } else if atFileEdge == .last {
            await nextFile()
        } else if canGoToNextFile {
            atFileEdge = .last
        }
    }

    func previousChange() async {
        guard changeCount > 0 else {
            if let file = neighbour(-1) { await select(file, at: .max) }
            return
        }
        if currentChange > 0 {
            go(to: currentChange - 1)
        } else if atFileEdge == .first, let file = neighbour(-1) {
            // Arriving from below: on the file's last block.
            await select(file, at: .max)
        } else if canGoToPreviousFile {
            atFileEdge = .first
        }
    }

    /// Goes to a block, from the change map.
    func go(to change: Int) {
        guard change >= 0, change < changeCount else { return }
        currentChange = change
        atFileEdge = nil
        scrollRequest += 1
    }

    /// The row to scroll to for the current block, in the current layout.
    var scrollTarget: Int? {
        guard let diff, diff.changes.indices.contains(currentChange) else { return nil }
        let change = diff.changes[currentChange]
        return Int(layout == .unified ? change.firstLine : change.firstRow)
    }

    /// The rows of the current block in the current layout, to tint its
    /// gutter.
    var currentRows: Range<Int> {
        guard let diff, diff.changes.indices.contains(currentChange) else { return 0..<0 }
        let change = diff.changes[currentChange]
        if layout == .unified {
            let start = Int(change.firstLine)
            return start..<start + Int(change.added + change.removed)
        }
        let start = Int(change.firstRow)
        return start..<start + Int(change.rowCount)
    }

    var changeMap: [MapMark] {
        guard let diff, rowCount > 0 else { return [] }
        let total = Double(rowCount)
        return diff.changes.enumerated().map { index, change in
            let lines = Double(change.added + change.removed)
            let (start, length) = layout == .unified
                ? (Double(change.firstLine), lines)
                : (Double(change.firstRow), Double(change.rowCount))
            return MapMark(
                id: index,
                start: start / total,
                length: length / total,
                removedShare: lines > 0 ? Double(change.removed) / lines : 0,
                current: index == currentChange
            )
        }
    }

    // MARK: Words

    /// "6 changes · +16 −7", under the file.
    var status: String {
        switch content {
        case .text(let diff):
            if let atFileEdge {
                let edge = atFileEdge == .last ? "Last" : "First"
                let file = atFileEdge == .last ? "next" : "previous"
                return "\(edge) change · press again for the \(file) file"
            }
            if let viewedError { return viewedError }
            if diff.changes.isEmpty { return "No change in the content" }
            return "\(Self.count(diff.changes.count, "change")) · +\(diff.added) −\(diff.removed)"
        case .binary: return "Binary file"
        case .conflicted: return "Conflicted"
        case .submodule: return "A repository of its own"
        case .none, .loading, .failed: return ""
        }
    }

    /// "Swift", when Scry has a grammar for the file.
    var language: String? { diff?.language }

    /// The line under the compare bar, when the file needs one.
    var notice: String? {
        guard let selected, let diff else { return nil }
        if diff.large {
            let lines = max(diff.oldLines.count, diff.newLines.count).formatted()
            return "Large file (\(lines) lines): syntax colours and word marks are off, so it stays fast. Added and removed lines are still coloured."
        }
        switch selected.state {
        case .added, .untracked: return "New file: every line is added."
        case .deleted: return "Deleted: every line is removed."
        case .renamed where diff.changes.isEmpty:
            return "Renamed from \(selected.oldPath ?? ""). The content is the same."
        default:
            break
        }
        if diff.oldMissingFinalNewline != diff.newMissingFinalNewline {
            let side = diff.newMissingFinalNewline ? "new" : "old"
            return "The \(side) side's last line has no line ending."
        }
        return nil
    }

    /// What to show in place of the list or the file when there is none.
    var emptyMessage: String {
        "Nothing to show. No file differs: \(header.sentence.prefix(1).lowercased() + header.sentence.dropFirst())."
    }

    /// The letter in a file's badge: never colour alone.
    static func badge(_ file: ChangesFile) -> String {
        if file.binary { return "B" }
        switch file.state {
        case .added: return "A"
        case .modified: return "M"
        case .deleted: return "D"
        case .renamed: return "R"
        case .untracked: return "U"
        case .typeChanged: return "T"
        case .conflicted: return "!"
        }
    }

    /// The badge's word, for its tooltip and VoiceOver.
    static func stateName(_ file: ChangesFile) -> String {
        if file.binary { return "Binary" }
        switch file.state {
        case .added: return "Added"
        case .modified: return "Modified"
        case .deleted: return "Deleted"
        case .renamed: return "Renamed"
        case .untracked: return "Untracked"
        case .typeChanged: return "Type changed"
        case .conflicted: return "Conflicted"
        }
    }

    /// The second line of a file's row: its folder, then what is unusual
    /// about it.
    static func detail(_ file: ChangesFile, arrival: Arrival? = nil) -> String {
        // What happened since the file was viewed or listed comes first.
        if file.viewed == .changedSince { return "changed since viewed" }
        var parts: [String] = []
        if arrival == .new { parts.append("new") }
        if !file.directory.isEmpty { parts.append(file.directory) }
        if let oldPath = file.oldPath {
            let oldName = URL(fileURLWithPath: oldPath).lastPathComponent
            // The old name when only the name changed, the old path when
            // the file moved.
            let sameFolder = oldPath.dropLast(oldName.count) == file.directory
            parts.append("was \(sameFolder ? oldName : oldPath)")
        }
        if file.binary { parts.append("binary") }
        if file.submodule { parts.append("a repository of its own") }
        if file.state == .conflicted { parts.append("conflicted") }
        return parts.joined(separator: " · ")
    }

    /// What the "partly staged" tag means, for its tooltip.
    static let partlyStagedHelp =
        "Part of this file's change is staged, part is not. Scry shows the whole change against HEAD, in one entry."

    /// "1.4 KB", or "nothing" for a side the file is not on.
    static func size(_ bytes: UInt64?) -> String {
        guard let bytes else { return "nothing" }
        return ByteCountFormatter.string(fromByteCount: Int64(bytes), countStyle: .file)
    }

    private static func count(_ count: Int, _ noun: String) -> String {
        count == 1 ? "1 \(noun)" : "\(count) \(noun)s"
    }
}
