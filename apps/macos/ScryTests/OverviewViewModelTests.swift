import Foundation
import Testing
@testable import Scry

/// An operation in memory: these tests must never stage, continue or abort
/// in a real repository.
final class FakeOverview: OverviewSource, @unchecked Sendable {
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
            headline: operation.map { _ in "Rebasing feature onto main" },
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
            changes: [
                CommitDiffLine(kind: .hunk, oldNumber: nil, newNumber: nil, text: "-1,2 +1,2"),
                CommitDiffLine(kind: .context, oldNumber: 1, newNumber: 1, text: "keep"),
                CommitDiffLine(kind: .removed, oldNumber: 2, newNumber: nil, text: "old"),
                CommitDiffLine(kind: .added, oldNumber: nil, newNumber: 2, text: "new"),
            ]
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
                PinChangeInfo(identity: "kingfisher", base: "7.9.0", ours: "7.9.0", theirs: "7.10.0", movedByOurs: false),
                PinChangeInfo(identity: "lottie-ios", base: nil, ours: "4.4.1", theirs: nil, movedByOurs: true),
            ],
            conflicts: [PinChangeInfo(identity: "alamofire", base: "5.8.0", ours: "5.8.1", theirs: "5.9.0", movedByOurs: true)],
            originHashStale: true
        )
    }

    var lockfileInfo = LockfileInfo(
        oursLabel: "main", theirsLabel: "feature", tool: "npm", manifest: "web/package.json", manifestConflicted: false,
        command: "cd web && npm install", mergesByLines: false,
        changes: [PackageChangeInfo(name: "lodash", base: "4.17.20", ours: "4.17.21", theirs: nil, bothChanged: false)]
    )

    func lockfile(path: String) throws -> LockfileInfo { lockfileInfo }

    var unusualInfo = UnusualInfo(what: "A submodule both sides moved to different commits.", advice: "Check out the commit you want.", inFolder: true)

    func unusual(path: String) throws -> UnusualInfo { unusualInfo }

    /// The paths whose diff was asked for, in order.
    private(set) var diffs: [String] = []

    func commitDiff(commit: String, path: String) throws -> [CommitDiffLine] {
        diffs.append(path)
        return [
            CommitDiffLine(kind: .hunk, oldNumber: nil, newNumber: nil, text: "final class SyncEngine"),
            CommitDiffLine(kind: .removed, oldNumber: 13, newNumber: nil, text: "var pending"),
            CommitDiffLine(kind: .added, oldNumber: nil, newNumber: 13, text: "var outbox \(path)"),
        ]
    }

    func apply(action: OverviewAction) throws -> OverviewOutcome {
        actions.append(action)
        switch action {
        case .stageResolved:
            files = files.map { file($0.path, $0.state == .resolved ? .staged : $0.state, absolutePath: $0.absolutePath) }
        case .stage(let paths):
            files = files.map { paths.contains($0.path) ? file($0.path, .staged) : $0 }
        case .unstage(let paths):
            files = files.map { paths.contains($0.path) ? file($0.path, .resolved) : $0 }
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
        case .markResolved(let path):
            files = files.map { $0.path == path ? file(path, .staged) : $0 }
        }
        return OverviewOutcome(events: [], snapshot: try snapshot())
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
        special: special,
        // The core's rules (`FileView::can_open` and its siblings).
        canOpen: (openable || special != nil) && state != .staged,
        canStage: state == .resolved || state == .needsReview,
        canUnstage: state == .staged
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
    @Test func stagesThenContinues() async {
        let source = FakeOverview(files: [file("A.swift", .resolved), file("B.swift", .staged)])
        let overview = OverviewViewModel(source: source) { _ in }
        #expect(overview.stageTitle == "Stage resolved (1)")
        #expect(!overview.canContinue)

        overview.continueOperation()
        await overview.idle()
        #expect(source.actions.isEmpty)

        overview.stageResolved()
        await overview.idle()
        #expect(overview.canContinue)
        overview.continueOperation()
        await overview.idle()

        #expect(source.actions == [.stageResolved, .continue(message: nil)])
        #expect(!overview.hasOperation)
        #expect(overview.summary.hasPrefix("No merge"))
    }

    /// One file is staged once resolved, and unstaged to change it again;
    /// files in other states offer neither, so a click never asks git for
    /// what it would refuse.
    @Test func stagesAndUnstagesOneFile() async {
        let source = FakeOverview(files: [
            file("A.swift", .resolved), file("B.swift", .staged), file("C.swift", .unresolved),
        ])
        let overview = OverviewViewModel(source: source) { _ in }
        #expect(overview.rows.map(\.canStage) == [true, false, false])
        #expect(overview.rows.map(\.canUnstage) == [false, true, false])

        overview.stage("C.swift")
        await overview.idle()
        overview.unstage("A.swift")
        await overview.idle()
        #expect(source.actions.isEmpty)

        overview.stage("A.swift")
        await overview.idle()
        overview.unstage("B.swift")
        await overview.idle()
        #expect(source.actions == [.stage(paths: ["A.swift"]), .unstage(paths: ["B.swift"])])
        #expect(overview.rows.map(\.state) == [.staged, .resolved, .unresolved])
    }

    /// The row menu keeps the same items in every state: those that do not
    /// apply stay disabled and say why, so the user learns what to do
    /// instead of wondering where an item went.
    @Test func rowMenuSaysWhyAnItemIsDisabled() throws {
        let source = FakeOverview(files: [
            file("A.swift", .unresolved), file("B.swift", .resolved), file("C.swift", .staged),
            file("Gone.swift", .staged, special: .deletedOnOneSide), file("R.swift", .needsReview),
            file("Icon.png", .unresolved, openable: false, special: .binary),
        ])
        let overview = OverviewViewModel(source: source) { _ in }
        let menu = { (path: String) in try #require(overview.menu(for: [path])) }

        let unresolved = try menu("A.swift")
        #expect(unresolved.open == .init(title: "Open in Merge Editor", detail: nil, enabled: true))
        #expect(unresolved.stage == .init(title: "Stage", detail: "Resolve the file first", enabled: false))
        #expect(unresolved.absolutePaths == ["/r/A.swift"])

        #expect(try menu("B.swift").stage == .init(title: "Stage", detail: nil, enabled: true))
        #expect(try menu("Icon.png").open.title == "Open…")
        #expect(try menu("R.swift").open.title == "Review Auto-resolution")
        #expect(try menu("R.swift").stage == .init(title: "Accept and Stage", detail: "Keeps the rule’s result as it is", enabled: true))

        // Staged files do not open: unstaging is the explicit step.
        let staged = try menu("C.swift")
        #expect(staged.open == .init(title: "Open in Merge Editor", detail: "Unstage to change it", enabled: false))
        #expect(staged.stage.title == "Unstage")
        #expect(staged.stage.detail?.hasSuffix("now shown as Resolved.") == true)
        let special = try menu("Gone.swift")
        #expect(special.open.title == "Show Choice")
        #expect(!special.open.enabled)
        #expect(special.stage.title == "Unstage and Choose Again")
    }

    /// With several rows selected, the second item counts them and works
    /// only if it applies to every row; the File menu acts on the same
    /// selection.
    @Test func rowMenuActsOnEverySelectedRow() async throws {
        let source = FakeOverview(files: [
            file("A.swift", .resolved), file("B.swift", .resolved), file("C.swift", .staged), file("D.swift", .unresolved),
        ])
        let overview = OverviewViewModel(source: source) { _ in }
        #expect(overview.selectionMenu == nil)

        overview.selection = ["A.swift", "B.swift"]
        let both = try #require(overview.selectionMenu)
        #expect(both.stage == .init(title: "Stage 2 Files", detail: nil, enabled: true))
        #expect(!both.open.enabled)

        let mixed = try #require(overview.menu(for: ["A.swift", "C.swift"]))
        #expect(mixed.stage.title == "Stage 2 Files")
        #expect(!mixed.stage.enabled)
        overview.stageOrUnstage(mixed.paths)
        await overview.idle()
        #expect(source.actions.isEmpty)
        #expect(try #require(overview.menu(for: ["A.swift", "D.swift"])).stage.detail == "Resolve every file first")

        overview.stageOrUnstage(both.paths)
        await overview.idle()
        #expect(source.actions == [.stage(paths: ["A.swift", "B.swift"])])
        #expect(try #require(overview.menu(for: ["A.swift", "B.swift", "C.swift"])).stage.title == "Unstage 3 Files")
    }

    /// While git runs an action, the steps wait: a second click never
    /// reaches git, so it cannot act on the stop git moves to next.
    @Test func aSecondActionWaitsForTheFirst() async {
        let source = FakeOverview(files: [file("A.swift", .resolved)])
        let overview = OverviewViewModel(source: source) { _ in }

        overview.stageResolved()
        #expect(overview.isRunning)
        #expect(!overview.canStage)
        #expect(overview.footerNote == "Git is running…")
        overview.stageResolved()
        overview.refresh()
        await overview.idle()

        #expect(!overview.isRunning)
        #expect(source.actions == [.stageResolved])
        #expect(overview.canContinue)
    }

    /// Cancelling a file's sheet reads the list again: the user may have
    /// taken a lockfile's side, or changed the file in a terminal.
    @Test func cancellingASheetReadsTheListAgain() async {
        let source = FakeOverview(files: [file("Podfile.lock", .unresolved, openable: false, special: .lockfile)])
        let overview = OverviewViewModel(source: source) { _ in }
        overview.open("Podfile.lock")
        #expect(overview.special != nil)
        source.files = [file("Podfile.lock", .resolved)]

        overview.closeSpecial()
        await overview.idle()
        #expect(overview.special == nil)
        #expect(overview.rows.first?.state == .resolved)
    }

    /// The overview's hint strip offers ↩ only while a file can open, and
    /// the app's own shortcuts always.
    @Test func hintStripOffersOpeningOnlyWhenAFileCan() {
        let open = OverviewViewModel(source: FakeOverview(files: [file("A.swift", .unresolved)])) { _ in }
        #expect(open.hintStrip.map(\.id) == ["openFile", "openRepository", "openConflictedFile", "keyboardShortcuts"])
        let staged = OverviewViewModel(source: FakeOverview(files: [file("A.swift", .staged)])) { _ in }
        #expect(staged.hintStrip.map(\.id) == ["openRepository", "openConflictedFile", "keyboardShortcuts"])
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
    @Test func continuesARewordWithTheMessageWritten() async {
        let source = FakeOverview(files: [])
        source.reword = RewordInfo(message: "Retry uploads", committed: true, commit: nil)
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
        await overview.idle()
        #expect(source.actions == [.continue(message: "Retry uploads twice\n\nWith a body.")])
        #expect(!overview.isRewording)
    }

    /// Git refuses an empty message, so Continue waits for one instead of
    /// failing in git.
    @Test func anEmptyRewordMessageCannotContinue() async {
        let source = FakeOverview(files: [])
        source.reword = RewordInfo(message: "Retry uploads", committed: true, commit: nil)
        let overview = OverviewViewModel(source: source) { _ in }
        overview.rewordMessage = "  \n"
        #expect(!overview.canContinue)
        // Said under the editor and, once, beside the disabled Continue.
        #expect(overview.rewordNote == .init(text: "Continue needs a message.", warning: true))
        #expect(overview.continueReason == "Write a message to continue")
        overview.continueOperation()
        await overview.idle()
        #expect(source.actions.isEmpty)
    }

    /// Reading the operation again, after resolving a file in the editor
    /// for instance, keeps the message being typed.
    @Test func aRefreshKeepsTheMessageBeingTyped() async {
        let source = FakeOverview(files: [file("A.swift", .unresolved)])
        source.reword = RewordInfo(message: "Retry uploads", committed: false, commit: nil)
        let overview = OverviewViewModel(source: source) { _ in }
        overview.rewordMessage = "Retry uploads, reworded"
        overview.refresh()
        await overview.idle()
        #expect(overview.rewordMessage == "Retry uploads, reworded")
        // Files to resolve first: the list stays, the message goes below.
        #expect(!overview.rewordOnly)
        #expect(overview.rewordStatus == .init(text: "Edited", warning: false))
        #expect(overview.showsStage)
    }

    /// The subject's length is advice: past 72 the note and the docked
    /// status say so, text on line 2 is pointed out, and Continue stays
    /// enabled, since git takes such a message.
    @Test func longSubjectsAndLineTwoAreAdviceOnly() {
        let source = FakeOverview(files: [])
        source.reword = RewordInfo(message: "Retry uploads", committed: true, commit: nil)
        let overview = OverviewViewModel(source: source) { _ in }
        #expect(overview.rewordNote == .init(text: "Original message, unchanged.", warning: false))

        overview.rewordMessage = String(repeating: "a", count: 89) + "\n\nBody"
        #expect(overview.subjectLength == 89)
        #expect(overview.subjectTooLong)
        #expect(overview.rewordNote.text.hasPrefix("The subject is 89 characters."))
        #expect(overview.rewordStatus == .init(text: "Subject: 89 of 72", warning: true))
        #expect(overview.canContinue)

        overview.rewordMessage = "Retry uploads\nwith a body on line 2"
        #expect(overview.rewordNote.text.hasPrefix("Leave line 2 empty"))
        #expect(overview.canContinue)

        overview.rewordMessage = "Retry uploads, again"
        #expect(overview.rewordNote == .init(text: "Edited.", warning: false))
    }

    /// While writing the message the user reads what the commit changes:
    /// its files, the first selected, and the selected file's diff, with
    /// who wrote it and when. Nothing to stage at such a step.
    @Test func aCommittedRewordShowsWhatTheCommitChanges() async {
        let source = FakeOverview(files: [])
        source.plan = Self.plan(current: 2)
        source.reword = RewordInfo(
            message: "add notes", committed: true,
            commit: RewordCommitInfo(
                id: "6b7c8d9", author: "Ada", byYou: true, authoredAt: 1_000_000,
                files: [
                    ChangedFileInfo(path: "Sources/SyncQueue.swift", name: "SyncQueue.swift", directory: "Sources/", added: 54, removed: 0),
                    ChangedFileInfo(path: "icon.png", name: "icon.png", directory: "", added: nil, removed: nil),
                ]
            )
        )
        let now = Date(timeIntervalSince1970: 1_000_000 + 3 * 86_400)
        let overview = OverviewViewModel(source: source, now: { now }) { _ in }

        #expect(overview.rewordTitle == "Commit message · abc0002")
        // Stopped for the message, not for conflicts: the pencil, not the triangle.
        #expect(overview.planRows.filter(\.conflicted).isEmpty)
        #expect(overview.rewordFiles.map(\.added) == ["+54", "binary"])
        #expect(overview.rewordFiles.map(\.removed) == [nil, nil])
        #expect(overview.rewordFiles[1].directory == "repository root")
        #expect(overview.rewordSelection == "Sources/SyncQueue.swift")
        #expect(overview.rewordDiff.map(\.text) == ["@@ final class SyncEngine", "var pending", "var outbox Sources/SyncQueue.swift"])
        #expect(overview.rewordDiff.map(\.number) == ["", "13", "13"])
        #expect(overview.rewordCommitDetail.hasPrefix("You · "))
        #expect(overview.rewordCommitDetail.hasSuffix(" · replayed without conflicts"))
        #expect(!overview.showsStage)

        overview.selectRewordFile("icon.png")
        #expect(overview.rewordDiff.last?.text == "var outbox icon.png")
        // A refresh keeps the file being read.
        overview.refresh()
        await overview.idle()
        #expect(overview.rewordSelection == "icon.png")
        #expect(source.diffs == ["Sources/SyncQueue.swift", "icon.png"])
    }

    /// The plan keeps the id the user wrote in the todo list, with the
    /// replayed commit's id in a tooltip, and a reworded step tells its old
    /// subject; the header names the step git stopped on.
    @Test func planKeepsTheTodoIdsAndTellsAReword() {
        let source = FakeOverview(files: [file("A.swift", .unresolved)])
        var plan = Self.plan(current: 3)
        plan.steps[1].replayedId = "6b7c8d9"
        plan.steps[1].was = "add notes"
        plan.steps[1].subject = "Add the notes"
        source.plan = plan
        let overview = OverviewViewModel(source: source) { _ in }

        let reworded = overview.planRows.first { $0.number == 2 }
        #expect(reworded?.shortId == "abc0002")
        #expect(reworded?.idHelp == "Replayed as 6b7c8d9")
        #expect(reworded?.was == "was “add notes”")
        #expect(overview.planRows.first { $0.number == 1 }?.was == nil)
        // The step git stopped on for conflicts shows the conflict glyph.
        #expect(overview.planRows.filter(\.conflicted).map(\.number) == [3])
        #expect(overview.stepLine == "Step 3 of 5 · fixup abc0003 “fixup! add notes”")
        #expect(overview.summary == "Rebasing feature onto main")
    }

    /// Abort only runs once confirmed: the view asks, then calls abort.
    @Test func abortsOnlyWhenAsked() async {
        let source = FakeOverview(files: [file("A.swift", .unresolved)])
        let overview = OverviewViewModel(source: source) { _ in }

        overview.confirmingAbort = true
        #expect(source.actions.isEmpty)
        overview.abort()
        await overview.idle()
        #expect(source.actions == [.abort])
    }

    /// Skip throws away the commit and the choices made in its files, so
    /// it asks first, as Abort does: the view asks, then calls skip.
    @Test func skipsOnlyWhenAsked() async {
        let source = FakeOverview(files: [file("A.swift", .unresolved)])
        let overview = OverviewViewModel(source: source) { _ in }

        overview.confirmingSkip = true
        #expect(source.actions.isEmpty)
        #expect(overview.skipMessage.hasPrefix("Leaves the commit being applied out of the rebase"))
        overview.skip()
        await overview.idle()
        #expect(source.actions == [.skip])
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
    @Test func explainsTheStepsGitRanOnItsOwn() async {
        let source = FakeOverview(files: [file("A.swift", .unresolved)])
        source.plan = Self.plan(current: 5)
        source.applied = [2, 3, 4]
        let overview = OverviewViewModel(source: source) { _ in }

        #expect(overview.appliedNote == "Git ran steps 2 to 4 (reword, fixup, drop) without conflicts, then stopped at step 5.")
        #expect(overview.planCaption == "Step 5 of 5 · 4 done")

        source.applied = [4]
        overview.refresh()
        await overview.idle()
        #expect(overview.appliedNote == "Git ran step 4 (drop) without conflicts, then stopped at step 5.")
    }

    private static func plan(current: UInt32) -> RebasePlan {
        let lines: [(StepAction, String, UInt32?)] = [
            (.pick, "change a", nil), (.reword, "add notes", nil), (.fixup, "fixup! add notes", 2),
            (.drop, "debug", nil), (.pick, "change b", nil),
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
                    actionName: "\(line.0)",
                    shortId: "abc000\(number)",
                    subject: line.1,
                    replayedId: nil,
                    was: nil,
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
