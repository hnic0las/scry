import Foundation
import Testing
@testable import Scry

/// Explain on lines selected in the Changes window.
@MainActor
struct LineExplainTests {
    private static let moment = Date(timeIntervalSince1970: 2000)

    /// Two files: seven lines on the new side where old line 3 became
    /// lines 3 and 4, and a deleted file of four lines.
    ///
    /// Unified rows of the first: 1, 2, −3, +3, +4, 5, 6, 7.
    private func source() -> FakeChanges {
        let source = FakeChanges(files: [
            FakeChanges.file(0, "Sources/A.swift"),
            FakeChanges.file(1, "Sources/Gone.swift", .deleted, added: 0, removed: 4),
        ])
        source.diffs[0] = FakeChanges.text(lines: 6, changesAt: [2])
        let line = DiffLine(text: "line", columns: 4, spans: [])
        source.diffs[1] = .text(diff: TextDiffInfo(
            oldLines: Array(repeating: line, count: 4), newLines: [],
            rows: (1...4).map { DiffRow(kind: .removed, old: $0, new: nil, oldWords: [], newWords: []) },
            unified: (1...4).map { UnifiedDiffLine(kind: .removed, old: $0, new: nil, words: []) },
            changes: [DiffChange(firstRow: 0, rowCount: 4, firstLine: 0, added: 0, removed: 4)],
            longestLine: 4, added: 0, removed: 4,
            oldMissingFinalNewline: false, newMissingFinalNewline: false, large: false, language: "Swift"
        ))
        source.askTime = UInt64(Self.moment.timeIntervalSince1970 * 1000)
        return source
    }

    private func defaults() -> UserDefaults {
        UserDefaults(suiteName: "ScryTests.\(UUID().uuidString)")!
    }

    private func window(_ source: FakeChanges, defaults: UserDefaults? = nil) async -> ChangesViewModel {
        let viewModel = ChangesViewModel(source: source, defaults: defaults ?? self.defaults(), now: { Self.moment })
        await viewModel.load()
        return viewModel
    }

    /// A click selects a line, a shift-click extends from it, a drag
    /// selects a range; a row with no line of the new side selects the
    /// nearest one; esc clears.
    @Test func lineNumbersSelectWholeLines() async {
        let viewModel = await window(source())
        #expect(viewModel.canSelectLines)
        #expect(viewModel.selection == nil)

        viewModel.pressLine(row: 0)
        viewModel.releaseLine()
        #expect(viewModel.selection == 1...1)
        #expect(viewModel.status.hasSuffix(" · Line 1 selected"))
        #expect(viewModel.hintStrip.contains(.explainLines))

        viewModel.pressLine(row: 4, extend: true)
        viewModel.releaseLine()
        #expect(viewModel.selection == 1...4)
        #expect(viewModel.status.hasSuffix(" · Lines 1–4 selected"))

        // A drag from line 5 two rows down, then back above where it began.
        viewModel.pressLine(row: 5)
        viewModel.pressLine(row: 5, rowDelta: 2)
        #expect(viewModel.selection == 5...7)
        viewModel.pressLine(row: 5, rowDelta: -2)
        #expect(viewModel.selection == 3...5)
        viewModel.releaseLine()

        // The removed line's row has no new line: the one above it.
        viewModel.pressLine(row: 2)
        viewModel.releaseLine()
        #expect(viewModel.selection == 2...2)

        viewModel.clearSelection()
        #expect(viewModel.selection == nil)
        #expect(!viewModel.hintStrip.contains(.explainLines))
    }

    /// A deleted file has no new side: its old lines are selected.
    @Test func aDeletedFileSelectsItsOldLines() async {
        let viewModel = await window(source())
        await viewModel.select(viewModel.files[1])
        #expect(viewModel.selectsOldSide)
        viewModel.pressLine(row: 1)
        viewModel.pressLine(row: 1, rowDelta: 2)
        #expect(viewModel.selection == 2...4)
    }

    /// Asking sends the selected lines; the explanation waits, is
    /// answered, then is stale, and the strip beside its lines follows.
    @Test func theSelectionIsAskedAboutAndItsAnswerShown() async {
        let source = source()
        let viewModel = await window(source)
        await viewModel.refreshExplanations()
        #expect(viewModel.watching == true)
        #expect(!viewModel.showsExplainInspector)

        viewModel.pressLine(row: 3)
        viewModel.pressLine(row: 3, rowDelta: 1)
        viewModel.releaseLine()
        #expect(viewModel.showsExplainInspector)
        #expect(viewModel.currentExplanation == nil)

        await viewModel.explainSelection()
        #expect(source.lineAsks.map { [$0.index, $0.first, $0.last] } == [[0, 3, 4]])
        #expect(viewModel.openExplanation == "L1")
        #expect(viewModel.explainedLines.map(\.label) == ["Lines 3–4"])
        #expect(viewModel.explainedLines[0].state == .waiting(seconds: 0, previous: nil))
        #expect(viewModel.strip(line: 3)?.kind == .waiting)
        #expect(viewModel.strip(line: 5) == nil)
        #expect(viewModel.currentExplanation?.id == "L1")

        let answer = Explanation(text: "These lines do this.", by: "Claude Code", at: source.askTime + 5000, failed: false)
        source.explained.items[0] = LineExplanation(id: "L1", first: 3, last: 4, stale: false, askedAt: nil, answer: answer)
        await viewModel.refreshExplanations()
        #expect(viewModel.explainedLines[0].state == .answered(answer, watching: true))
        #expect(viewModel.strip(line: 4)?.kind == .answered)

        source.explained.items[0] = LineExplanation(id: "L1", first: 3, last: 4, stale: true, askedAt: nil, answer: answer)
        await viewModel.refreshExplanations()
        #expect(viewModel.explainedLines[0].state == .stale(answer))
        #expect(viewModel.strip(line: 3)?.kind == .stale)

        // Asked again from its block: the same lines.
        await viewModel.askAgain(source.explained.items[0])
        #expect(source.lineAsks.count == 2)
        #expect(source.lineAsks[1].first == 3 && source.lineAsks[1].last == 4)

        await viewModel.forgetExplanation(source.explained.items[0])
        #expect(viewModel.explainedLines.map(\.id) == ["L2"])
    }

    /// With nobody watching the ask waits for a session; after a minute
    /// with a session, no answer came; a failure to ask is said.
    @Test func anAskWaitsForASessionOrSaysNothingCame() async {
        let source = source()
        source.explained.watching = false
        let viewModel = await window(source)
        viewModel.pressLine(row: 0)
        await viewModel.explainSelection()
        #expect(viewModel.explainedLines[0].state == .noSession(previous: nil))
        #expect(viewModel.watching == false)

        source.explained.watching = true
        source.explained.items[0].askedAt = source.askTime - 61_000
        await viewModel.refreshExplanations()
        #expect(viewModel.explainedLines[0].state == .noAnswer(previous: nil))

        await viewModel.cancelExplain(source.explained.items[0])
        #expect(viewModel.explainedLines.isEmpty)

        source.failure = ScryError.Refused(message: "No.")
        await viewModel.explainSelection()
        #expect(viewModel.explainError == "No.")
    }

    /// A click on an explanation's lines selects them and scrolls there,
    /// in the layout shown; going to a change takes the scroll back.
    @Test func anExplanationLeadsToItsLines() async {
        let source = source()
        source.explained.items = [
            LineExplanation(id: "L1", first: 1, last: 1, stale: false, askedAt: nil, answer: nil),
            LineExplanation(id: "L2", first: 5, last: 6, stale: false, askedAt: nil, answer: nil),
        ]
        let viewModel = await window(source)
        await viewModel.refreshExplanations()
        #expect(viewModel.showsExplainInspector)
        let scrolls = viewModel.scrollRequest

        viewModel.show(source.explained.items[1])
        #expect(viewModel.selection == 5...6)
        #expect(viewModel.openExplanation == "L2")
        #expect(viewModel.isOpen(viewModel.explainedLines[1]) && !viewModel.isOpen(viewModel.explainedLines[0]))
        #expect(viewModel.scrollRequest == scrolls + 1)
        #expect(viewModel.scrollTarget == 5)
        viewModel.layout = .split
        #expect(viewModel.scrollTarget == 4)

        viewModel.go(to: 0)
        #expect(viewModel.scrollTarget == 2)

        // Selecting lines of an explanation opens it.
        viewModel.pressLine(row: 0)
        #expect(viewModel.openExplanation == "L1")
    }

    /// Another file starts with nothing selected; with Explain off in
    /// Settings, lines cannot be selected and nothing is shown.
    @Test func selectionEndsWithTheFileAndWithExplainOff() async {
        let source = source()
        let defaults = defaults()
        let viewModel = await window(source, defaults: defaults)
        viewModel.pressLine(row: 0)
        viewModel.releaseLine()
        await viewModel.select(viewModel.files[1])
        #expect(viewModel.selection == nil)

        defaults.set(false, forKey: InspectorViewModel.explainKey)
        #expect(!viewModel.canSelectLines)
        viewModel.pressLine(row: 0)
        #expect(viewModel.selection == nil)
        await viewModel.refreshExplanations()
        #expect(viewModel.explanations == nil)
        #expect(viewModel.watching == nil)
        #expect(!viewModel.showsExplainInspector)
    }
}
