import Foundation
import Testing
@testable import Scry

/// A blame a test fills in: git "finds" the first `available` groups of
/// lines, and is done once `read` ran.
final class FakeBlame: BlameSource, @unchecked Sendable {
    var commits: [BlameCommitInfo]
    var lines: [BlameLines]
    var available: Int
    var neverCommitted = false
    private(set) var done = false
    private(set) var stopped = false

    init(commits: [BlameCommitInfo], lines: [BlameLines]) {
        self.commits = commits
        self.lines = lines
        available = lines.count
    }

    func read() { done = true }
    func stop() { stopped = true }

    func progress(commits taken: UInt32, lines takenLines: UInt32) -> BlameProgress {
        let found = Array(lines.prefix(available))
        let named = (found.compactMap(\.commit).max()).map { Int($0) + 1 } ?? 0
        return BlameProgress(
            commits: Array(commits.prefix(named).dropFirst(Int(taken))),
            lines: Array(found.dropFirst(Int(takenLines))),
            done: done && available >= lines.count, neverCommitted: neverCommitted, failure: nil
        )
    }

    static func commit(_ id: String, author: String, mine: Bool = false) -> BlameCommitInfo {
        BlameCommitInfo(
            id: "id\(id)", shortId: id, author: author, mine: mine, time: 1_000, subject: "Subject \(id)",
            path: "Sources/A.swift"
        )
    }

    /// Ten lines: 1–4 by Priya, 5–6 not committed, 7–8 by the user, 9–10
    /// by Priya again; git names the newest commit first.
    static func sample() -> FakeBlame {
        FakeBlame(
            commits: [commit("b", author: "Alex Martin", mine: true), commit("a", author: "Priya Shah")],
            lines: [
                BlameLines(first: 5, count: 2, commit: nil),
                BlameLines(first: 7, count: 2, commit: 0),
                BlameLines(first: 1, count: 4, commit: 1),
                BlameLines(first: 9, count: 2, commit: 1),
            ]
        )
    }
}

@MainActor
struct BlameTests {
    /// The lines git has not named yet are unknown; the others say their
    /// commit, and the first line of each group says so.
    @Test func linesAreKnownLittleByLittle() {
        let source = FakeBlame.sample()
        source.available = 2
        let blame = BlameReading(source: source, lineCount: 10)
        blame.take()
        #expect(blame.cell(line: 1) == .unknown)
        #expect(blame.cell(line: 5) == .notCommitted(first: true))
        #expect(blame.cell(line: 6) == .notCommitted(first: false))
        #expect(blame.cell(line: 7) == .commit(source.commits[0], first: true))
        #expect(blame.cell(line: 8) == .commit(source.commits[0], first: false))
        #expect(blame.status == "Blame: reading history… 40 %")
        #expect(!blame.done)

        source.available = 4
        source.read()
        blame.take()
        #expect(blame.done)
        #expect(blame.status == nil)
        #expect(blame.cell(line: 1) == .commit(source.commits[1], first: true))
        #expect(blame.cell(line: 4) == .commit(source.commits[1], first: false))
        // The same commit again after other lines: a group of its own.
        #expect(blame.cell(line: 9) == .commit(source.commits[1], first: true))
        #expect(blame.groupStart(line: 10) == 9)
        #expect(blame.groupStart(line: 3) == 1)
        #expect(blame.cell(line: 11) == .unknown)
    }

    @Test func runReadsUntilGitIsDone() async {
        let source = FakeBlame.sample()
        let blame = BlameReading(source: source, lineCount: 10, pause: {})
        await blame.run()
        #expect(blame.done)
        #expect(blame.known == 10)
    }

    /// A file that is in no commit: every line is not committed.
    @Test func aFileNeverCommittedIsAllNotCommitted() {
        let source = FakeBlame(commits: [], lines: [])
        source.neverCommitted = true
        source.read()
        let blame = BlameReading(source: source, lineCount: 3)
        blame.take()
        #expect(blame.cell(line: 1) == .notCommitted(first: true))
        #expect(blame.cell(line: 3) == .notCommitted(first: false))
    }

    @Test func authorsAreNamedOrShortened() {
        #expect(BlameReading.author(FakeBlame.commit("a", author: "Priya Shah")) == "Priya Shah")
        #expect(BlameReading.author(FakeBlame.commit("a", author: "Alex Martin", mine: true)) == "You")
        #expect(BlameReading.initials(FakeBlame.commit("a", author: "Priya Shah")) == "PS")
        #expect(BlameReading.initials(FakeBlame.commit("a", author: "Jean-Luc van Damme")) == "JD")
        #expect(BlameReading.initials(FakeBlame.commit("a", author: "nicolas")) == "N")
        #expect(BlameReading.initials(FakeBlame.commit("a", author: "Alex Martin", mine: true)) == "You")
        let help = BlameReading.help(FakeBlame.commit("a", author: "Priya Shah"), timeZone: TimeZone(identifier: "UTC")!)
        #expect(help.hasPrefix("Subject a\na · Priya Shah · "))
        #expect(help.hasSuffix("Click to see what this commit did to the file."))
    }

    private func changes() -> FakeChanges {
        let source = FakeChanges(files: [
            FakeChanges.file(0, "Sources/A.swift"), FakeChanges.file(1, "Sources/B.swift"),
            FakeChanges.file(2, "Sources/Gone.swift", .deleted),
        ])
        for index: UInt32 in 0...2 {
            source.diffs[index] = FakeChanges.text(lines: 10, changesAt: [2])
            source.blames[index] = FakeBlame.sample()
        }
        return source
    }

    /// Blame is the window's: turned on, each file shown gets its own,
    /// and the one of the file left is stopped.
    @Test func theOpenFileIsBlamedWhileBlameIsOn() async {
        let source = changes()
        let blame = BlameSwitch()
        let viewModel = ChangesViewModel(source: source, defaults: .isolated, blame: blame)
        await viewModel.load()
        #expect(viewModel.canBlame)
        #expect(viewModel.blame == nil)
        #expect(viewModel.toolbarHints.contains(.blame))

        viewModel.toggleBlame()
        #expect(blame.isOn)
        let first = viewModel.blame
        #expect(first != nil)

        await viewModel.select(viewModel.files[1])
        #expect(viewModel.blame != nil)
        #expect(viewModel.blame !== first)
        #expect(source.blames[0]?.stopped == true)

        // A deleted file has no new side to blame; blame can still be
        // turned off from it.
        await viewModel.select(viewModel.files[2])
        #expect(!viewModel.canBlame)
        #expect(viewModel.blame == nil)
        viewModel.toggleBlame()
        #expect(!blame.isOn)

        // Another model of the same window follows the switch.
        blame.isOn = true
        let other = ChangesViewModel(source: changes(), defaults: .isolated, blame: blame)
        await other.load()
        #expect(other.blame != nil)
    }

    @Test func aClickOnALineAsksForItsCommit() async {
        let source = changes()
        var asked: [String] = []
        let viewModel = ChangesViewModel(
            source: source, defaults: .isolated,
            openBlamed: { commit, file in asked.append("\(commit.id) \(file.path)") }
        )
        await viewModel.load()
        viewModel.showBlamed(FakeBlame.commit("a", author: "Priya Shah"))
        #expect(asked == ["ida Sources/A.swift"])
    }

    /// From a line of the blame, a history shows what the line's commit
    /// did, reading older commits to find it.
    @Test func aHistoryGoesToACommit() async {
        let source = FakeHistory(points: [
            FakeHistory.uncommitted(0), FakeHistory.commit(1, "Retry", head: true),
        ])
        source.older = [FakeHistory.commit(2, "Start")]
        let history = HistoryViewModel(source: source, defaults: .isolated, startAt: "idRetry")
        await history.load()
        #expect(history.selected?.index == 1)

        await history.setMode(.againstPin)
        await history.show(commit: "idStart")
        #expect(history.selected?.index == 2)
        #expect(history.mode == .eachChange)

        await history.show(commit: "idNowhere")
        #expect(history.selected?.index == 2)
    }

    /// The gutter has names in a wide unified view, initials otherwise,
    /// and takes its width from the new side's code.
    @Test func theGutterTakesItsWidthFromTheCode() {
        let off = DiffMetrics(layout: .unified, width: 1_000, digits: 3)
        let wide = DiffMetrics(layout: .unified, width: 1_000, digits: 3, blame: true)
        #expect(off.blame == 0)
        #expect(wide.blame == DiffMetrics.blameFull && !wide.compactBlame)
        #expect(wide.code == off.code - DiffMetrics.blameFull)
        let narrow = DiffMetrics(layout: .unified, width: 700, digits: 3, blame: true)
        #expect(narrow.blame == DiffMetrics.blameCompact && narrow.compactBlame)
        let split = DiffMetrics(layout: .split, width: 1_400, digits: 3, blame: true)
        #expect(split.blame == DiffMetrics.blameCompact)
    }
}
