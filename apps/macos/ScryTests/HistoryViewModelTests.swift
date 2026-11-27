import Foundation
import Testing
@testable import Scry

/// A file's history a test fills in, in place of a repository.
final class FakeHistory: HistorySource, @unchecked Sendable {
    /// The points loaded at first, and those the next page adds.
    var points: [HistoryPoint]
    var older: [HistoryPoint] = []
    var total: UInt32?
    var failure: ScryError?
    private(set) var started = 0
    private(set) var asked: [(index: UInt32, pin: HistoryPin?)] = []
    private(set) var counted: [HistoryPin] = []

    init(points: [HistoryPoint]) {
        self.points = points
    }

    func header() -> HistoryHeader {
        HistoryHeader(
            repository: "offline-sync", root: "/r", path: "Sources/SyncEngine.swift", name: "SyncEngine.swift",
            me: "Alex Martin", fromHead: true
        )
    }

    func start() throws -> HistoryList {
        started += 1
        if let failure { throw failure }
        return HistoryList(points: points, more: !older.isEmpty)
    }

    func loadOlder() throws -> HistoryList {
        points += older
        older = []
        return HistoryList(points: points, more: false)
    }

    func count() throws -> UInt32 {
        total ?? UInt32(points.count { $0.kind == .commit } + older.count)
    }

    func changes(index: UInt32, pin: HistoryPin?) throws -> ChangesSource {
        asked.append((index, pin))
        guard let point = points.first(where: { $0.index == index }) else {
            throw ScryError.Refused(message: "This point is no longer in the history.")
        }
        // The pin itself has nothing to show.
        let isPin = pin == .point(index: index) || (pin == .workingCopy && point.kind == .notCommitted)
        let source = FakeChanges(files: isPin ? [] : [FakeChanges.file(0, point.path, group: nil)])
        source.diffs[0] = FakeChanges.text(lines: 10, changesAt: [2])
        source.singleFile = true
        source.hasHistory = false
        return source
    }

    func pinnedCounts(index: UInt32, pin: HistoryPin) throws -> PointCounts {
        counted.append(pin)
        return PointCounts(added: index, removed: 0, same: pin == .point(index: index))
    }

    static func commit(
        _ index: UInt32, _ subject: String, author: String = "Sam Lee", time: Int64 = 1_000, head: Bool = false,
        path: String = "Sources/SyncEngine.swift", oldPath: String? = nil
    ) -> HistoryPoint {
        HistoryPoint(
            index: index, kind: .commit, id: "id\(subject)", shortId: "id\(index)", subject: subject, author: author,
            time: time, path: path, oldPath: oldPath, added: 2, removed: 1, head: head
        )
    }

    static func uncommitted(_ index: UInt32, staged: Bool = false) -> HistoryPoint {
        HistoryPoint(
            index: index, kind: staged ? .staged : .notCommitted, id: "", shortId: "", subject: "", author: "",
            time: 0, path: "Sources/SyncEngine.swift", oldPath: nil, added: 1, removed: 0, head: false
        )
    }
}

@MainActor
struct HistoryViewModelTests {
    /// What is not committed, then three commits, the last one a rename.
    private func sample() -> FakeHistory {
        FakeHistory(points: [
            FakeHistory.uncommitted(0),
            FakeHistory.commit(1, "Retry", author: "Alex Martin", head: true),
            FakeHistory.commit(2, "Rename", oldPath: "Sources/SyncManager.swift"),
            FakeHistory.commit(3, "Start", path: "Sources/SyncManager.swift"),
        ])
    }

    private func viewModel(_ source: FakeHistory) -> HistoryViewModel {
        HistoryViewModel(source: source, defaults: .isolated, now: { Date(timeIntervalSince1970: 1_000 + 3 * 86_400) })
    }

    /// The history opens on its newest point, with the file at that
    /// point read, and says how many commits touched the file.
    @Test func opensOnTheNewestPointWithItsFile() async {
        let source = sample()
        let history = viewModel(source)
        #expect(history.loading)
        await history.load()
        #expect(!history.loading)
        #expect(history.points.count == 4)
        #expect(history.selected?.index == 0)
        #expect(history.file?.selected?.path == "Sources/SyncEngine.swift")
        #expect(history.file?.header.singleFile == true)
        #expect(history.summary == "3 commits · renames followed")
        #expect(history.loadedSummary == nil)
        #expect(source.asked.map(\.index) == [0])
        #expect(source.asked.first?.pin == nil)
        // Coming back to the history does not read it again.
        await history.loadOnce()
        #expect(source.started == 1)
    }

    /// A point says what it is, who wrote it ("You" for the user's own
    /// commits) and how long ago.
    @Test func aPointSaysWhoAndWhen() async {
        let history = viewModel(sample())
        await history.load()
        #expect(HistoryViewModel.title(history.points[0]) == "Not committed")
        #expect(HistoryViewModel.title(history.points[1]) == "Retry")
        #expect(history.detail(history.points[1]) == "id1 · You · 3 d")
        #expect(history.detail(history.points[2]) == "id2 · Sam Lee · 3 d")
        let now = Date(timeIntervalSince1970: 100_000_000)
        #expect(HistoryViewModel.age(of: 100_000_000 - 30, now: now) == "now")
        #expect(HistoryViewModel.age(of: 100_000_000 - 300, now: now) == "5 min")
        #expect(HistoryViewModel.age(of: 100_000_000 - 7_200, now: now) == "2 h")
        #expect(HistoryViewModel.age(of: 100_000_000 - 70 * 86_400, now: now) == "2 mo")
        #expect(HistoryViewModel.age(of: 100_000_000 - 800 * 86_400, now: now) == "2 y")
        #expect(HistoryViewModel.age(of: 100_000_000 + 50, now: now) == "now")
    }

    /// Older and newer step through the points; at the end of the page,
    /// older loads the next one first.
    @Test func olderAndNewerStepThroughThePoints() async {
        let source = sample()
        source.older = [FakeHistory.commit(4, "Before"), FakeHistory.commit(5, "First")]
        let history = viewModel(source)
        await history.load()
        #expect(history.more)
        #expect(history.loadedSummary == "3 of 5 loaded")
        #expect(history.summary == "5 commits · renames followed")
        #expect(!history.canGoToNewer && history.canGoToOlder)
        await history.older()
        #expect(history.selected?.index == 1)
        await history.newer()
        #expect(history.selected?.index == 0)
        await history.select(history.points[3])
        #expect(history.canGoToOlder)
        await history.older()
        #expect(history.points.count == 6)
        #expect(history.selected?.index == 4)
        #expect(!history.more && history.loadedSummary == nil)
        await history.older()
        #expect(history.selected?.index == 5)
        #expect(!history.canGoToOlder)
        await history.older()
        #expect(history.selected?.index == 5)
    }

    /// The inspector is the window's: the toolbar's button and its key,
    /// made for the point first shown, still show or hide it once another
    /// point is shown (found while taking the README's screenshots).
    @Test func theInspectorFollowsTheWindowFromPointToPoint() async {
        let history = viewModel(sample())
        await history.load()
        let first = history.file
        #expect(first?.inspectorShown == true)
        await history.older()
        #expect(history.file !== first)
        first?.toggleInspector()
        #expect(history.file?.showsInspector == false)
        first?.toggleInspector()
        #expect(history.file?.inspectorShown == true)
    }

    /// "Against a pin" pins the newest commit, reads each point against
    /// it and counts what differs; the pin itself has nothing to show.
    @Test func againstAPinComparesEveryPointWithIt() async {
        let source = sample()
        let history = viewModel(source)
        await history.load()
        #expect(history.canPin)
        await history.select(history.points[3])
        await history.setMode(.againstPin)
        #expect(history.mode == .againstPin)
        #expect(history.pin == .point(index: 1))
        #expect(history.pinName == "HEAD")
        #expect(history.isPinned(history.points[1]))
        #expect(source.asked.last?.pin == .point(index: 1))
        #expect(history.modeSentence == "Each point shows everything between it and the pin, HEAD.")
        // The counts on the rows are against the pin; none on the pin.
        #expect(history.counts(history.points[3]).added == 3)
        #expect(history.counts(history.points[1]).added == nil)
        await history.select(history.points[1])
        #expect(history.file?.files.isEmpty == true)
        #expect(history.file?.emptyMessage.hasPrefix("This point is the pin") == true)
        // Back to each change: the points' own counts, no pin asked.
        await history.setMode(.eachChange)
        #expect(source.asked.last?.pin == nil)
        #expect(history.counts(history.points[3]).added == 2)
        #expect(!history.isPinned(history.points[1]))
        #expect(history.pin == .point(index: 1))
    }

    /// The key pins the selected point, the working copy included but
    /// not the index; on the pin itself, it goes back to each change.
    @Test func theKeyPinsTheSelectedPoint() async {
        let source = FakeHistory(points: [
            FakeHistory.uncommitted(0), FakeHistory.uncommitted(1, staged: true),
            FakeHistory.commit(2, "Retry", head: true), FakeHistory.commit(3, "Start"),
        ])
        let history = viewModel(source)
        await history.load()
        await history.togglePinOnSelected()
        #expect(history.mode == .againstPin)
        #expect(history.pin == .workingCopy)
        #expect(history.pinName == "the working copy")
        #expect(history.isPinned(history.points[0]))
        await history.select(history.points[1])
        #expect(!history.canPinSelected)
        await history.togglePinOnSelected()
        #expect(history.pin == .workingCopy)
        await history.select(history.points[3])
        await history.togglePinOnSelected()
        #expect(history.pin == .point(index: 3))
        #expect(history.pinName == "id3")
        await history.togglePinOnSelected()
        #expect(history.mode == .eachChange)
    }

    /// Read again after a commit, the selected commit stays selected and
    /// the pin follows its commit to its new place.
    @Test func readingAgainKeepsThePointAndThePin() async {
        let source = sample()
        let history = viewModel(source)
        await history.load()
        await history.select(history.points[2])
        await history.togglePinOnSelected()
        await history.select(history.points[3])
        // The change not committed was committed: every place moves.
        source.points = [
            FakeHistory.commit(0, "New", head: true),
            FakeHistory.commit(1, "Retry", author: "Alex Martin"),
            FakeHistory.commit(2, "Rename", oldPath: "Sources/SyncManager.swift"),
            FakeHistory.commit(3, "Start", path: "Sources/SyncManager.swift"),
        ]
        await history.load()
        #expect(history.selected?.subject == "Start")
        #expect(history.pin == .point(index: 2))
        #expect(history.mode == .againstPin)
    }

    /// A history that cannot be read says why; a file with no point has
    /// nothing selected.
    @Test func saysWhyAHistoryCannotBeRead() async {
        let source = sample()
        source.failure = ScryError.Failed(message: "git failed.")
        let history = viewModel(source)
        await history.load()
        #expect(history.error == "git failed.")
        #expect(history.points.isEmpty)

        let empty = viewModel(FakeHistory(points: []))
        await empty.load()
        #expect(empty.selected == nil && empty.file == nil)
        #expect(empty.summary == "Not committed yet")
        #expect(!empty.canPin)
        await empty.setMode(.againstPin)
        #expect(empty.mode == .eachChange)
    }

    /// Only a commit has a whole commit to show; leaving goes back to
    /// the changed files.
    @Test func showsTheWholeCommitAndLeaves() async {
        var shown: [String] = []
        var left = 0
        let history = HistoryViewModel(
            source: sample(), defaults: .isolated, showCommit: { shown.append($0.id) }, leave: { left += 1 }
        )
        await history.load()
        #expect(!history.canShowWholeCommit)
        history.showWholeCommit()
        #expect(shown.isEmpty)
        await history.older()
        #expect(history.canShowWholeCommit)
        history.showWholeCommit()
        #expect(shown == ["idRetry"])
        history.showChangedFiles()
        #expect(left == 1)
    }
}
