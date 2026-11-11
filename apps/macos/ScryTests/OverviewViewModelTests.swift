import Testing
@testable import Scry

/// An operation in memory: these tests must never stage, continue or abort
/// in a real repository.
final class FakeOverview: OverviewSource {
    var operation: String? = "rebase"
    var files: [OverviewFile]
    var plan: RebasePlan?
    var applied: [UInt32] = []
    private(set) var actions: [OverviewAction] = []
    private(set) var reads = 0

    init(files: [OverviewFile]) {
        self.files = files
    }

    func snapshot() throws -> OverviewSnapshot {
        reads += 1
        return OverviewSnapshot(
            repository: "offline-sync",
            root: "/r",
            operation: operation,
            summary: operation.map { _ in "Rebasing feature onto main · commit 2 of 3: a1b2c3d Retry" },
            step: operation == nil ? nil : 2,
            steps: operation == nil ? nil : 3,
            plan: operation == nil ? nil : plan,
            applied: applied,
            files: files,
            canContinue: operation != nil && files.allSatisfy { $0.state == .staged },
            canSkip: operation == "rebase"
        )
    }

    func apply(action: OverviewAction) throws {
        actions.append(action)
        switch action {
        case .stageResolved:
            files = files.map { file($0.path, $0.state == .resolved ? .staged : $0.state, absolutePath: $0.absolutePath) }
        case .continue, .abort:
            operation = nil
            files = []
        case .skip:
            files = []
        }
    }
}

func file(_ path: String, _ state: ConflictState, absolutePath: String? = nil, openable: Bool = true) -> OverviewFile {
    let name = path.split(separator: "/").last.map(String.init) ?? path
    let directory = path.contains("/") ? String(path[..<path.lastIndex(of: "/")!]) : ""
    return OverviewFile(
        path: path,
        absolutePath: absolutePath ?? "/r/" + path,
        name: name,
        directory: directory,
        kind: "Swift",
        state: state,
        conflicts: state == .unresolved ? 2 : 0,
        openable: openable
    )
}

@MainActor
struct OverviewViewModelTests {
    /// The header says where the rebase is and how many files are ready,
    /// as the design's S1 shows.
    @Test func headerShowsTheOperationAndProgress() {
        let source = FakeOverview(files: [
            file("Sources/A.swift", .unresolved), file("B.swift", .resolved), file("C.swift", .staged),
        ])
        let overview = OverviewViewModel(source: source) { _ in }

        #expect(overview.title == "offline-sync")
        #expect(overview.summary.hasPrefix("Rebasing feature onto main"))
        #expect(overview.planRows.isEmpty)
        #expect(overview.planBase == nil)
        #expect(overview.progress == "2 of 3 files ready")
        #expect(overview.progressDetail == "1 unresolved")
        #expect(overview.abortTitle == "Abort rebase…")
        #expect(overview.continueTitle == "Continue rebase")
    }

    /// Filters count each state and show only its files; a file at the
    /// root says so instead of an empty folder.
    @Test func filtersCountAndNarrowTheFiles() {
        let overview = OverviewViewModel(source: FakeOverview(files: [
            file("Sources/A.swift", .unresolved), file("B.swift", .unresolved), file("C.swift", .staged),
        ])) { _ in }

        #expect(overview.filters.map(\.count) == [3, 2, 0, 0, 1])
        overview.select(.unresolved)
        #expect(overview.rows.map(\.name) == ["A.swift", "B.swift"])
        #expect(overview.rows.map(\.directory) == ["Sources", "repository root"])
        #expect(overview.filters.first { $0.selected }?.filter == .unresolved)
    }

    /// Continue waits until git has every file: staging the resolved ones
    /// goes through the core, then continue is offered and finishes.
    @Test func stagesThenContinues() {
        let source = FakeOverview(files: [file("A.swift", .resolved), file("B.swift", .staged)])
        let overview = OverviewViewModel(source: source) { _ in }
        #expect(overview.stageTitle == "Stage resolved (1)")
        #expect(!overview.canContinue)

        overview.continueOperation()
        #expect(source.actions.isEmpty)

        overview.stageResolved()
        #expect(overview.canContinue)
        overview.continueOperation()

        #expect(source.actions == [.stageResolved, .continue])
        #expect(!overview.hasOperation)
        #expect(overview.summary.hasPrefix("No merge"))
    }

    /// Only a file the editor can read opens, and staged files have
    /// nothing left to do from the list.
    @Test func opensOnlyWhatTheEditorCanRead() {
        var opened: [String] = []
        let overview = OverviewViewModel(source: FakeOverview(files: [
            file("A.swift", .unresolved), file("Gone.swift", .unresolved, openable: false), file("C.swift", .staged),
        ])) { opened.append($0.path) }

        #expect(overview.rows.map(\.action) == ["Open", nil, nil])
        overview.open("Gone.swift")
        overview.open("A.swift")
        #expect(opened == ["A.swift"])
    }

    /// Abort only runs once confirmed: the view asks, then calls abort.
    @Test func abortsOnlyWhenAsked() {
        let source = FakeOverview(files: [file("A.swift", .unresolved)])
        let overview = OverviewViewModel(source: source) { _ in }

        overview.confirmingAbort = true
        #expect(source.actions.isEmpty)
        overview.abort()
        #expect(source.actions == [.abort])
    }

    /// The plan reads as a log, newest step first, down to the commit
    /// replayed onto; a fixup says which commit it joins, a drop is
    /// marked, and each step says what git does with it.
    @Test func planShowsEveryStepNewestFirst() {
        let source = FakeOverview(files: [file("A.swift", .unresolved)])
        source.plan = Self.plan(current: 1)
        let overview = OverviewViewModel(source: source) { _ in }

        #expect(overview.planRows.map(\.number) == [5, 4, 3, 2, 1])
        #expect(overview.planRows.map(\.action) == ["pick", "drop", "fixup", "reword", "pick"])
        let fixup = overview.planRows[2]
        #expect(fixup.joined && fixup.note == "into 2")
        #expect(fixup.detail == "To come: folded into step 2, keeping that commit's message.")
        #expect(overview.planRows[1].dropped)
        #expect(overview.planRows[4].detail == "Git stopped here: resolve its files, then continue.")
        #expect(overview.planBase == .init(name: "main", shortId: "9f3e2c1", subject: "Serve status as JSON"))
        #expect(overview.planBranch == "feature")
        #expect(overview.planCaption == "Step 1 of 5 · 0 done")
        #expect(overview.appliedNote == nil)
    }

    /// When git runs steps without conflicts after Continue, the overview
    /// says which, so a jump from step 1 to step 5 is explained.
    @Test func explainsTheStepsGitRanOnItsOwn() {
        let source = FakeOverview(files: [file("A.swift", .unresolved)])
        source.plan = Self.plan(current: 5)
        source.applied = [2, 3, 4]
        let overview = OverviewViewModel(source: source) { _ in }

        #expect(overview.appliedNote == "Git ran steps 2 to 4 (reword, fixup, drop) without conflicts, then stopped at step 5.")
        #expect(overview.planCaption == "Step 5 of 5 · 4 done")

        source.applied = [4]
        overview.refresh()
        #expect(overview.appliedNote == "Git ran step 4 (drop) without conflicts, then stopped at step 5.")
    }

    private static func plan(current: UInt32) -> RebasePlan {
        let lines: [(String, String, UInt32?)] = [
            ("pick", "change a", nil), ("reword", "add notes", nil), ("fixup", "fixup! add notes", 2),
            ("drop", "debug", nil), ("pick", "change b", nil),
        ]
        return RebasePlan(
            ontoName: "main",
            branch: "feature",
            ontoShortId: "9f3e2c1",
            ontoSubject: "Serve status as JSON",
            steps: lines.enumerated().map { index, line in
                let number = UInt32(index + 1)
                return RebaseStep(
                    number: number,
                    action: line.0,
                    shortId: "abc000\(number)",
                    subject: line.1,
                    state: number < current ? .done : number == current ? .current : .pending,
                    joins: line.2
                )
            }
        )
    }
}
