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
    /// The sizes and the last commit of the file at `index`.
    func fileFacts(index: UInt32) -> FileFacts
    /// The blame of the new side of the file at `index` of the last list,
    /// not read yet.
    func blameSource(index: UInt32) throws -> BlameSource
    /// Where Explain stands on the lines of the file at `index`.
    func lineExplanations(index: UInt32) -> LineExplanations
    /// Asks about lines `first...last` of the file at `index`; gives the
    /// ask's id.
    func askLines(index: UInt32, first: UInt32, last: UInt32) throws -> String
    func cancelLines(index: UInt32, id: String) throws
    func forgetLines(index: UInt32, id: String) throws
}

extension ChangesSource {
    func fileFacts(index: UInt32) -> FileFacts {
        FileFacts(oldSize: nil, newSize: nil, lastCommit: nil)
    }

    func lineExplanations(index: UInt32) -> LineExplanations {
        LineExplanations(available: false, watching: false, items: [])
    }

    func askLines(index: UInt32, first: UInt32, last: UInt32) throws -> String {
        throw ScryError.Refused(message: "Explain needs a text file in a git repository.")
    }

    func cancelLines(index: UInt32, id: String) throws {}
    func forgetLines(index: UInt32, id: String) throws {}
}

extension ChangeSet: ChangesSource {
    func blameSource(index: UInt32) throws -> BlameSource {
        try blame(index: index)
    }
}

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
        /// What the list shows under the title: the files, under their
        /// folders when the list is by folder.
        let rows: [Row]
        var id: String { title ?? "" }
    }

    /// How the list shows its files: under their folders, or one after
    /// the other with the folder under each name.
    enum ListMode: String {
        case folders, flat
    }

    /// A folder of the list, or several in one row when each holds only
    /// the next.
    struct Folder: Equatable {
        /// The section's title and the path: the same folder can be in
        /// two groups.
        let key: String
        /// "ios/Sources/Views" when the first two hold nothing else.
        let name: String
        let path: String
        let depth: Int
        /// The files under it, however deep.
        let count: Int
        /// Their lines added and removed; binary files count for none.
        let added: Int
        let removed: Int
        let collapsed: Bool
    }

    /// A block of the open file, for the inspector's list.
    struct ChangeRow: Identifiable, Equatable {
        /// Its place among the file's blocks, from 0.
        let id: Int
        /// The number of its first line.
        let line: Int
        /// Its first line, without the indentation.
        let text: String
        let added: Int
        let removed: Int
    }

    /// A line of the inspector's "File": "Size", "1.4 KB → 1.9 KB".
    struct Fact: Identifiable, Equatable {
        let title: String
        let value: String
        /// More in the tooltip: the last commit's subject and author.
        var help: String?
        var id: String { title }
    }

    enum Row: Identifiable, Equatable {
        case folder(Folder)
        case file(ChangesFile, depth: Int)

        var id: String {
            switch self {
            case let .folder(folder): "folder \(folder.key)"
            case let .file(file, _): "file \(file.index)"
            }
        }
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
    /// The folders whose files are hidden under their name, by key.
    private(set) var collapsedFolders: Set<String> = []
    var listMode: ListMode {
        didSet {
            guard listMode != oldValue else { return }
            defaults.set(listMode.rawValue, forKey: Self.listModeKey)
        }
    }

    /// The user wants the inspector beside the file: the window's choice.
    var inspectorShown: Bool {
        get { inspectorSwitch.isOn }
        set { inspectorSwitch.isOn = newValue }
    }

    let inspectorSwitch: InspectorSwitch

    /// The open file's sizes and last commit, once read.
    private(set) var facts: FileFacts?

    /// The file's place among those `git difftool` opens in turn.
    let queue: Queue?
    var layout: Layout {
        didSet {
            guard layout != oldValue else { return }
            defaults.set(layout.rawValue, forKey: Self.layoutKey)
            // The other layout has other rows.
            lineRow = selection.flatMap { row(ofLine: $0.lowerBound) }
            scrollRequest += 1
        }
    }

    enum FileEdge: Equatable {
        case first, last
    }

    static let layoutKey = "changesLayout"
    static let listModeKey = "changesListMode"

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
    /// Opens the history of a file of the list.
    @ObservationIgnored private let openHistory: ((ChangesFile) -> Void)?
    /// What to say when no file differs, in place of the usual words.
    @ObservationIgnored private let emptyText: String?
    /// Blame is shown, for the whole window.
    let blameSwitch: BlameSwitch
    /// The open file's blame, while blame is shown.
    private(set) var blame: BlameReading?
    @ObservationIgnored private var blameTask: Task<Void, Never>?
    /// Goes to what a commit did to a file, from a line of the blame.
    @ObservationIgnored private let openBlamed: ((BlameCommitInfo, ChangesFile) -> Void)?
    /// The lines selected in the gutter, from 1: of the new side, or of
    /// the old one in a deleted file.
    private(set) var selection: ClosedRange<Int>?
    /// The line a shift-click or a drag extends the selection from.
    @ObservationIgnored private var selectionAnchor: Int?
    /// The pointer is down on a line number.
    @ObservationIgnored private var pressing = false
    /// Where Explain stands on the open file's lines, as last read.
    private(set) var explanations: LineExplanations?
    /// When `explanations` was read: the waiting time counts from it.
    private(set) var explainNow = Date()
    /// Why the last ask or cancel failed.
    private(set) var explainError: String?
    /// The explanation shown whole in the inspector, by its id.
    private(set) var openExplanation: String?
    /// The long answers the user opened, by id.
    private(set) var expandedExplanations: Set<String> = []
    /// The row to scroll to for an explanation's lines, until a change is
    /// gone to.
    @ObservationIgnored private var lineRow: Int?
    /// No for a preview, which shows what it was given.
    @ObservationIgnored var pollsExplanations = true

    init(
        source: ChangesSource,
        defaults: UserDefaults = .standard,
        queue: Queue? = nil,
        watch: ((ChangesHeader) -> AsyncStream<Void>)? = nil,
        now: @escaping () -> Date = Date.init,
        emptyMessage: String? = nil,
        leaveQueue: @escaping (Bool) -> Void = { _ in },
        openConflicted: @escaping (String) -> Void = { _ in },
        openHistory: ((ChangesFile) -> Void)? = nil,
        blame: BlameSwitch = BlameSwitch(),
        inspector: InspectorSwitch? = nil,
        openBlamed: ((BlameCommitInfo, ChangesFile) -> Void)? = nil
    ) {
        inspectorSwitch = inspector ?? InspectorSwitch(defaults: defaults)
        self.openHistory = openHistory
        blameSwitch = blame
        self.openBlamed = openBlamed
        emptyText = emptyMessage
        self.watch = watch
        self.now = now
        self.source = source
        self.defaults = defaults
        self.queue = queue
        self.leaveQueue = leaveQueue
        self.openConflicted = openConflicted
        header = source.header()
        layout = defaults.string(forKey: Self.layoutKey).flatMap(Layout.init) ?? .unified
        listMode = defaults.string(forKey: Self.listModeKey).flatMap(ListMode.init) ?? .folders
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
            // Read again, the file stays on the block it was on.
            await select(kept ?? listed.first, at: kept == nil ? 0 : currentChange)
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
        let oldIndex = old.flatMap { listed.firstIndex(of: $0) }
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
            let fresh = listed
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
        readFacts(file)
        atFileEdge = nil
        restartBlame()
        // The lines moved under the selection.
        clearSelection()
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
        if viewed { closeViewedFolders(above: file) }
    }

    /// The files under a folder of the list, however deep, in its group.
    func files(in folder: Folder) -> [ChangesFile] {
        let section = folder.key.dropLast(folder.path.count + 1)
        let group = Self.groups.first { $0.1 == section }?.0
        return files.filter { file in
            (!header.grouped || file.group == group) && file.path.hasPrefix(folder.path + "/")
        }
    }

    /// The folder's menu: marks every file under it viewed, or removes
    /// their marks. Stops at the first file that cannot be marked.
    func setViewed(in folder: Folder, _ viewed: Bool) {
        for file in files(in: folder) where (file.viewed == .viewed) != viewed {
            setViewed(file, viewed)
            if viewedError != nil { return }
        }
    }

    /// A group's menu: marks every file of the group viewed, or removes
    /// their marks.
    func setViewed(in group: ChangeGroup, _ viewed: Bool) {
        for file in files where file.group == group && (file.viewed == .viewed) != viewed {
            setViewed(file, viewed)
            if viewedError != nil { return }
        }
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
        // After this file first, then from the top, in the list's order.
        let files = listed
        guard viewedError == nil, let index = files.firstIndex(where: { $0.path == selected.path }) else { return }
        let order = files[(index + 1)...] + files[..<index]
        if let next = order.first(where: { $0.viewed != .viewed }) {
            await select(next)
        }
    }

    var sections: [Section] {
        guard header.grouped else {
            return files.isEmpty
                ? []
                : [Section(title: nil, group: nil, files: files, collapsed: false, rows: rows(files, in: ""))]
        }
        return Self.groups.compactMap { group, title in
            let members = files.filter { $0.group == group }
            return members.isEmpty
                ? nil
                : Section(
                    title: title, group: group, files: members, collapsed: collapsed.contains(group),
                    rows: rows(members, in: title)
                )
        }
    }

    private static let groups: [(ChangeGroup, String)] = [
        (.staged, "Staged"), (.notStaged, "Not staged"), (.untracked, "Untracked"),
    ]

    /// The files in the order the list shows them, hidden ones included:
    /// the window opens on the first, and previous and next file follow
    /// that order.
    var listed: [ChangesFile] {
        guard header.grouped else { return files }
        // A file in no group is not listed; it still has its place.
        return Self.groups.flatMap { group, _ in files.filter { $0.group == group } }
            + files.filter { $0.group == nil }
    }

    /// Shows the folders above a file again.
    private func openFolders(of file: ChangesFile) {
        guard !collapsedFolders.isEmpty else { return }
        for folder in folders(above: file) {
            collapsedFolders.remove(folder.key)
        }
    }

    /// The folders above a file, from the outermost, with their keys.
    private func folders(above file: ChangesFile) -> [(key: String, path: String)] {
        let section = header.grouped ? Self.groups.first { $0.0 == file.group }?.1 ?? "" : ""
        var path = ""
        return file.directory.split(separator: "/").map { name in
            path += path.isEmpty ? String(name) : "/\(name)"
            return ("\(section)/\(path)", path)
        }
    }

    /// Hides the folders above a file whose files are now all viewed:
    /// the list keeps what is left to read.
    private func closeViewedFolders(above file: ChangesFile) {
        guard listMode == .folders else { return }
        for folder in folders(above: file) {
            let inside = files.filter { other in
                (!header.grouped || other.group == file.group) && other.path.hasPrefix(folder.path + "/")
            }
            if inside.allSatisfy({ $0.viewed == .viewed }) { collapsedFolders.insert(folder.key) }
        }
    }

    /// A section's rows: its files, under their folders when the list is
    /// by folder. Folders and files keep the order the files came in.
    private func rows(_ files: [ChangesFile], in section: String) -> [Row] {
        guard listMode == .folders else { return files.map { .file($0, depth: 0) } }
        let root = FolderNode(name: "", path: "")
        for file in files {
            var node = root
            for name in file.directory.split(separator: "/") {
                node = node.folder(String(name))
            }
            node.children.append(.file(file))
        }
        var rows: [Row] = []
        add(root, depth: 0, section: section, to: &rows)
        return rows
    }

    private func add(_ node: FolderNode, depth: Int, section: String, to rows: inout [Row]) {
        for child in node.children {
            switch child {
            case let .file(file):
                rows.append(.file(file, depth: depth))
            case var .folder(folder):
                // A folder that holds only a folder shares its row.
                var name = folder.name
                while folder.children.count == 1, case let .folder(only) = folder.children[0] {
                    name += "/" + only.name
                    folder = only
                }
                let key = "\(section)/\(folder.path)"
                let collapsed = collapsedFolders.contains(key)
                let files = folder.files
                rows.append(.folder(Folder(
                    key: key, name: name, path: folder.path, depth: depth, count: files.count,
                    added: files.reduce(0) { $0 + Int($1.added ?? 0) },
                    removed: files.reduce(0) { $0 + Int($1.removed ?? 0) },
                    collapsed: collapsed
                )))
                if !collapsed { add(folder, depth: depth + 1, section: section, to: &rows) }
            }
        }
    }

    /// Hides a folder's files under its name, or shows them again.
    func toggle(_ folder: Folder) {
        if collapsedFolders.contains(folder.key) {
            collapsedFolders.remove(folder.key)
        } else {
            collapsedFolders.insert(folder.key)
        }
    }

    /// The sidebar's button: by folder, or flat.
    func toggleListMode() {
        listMode = listMode == .folders ? .flat : .folders
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

    // MARK: History

    /// The selected file's history can be opened: not from the copies
    /// git difftool hands over, nor from a history.
    var canShowHistory: Bool { header.hasHistory && openHistory != nil && selected != nil }

    /// Replaces the list with the selected file's history.
    func showHistory() {
        guard canShowHistory, let selected else { return }
        openHistory?(selected)
    }

    // MARK: Blame

    /// The open file has lines to blame: a text file with a new side, in
    /// a repository.
    var canBlame: Bool {
        !header.reduced && diff != nil && selected?.state != .deleted
    }

    /// The key and the toolbar's button: shows the blame gutter, or
    /// hides it.
    func toggleBlame() {
        guard canBlame || blameSwitch.isOn else { return }
        blameSwitch.isOn.toggle()
        updateBlame()
    }

    /// Reads the open file's blame when blame is shown and it is not
    /// read yet; drops it when blame is hidden.
    func updateBlame() {
        guard blameSwitch.isOn else { return dropBlame() }
        guard blame == nil, canBlame, let selected, let diff,
              let read = try? source.blameSource(index: selected.index)
        else { return }
        let blame = BlameReading(source: read, lineCount: diff.newLines.count)
        self.blame = blame
        blameTask = Task { await blame.run() }
    }

    /// The file area shows other lines: their blame is another one.
    private func restartBlame() {
        dropBlame()
        updateBlame()
    }

    private func dropBlame() {
        blameTask?.cancel()
        blameTask = nil
        blame?.stop()
        blame = nil
    }

    /// A click on a line of the blame: what its commit did to the file.
    func showBlamed(_ commit: BlameCommitInfo) {
        guard let selected else { return }
        openBlamed?(commit, selected)
    }

    // MARK: Explain the selected lines

    /// One explanation of the open file and what its block shows.
    struct ExplainedLines: Identifiable, Equatable {
        let item: LineExplanation
        let state: InspectorViewModel.Explain
        var id: String { item.id }
        /// "Lines 10–18".
        var label: String { ChangesViewModel.linesLabel(Int(item.first)...Int(item.last)) }
    }

    /// The strip at the left of explained lines.
    enum ExplainStrip: Equatable {
        /// Asked, not answered yet: dashed.
        case waiting
        case answered
        /// The lines changed since, or the agent could not explain: grey,
        /// dashed.
        case stale
    }

    /// Lines can be selected to ask about them: a text file, not the
    /// copies git difftool hands over, with Explain on in Settings.
    var canSelectLines: Bool {
        !header.reduced && diff != nil && InspectorViewModel.explainEnabled(in: defaults)
    }

    /// The lines selected are the old side's: a deleted file has no new
    /// side.
    var selectsOldSide: Bool { selected?.state == .deleted }

    /// The pointer went down on the line number of `row`, or moved
    /// `rowDelta` rows from it while down. With `extend` (shift), the
    /// selection goes on from where it started.
    func pressLine(row: Int, rowDelta: Int = 0, extend: Bool = false) {
        guard canSelectLines else { return }
        if !pressing {
            guard let line = selectableLine(nearRow: row) else { return }
            pressing = true
            if !extend || selectionAnchor == nil { selectionAnchor = line }
        }
        guard let anchor = selectionAnchor, let line = selectableLine(nearRow: row + rowDelta) else { return }
        let range = min(anchor, line)...max(anchor, line)
        guard range != selection else { return }
        selection = range
        if let touched = currentExplanation { openExplanation = touched.id }
    }

    func releaseLine() {
        pressing = false
    }

    /// Esc: nothing is selected any more.
    func clearSelection() {
        selection = nil
        selectionAnchor = nil
        pressing = false
    }

    /// The selectable line of `row`, or of the nearest row that has one.
    private func selectableLine(nearRow row: Int) -> Int? {
        guard let diff, rowCount > 0 else { return nil }
        let old = selectsOldSide
        let unified = layout == .unified
        func number(_ row: Int) -> Int? {
            guard row >= 0, row < rowCount else { return nil }
            let number = unified
                ? (old ? diff.unified[row].old : diff.unified[row].new)
                : (old ? diff.rows[row].old : diff.rows[row].new)
            return number.map(Int.init)
        }
        let start = min(max(row, 0), rowCount - 1)
        for distance in 0..<rowCount {
            if let line = number(start - distance) ?? number(start + distance) { return line }
        }
        return nil
    }

    /// The row showing `line` of the selectable side, in the layout.
    private func row(ofLine line: Int) -> Int? {
        guard let diff else { return nil }
        let old = selectsOldSide
        let number = UInt32(line)
        return layout == .unified
            ? diff.unified.firstIndex { (old ? $0.old : $0.new) == number }
            : diff.rows.firstIndex { (old ? $0.old : $0.new) == number }
    }

    /// Reads where Explain stands on the open file's lines again, and
    /// whether a session watches: the window does it every second, since
    /// the agent answers from outside.
    func refreshExplanations() async {
        guard canSelectLines, let selected else {
            explanations = nil
            return
        }
        let source = source
        let asked = request
        let read = await Task.detached { source.lineExplanations(index: selected.index) }.value
        guard request == asked else { return }
        explainNow = now()
        if read != explanations { explanations = read }
    }

    /// The explanations of the open file, ordered by line; none with
    /// Explain off.
    var explainedLines: [ExplainedLines] {
        guard canSelectLines, let explanations else { return [] }
        return explanations.items.map { item in
            ExplainedLines(item: item, state: InspectorViewModel.explain(
                askedAt: item.askedAt, answer: item.answer, watching: explanations.watching,
                stale: item.stale, now: explainNow
            ))
        }
    }

    /// The explanation the selection touches: asking again extends it.
    var currentExplanation: LineExplanation? {
        guard let selection else { return nil }
        return explanations?.items.first { Int($0.first) <= selection.upperBound && selection.lowerBound <= Int($0.last) }
    }

    /// Whether a Claude Code session watches the repository, for the
    /// status bar; `nil` where lines cannot be asked about, or once
    /// Settings › Agent found Scry's plugin off.
    var watching: Bool? {
        guard canSelectLines, ClaudeCodePlugin.isOn(in: defaults), let explanations, explanations.available
        else { return nil }
        return explanations.watching
    }

    /// The inspector has Explain to show: lines are selected, or the
    /// file has explanations.
    var showsExplainInspector: Bool {
        guard canSelectLines, let explanations, explanations.available else { return false }
        return selection != nil || !explanations.items.isEmpty
    }

    // MARK: Inspector

    /// The window can have an inspector: not git difftool's, which has
    /// one file and nothing to say of it.
    var canShowInspector: Bool { !header.reduced }

    /// The inspector is beside the file.
    var showsInspector: Bool { canShowInspector && inspectorShown && selected != nil }

    /// With lines selected the inspector is about them; otherwise about
    /// the file, with its explanations below.
    var inspectorIsAboutLines: Bool { showsExplainInspector && selection != nil }

    /// The key and the toolbar's button.
    func toggleInspector() {
        guard canShowInspector else { return }
        inspectorShown.toggle()
    }

    /// Reads the open file's sizes and last commit, off the main thread.
    private func readFacts(_ file: ChangesFile) {
        guard canShowInspector else { return }
        let source = source
        let asked = request
        Task { [weak self] in
            let facts = await Task.detached { source.fileFacts(index: file.index) }.value
            guard let self, self.request == asked else { return }
            self.facts = facts
        }
    }

    /// "File": the state, the lines, the sizes and the last commit, each
    /// when there is one to say.
    var fileFacts: [Fact] {
        guard let selected else { return [] }
        var rows = [Fact(title: "State", value: Self.stateName(selected))]
        if let added = selected.added, let removed = selected.removed, added + removed > 0 {
            // As the list's counts: no "−0".
            let lines = [added > 0 ? "+\(added)" : nil, removed > 0 ? "−\(removed)" : nil]
            rows.append(Fact(title: "Lines", value: lines.compactMap { $0 }.joined(separator: " ")))
        }
        if let facts {
            let sizes = [facts.oldSize, facts.newSize].compactMap { $0 }.map { Self.size($0) }
            if !sizes.isEmpty { rows.append(Fact(title: "Size", value: sizes.joined(separator: " → "))) }
            if let commit = facts.lastCommit {
                let when = ExplainBlock.relative(UInt64(max(commit.time, 0)) * 1000)
                rows.append(Fact(
                    title: "Last commit", value: "\(commit.shortId) · \(when)",
                    help: "\(commit.subject)\n\(commit.author)"
                ))
            }
        }
        return rows
    }

    /// "Changes in this file": one row per block.
    var changeRows: [ChangeRow] {
        guard let diff else { return [] }
        return diff.changes.enumerated().map { index, change in
            let first = Int(change.firstLine)
            let line = diff.unified.indices.contains(first) ? diff.unified[first] : nil
            let number = line.flatMap { $0.new ?? $0.old }.map(Int.init) ?? 0
            // The block's first line that is not blank: an empty line says
            // nothing of where the block is.
            let block = diff.unified.dropFirst(first).prefix(Int(change.added + change.removed))
            let text = block.lazy.compactMap { line -> String? in
                let side = line.kind == .removed ? diff.oldLines : diff.newLines
                let at = Int((line.kind == .removed ? line.old : line.new) ?? 0) - 1
                guard side.indices.contains(at) else { return nil }
                let text = side[at].text.trimmingCharacters(in: .whitespaces)
                return text.isEmpty ? nil : text
            }.first ?? ""
            return ChangeRow(
                id: index, line: number, text: text, added: Int(change.added), removed: Int(change.removed)
            )
        }
    }

    /// The strip beside `line`, and its explanation's id.
    func strip(line: Int) -> (kind: ExplainStrip, id: String)? {
        guard canSelectLines,
              let item = explanations?.items.first(where: { Int($0.first) <= line && line <= Int($0.last) })
        else { return nil }
        if item.stale || item.answer?.failed == true { return (.stale, item.id) }
        return (item.askedAt != nil ? .waiting : .answered, item.id)
    }

    /// The explanation is shown whole in the inspector.
    func isOpen(_ explained: ExplainedLines) -> Bool {
        openExplanation == explained.id || explanations?.items.count == 1
            || (openExplanation == nil && currentExplanation?.id == explained.id)
    }

    /// ⌃⌘E and the Explain buttons: asks about the selected lines, again
    /// when they are explained already.
    func explainSelection() async {
        guard let selection else { return }
        // The answer is read in the inspector.
        inspectorShown = true
        await ask(selection)
    }

    func askAgain(_ item: LineExplanation) async {
        await ask(Int(item.first)...Int(item.last))
    }

    private func ask(_ lines: ClosedRange<Int>) async {
        guard canSelectLines, let selected else { return }
        do {
            openExplanation = try source.askLines(
                index: selected.index, first: UInt32(lines.lowerBound), last: UInt32(lines.upperBound)
            )
            explainError = nil
        } catch {
            explainError = error.message
        }
        await refreshExplanations()
        // The ask covers the explanations it touched: so does the selection.
        if selection != nil, let item = explanations?.items.first(where: { $0.id == openExplanation }) {
            selection = Int(item.first)...Int(item.last)
        }
    }

    /// Takes back a waiting ask; its last answer stays.
    func cancelExplain(_ item: LineExplanation) async {
        await change { try self.source.cancelLines(index: $0, id: item.id) }
    }

    /// Removes an explanation.
    func forgetExplanation(_ item: LineExplanation) async {
        await change { try self.source.forgetLines(index: $0, id: item.id) }
    }

    private func change(_ action: (UInt32) throws -> Void) async {
        guard let selected else { return }
        do {
            try action(selected.index)
            explainError = nil
        } catch {
            explainError = error.message
        }
        await refreshExplanations()
    }

    /// A click on an explanation's lines in the inspector: selects them
    /// and scrolls there.
    func show(_ item: LineExplanation) {
        selection = Int(item.first)...Int(item.last)
        selectionAnchor = Int(item.first)
        openExplanation = item.id
        inspectorShown = true
        lineRow = row(ofLine: Int(item.first))
        scrollRequest += 1
    }

    /// A click on a folded explanation, or on its strip in the code:
    /// shows it whole.
    func open(_ id: String) {
        openExplanation = id
        inspectorShown = true
    }

    func toggleExpanded(_ id: String) {
        expandedExplanations.formSymmetricDifference([id])
    }

    /// "Line 10", "Lines 10–18".
    nonisolated static func linesLabel(_ lines: ClosedRange<Int>) -> String {
        lines.count == 1 ? "Line \(lines.lowerBound)" : "Lines \(lines.lowerBound)–\(lines.upperBound)"
    }

    /// Shows a selection and explanations given at once, for previews.
    func showNow(selection: ClosedRange<Int>?, explanations: LineExplanations, open: String? = nil) {
        pollsExplanations = false
        self.selection = selection
        self.explanations = explanations
        openExplanation = open
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
        if canBlame { shortcuts.append(.blame) }
        if canShowHistory { shortcuts.append(.history) }
        if canSelectLines, selection != nil { shortcuts.append(.explainLines) }
        if !header.reduced { shortcuts.append(.refreshChanges) }
        if canShowInspector { shortcuts.append(.toggleInspector) }
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
        let files = listed
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
        let files = listed
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
        clearSelection()
        lineRow = nil
        explanations = nil
        openExplanation = nil
        explainError = nil
        self.allMarks = allMarks
        facts = nil
        if let file { arrived[file.path] = nil }
        // A file reached from the keyboard is shown in the list too.
        if let group = file?.group { collapsed.remove(group) }
        if let file { openFolders(of: file) }
        guard let file else {
            content = .none
            restartBlame()
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
        restartBlame()
        readFacts(file)
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
        if blameSwitch.isOn, canBlame, let selected, let diff,
           let read = try? source.blameSource(index: selected.index) {
            read.read()
            let blame = BlameReading(source: read, lineCount: diff.newLines.count)
            blame.take()
            self.blame = blame
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
        lineRow = nil
        scrollRequest += 1
    }

    /// The row to scroll to for the current block, in the current layout.
    var scrollTarget: Int? {
        if let lineRow { return lineRow }
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
            let reading = (blame?.status.map { " · \($0)" } ?? "")
                + (selection.map { " · \(Self.linesLabel($0)) selected" } ?? "")
            if diff.changes.isEmpty { return "No change in the content" + reading }
            return "\(Self.count(diff.changes.count, "change")) · +\(diff.added) −\(diff.removed)" + reading
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
        if let emptyText { return emptyText }
        return "Nothing to show. No file differs: \(header.sentence.prefix(1).lowercased() + header.sentence.dropFirst())."
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
    static func detail(_ file: ChangesFile, arrival: Arrival? = nil, folder: Bool = true) -> String {
        // What happened since the file was viewed or listed comes first.
        if file.viewed == .changedSince { return "changed since viewed" }
        var parts: [String] = []
        if arrival == .new { parts.append("new") }
        // Under its folder's row, the folder is already said.
        if folder, !file.directory.isEmpty { parts.append(file.directory) }
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

/// A folder while the list is built: its folders and files, in the
/// order they came.
private final class FolderNode {
    enum Child {
        case folder(FolderNode)
        case file(ChangesFile)
    }

    let name: String
    let path: String
    var children: [Child] = []
    private var folders: [String: FolderNode] = [:]

    init(name: String, path: String) {
        self.name = name
        self.path = path
    }

    /// The folder of that name in this one, added when it is new.
    func folder(_ name: String) -> FolderNode {
        if let known = folders[name] { return known }
        let node = FolderNode(name: name, path: path.isEmpty ? name : "\(path)/\(name)")
        folders[name] = node
        children.append(.folder(node))
        return node
    }

    /// The files under it, however deep.
    var files: [ChangesFile] {
        children.flatMap { child in
            switch child {
            case let .folder(folder): folder.files
            case let .file(file): [file]
            }
        }
    }
}
