import Foundation
import Testing
@testable import Scry

/// An operation in memory: these tests must never stage, continue or abort
/// in a real repository.
final class FakeOverview: OverviewSource {
    var operation: String? = "rebase"
    var files: [OverviewFile]
    var plan: RebasePlan?
    var merge: MergeHistory?
    var applied: [UInt32] = []
    var reword: RewordInfo?
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
            merge: operation == nil ? nil : merge,
            applied: applied,
            reword: operation == nil ? nil : reword,
            files: files,
            canContinue: operation != nil && files.allSatisfy { $0.state == .staged },
            canSkip: operation == "rebase"
        )
    }

    func deletion(path: String) throws -> DeletionInfo {
        DeletionInfo(
            deletedBy: .ours, deletedLabel: "main · rebase target", keptLabel: "Your commit a1b2c3d · Retry", keptLines: 12,
            changes: "@@ -1,2 +1,2 @@\n keep\n-old\n+new\n"
        )
    }

    func binary(path: String) throws -> BinaryInfo {
        BinaryInfo(
            ours: BinaryVersionInfo(label: "main", size: 2048, data: Data([1, 2, 3])),
            theirs: BinaryVersionInfo(label: "feature", size: nil, data: nil)
        )
    }

    var projectInfo = ProjectInfo(
        oursLabel: "main", theirsLabel: "feature",
        conflicts: [projectConflict(["Two.swift in Sources"], ["Three.swift in Sources"], reason: nil)],
        resolvable: 1, missing: [], missingRemovable: false
    )

    func project(path: String) throws -> ProjectInfo { projectInfo }

    func catalog(path: String) throws -> CatalogInfo {
        CatalogInfo(oursLabel: "main", theirsLabel: "feature", automatic: 3, conflicts: [
            CatalogConflictInfo(key: "sync.done", detail: "fr › value", base: "Synchronisé", ours: "Tout est synchronisé", theirs: "Synchronisation terminée"),
            CatalogConflictInfo(key: "sync.pending", detail: "de › value", base: nil, ours: nil, theirs: "%lld ausstehend"),
        ])
    }

    func packages(path: String) throws -> PackagesInfo {
        PackagesInfo(
            oursLabel: "main", theirsLabel: "feature",
            automatic: [
                PinChangeInfo(identity: "kingfisher", base: "7.9.0", ours: "7.9.0", theirs: "7.10.0"),
                PinChangeInfo(identity: "lottie-ios", base: nil, ours: "4.4.1", theirs: nil),
            ],
            conflicts: [PinChangeInfo(identity: "alamofire", base: "5.8.0", ours: "5.8.1", theirs: "5.9.0")],
            originHashStale: true
        )
    }

    var lockfileInfo = LockfileInfo(
        oursLabel: "main", theirsLabel: "feature", tool: "npm", manifest: "web/package.json", manifestConflicted: false,
        command: "cd web && npm install", mergesByLines: false,
        changes: [PackageChangeInfo(name: "lodash", base: "4.17.20", ours: "4.17.21", theirs: nil, bothChanged: false)]
    )

    func lockfile(path: String) throws -> LockfileInfo { lockfileInfo }

    func apply(action: OverviewAction) throws {
        actions.append(action)
        switch action {
        case .stageResolved:
            files = files.map { file($0.path, $0.state == .resolved ? .staged : $0.state, absolutePath: $0.absolutePath) }
        case .stage(let path):
            files = files.map { $0.path == path ? file(path, .staged) : $0 }
        case .unstage(let path):
            files = files.map { $0.path == path ? file(path, .resolved) : $0 }
        case .continue, .abort:
            reword = nil
            operation = nil
            files = []
        case .skip:
            files = []
        case .keepFile(let path), .deleteFile(let path), .takeVersion(let path, _):
            files = files.map { $0.path == path ? file(path, .staged) : $0 }
        case .keepBothAdditions(let path, _), .resolveCatalog(let path, _), .resolvePackages(let path, _),
             .takeLockfile(let path, _), .mergeLockfileLines(let path):
            files = files.map { $0.path == path ? file(path, .resolved) : $0 }
        }
    }
}

func projectConflict(_ ours: [String], _ theirs: [String], reason: String?) -> ProjectConflictInfo {
    ProjectConflictInfo(
        oursAdds: ours, theirsAdds: theirs, oursRemoves: [], theirsRemoves: [],
        oursChanges: [], theirsChanges: [], keepsBoth: reason == nil, reason: reason
    )
}

func file(
    _ path: String, _ state: ConflictState, absolutePath: String? = nil, openable: Bool = true, special: SpecialKind? = nil
) -> OverviewFile {
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
        openable: openable,
        special: special
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

        #expect(source.actions == [.stageResolved, .continue(message: nil)])
        #expect(!overview.hasOperation)
        #expect(overview.summary.hasPrefix("No merge"))
    }

    /// One file is staged once resolved, and unstaged to change it again;
    /// files in other states offer neither, so a click never asks git for
    /// what it would refuse.
    @Test func stagesAndUnstagesOneFile() {
        let source = FakeOverview(files: [
            file("A.swift", .resolved), file("B.swift", .staged), file("C.swift", .unresolved),
        ])
        let overview = OverviewViewModel(source: source) { _ in }
        #expect(overview.rows.map(\.canStage) == [true, false, false])
        #expect(overview.rows.map(\.canUnstage) == [false, true, false])

        overview.stage("C.swift")
        overview.unstage("A.swift")
        #expect(source.actions.isEmpty)

        overview.stage("A.swift")
        overview.unstage("B.swift")
        #expect(source.actions == [.stage(path: "A.swift"), .unstage(path: "B.swift")])
        #expect(overview.rows.map(\.state) == [.staged, .resolved, .unresolved])
    }

    /// Only a file the editor can read opens, and staged files have
    /// nothing left to do from the list.
    @Test func opensOnlyWhatTheEditorCanRead() {
        var opened: [String] = []
        let overview = OverviewViewModel(source: FakeOverview(files: [
            file("A.swift", .unresolved), file("Gone.swift", .unresolved, openable: false), file("C.swift", .staged),
        ])) { opened.append($0.path) }

        #expect(overview.rows.map(\.canOpen) == [true, false, false])
        overview.open("Gone.swift")
        overview.open("A.swift")
        #expect(opened == ["A.swift"])
    }

    /// At a reword step the message starts from the commit's own, can be
    /// edited and put back, and Continue hands git the edited message: the
    /// point of a reword.
    @Test func continuesARewordWithTheMessageWritten() {
        let source = FakeOverview(files: [])
        source.reword = RewordInfo(message: "Retry uploads", committed: true)
        let overview = OverviewViewModel(source: source) { _ in }
        #expect(overview.isRewording)
        #expect(overview.rewordOnly)
        #expect(overview.rewordMessage == "Retry uploads")
        #expect(!overview.rewordEdited)
        #expect(overview.canContinue)

        overview.rewordMessage = "Retry uploads twice"
        #expect(overview.rewordEdited)
        overview.resetRewordMessage()
        #expect(overview.rewordMessage == "Retry uploads")

        overview.rewordMessage = "Retry uploads twice\n\nWith a body.\n\n"
        overview.continueOperation()
        #expect(source.actions == [.continue(message: "Retry uploads twice\n\nWith a body.")])
        #expect(!overview.isRewording)
    }

    /// Git refuses an empty message, so Continue waits for one instead of
    /// failing in git.
    @Test func anEmptyRewordMessageCannotContinue() {
        let source = FakeOverview(files: [])
        source.reword = RewordInfo(message: "Retry uploads", committed: true)
        let overview = OverviewViewModel(source: source) { _ in }
        overview.rewordMessage = "  \n"
        #expect(!overview.canContinue)
        overview.continueOperation()
        #expect(source.actions.isEmpty)
    }

    /// Reading the operation again, after resolving a file in the editor
    /// for instance, keeps the message being typed.
    @Test func aRefreshKeepsTheMessageBeingTyped() {
        let source = FakeOverview(files: [file("A.swift", .unresolved)])
        source.reword = RewordInfo(message: "Retry uploads", committed: false)
        let overview = OverviewViewModel(source: source) { _ in }
        overview.rewordMessage = "Retry uploads, reworded"
        overview.refresh()
        #expect(overview.rewordMessage == "Retry uploads, reworded")
        // Files to resolve first: the list stays, the message goes below.
        #expect(!overview.rewordOnly)
        #expect(overview.rewordHelp.hasPrefix("Once its files are staged"))
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

    /// A merge lists the merge commit to come, each branch's commits in its
    /// own lane, those past the limit as a count, then the merge base.
    @Test func mergeShowsBothBranchesDownToTheirBase() {
        let source = FakeOverview(files: [file("A.swift", .unresolved)])
        source.operation = "merge"
        source.merge = MergeHistory(
            ours: HistoryLane(name: "feature", commits: [HistoryCommit(shortId: "aaa0001", subject: "change b")], more: 2),
            theirs: HistoryLane(name: "main", commits: [HistoryCommit(shortId: "bbb0001", subject: "main change")], more: 0),
            base: HistoryCommit(shortId: "ccc0001", subject: "base")
        )
        let overview = OverviewViewModel(source: source) { _ in }

        #expect(overview.historyNames?.ours == "feature")
        #expect(overview.historyRows.map(\.kind) == [.mergeCommit, .commit(.ours), .more(.ours), .commit(.theirs), .base])
        #expect(overview.historyRows.first?.title == "Merge main into feature")
        #expect(overview.historyRows[2].title == "2 more commits")
        #expect(overview.historyRows.last?.detail == "ccc0001 · merge base")
        #expect(overview.historyCaption == "3 commits on feature · 1 on main since they split")
        #expect(overview.planRows.isEmpty)
    }
}
