import Foundation
import Observation

/// Where the overview reads the operation and sends its steps: the core
/// (`RepositoryOverview`) in the app, a fake in tests and previews, which
/// must never stage, continue or abort in a real repository.
protocol OverviewSource {
    func snapshot() throws -> OverviewSnapshot
    func apply(action: OverviewAction) throws
    func deletion(path: String) throws -> DeletionInfo
    func binary(path: String) throws -> BinaryInfo
    func project(path: String) throws -> ProjectInfo
    func catalog(path: String) throws -> CatalogInfo
    func packages(path: String) throws -> PackagesInfo
    func lockfile(path: String) throws -> LockfileInfo
}

extension RepositoryOverview: OverviewSource {}

/// S1: the operation in progress and every file it left with conflicts,
/// with the operation's steps.
@MainActor
@Observable
final class OverviewViewModel {
    /// The files a filter shows.
    enum Filter: CaseIterable {
        case all
        case unresolved
        case needsReview
        case resolved
        case staged
    }

    /// A filter in the sidebar.
    struct FilterRow: Identifiable {
        let filter: Filter
        let label: String
        /// `nil` for All files.
        let state: ConflictState?
        let count: Int
        let selected: Bool
        var id: Filter { filter }
    }

    /// A line of the rebase's todo list, as the plan's graph draws it.
    struct PlanRow: Identifiable, Equatable {
        let number: Int
        /// Git's word: "pick", "reword", "fixup", "drop"…
        let action: String
        let shortId: String?
        let subject: String
        let state: RebaseStepState
        /// A fixup or squash, drawn beside the commit it joins.
        let joined: Bool
        /// "into 3" for a fixup or squash.
        let note: String?
        let dropped: Bool
        /// What git does, or did, with the line.
        let detail: String
        var id: Int { number }
    }

    /// A row of a merge's history: the merge commit to come, a commit of
    /// either branch, or their merge base.
    struct HistoryRow: Identifiable, Equatable {
        enum Side: Equatable {
            case ours
            case theirs
        }

        enum Kind: Equatable {
            case mergeCommit
            case commit(Side)
            /// Commits not listed on that side.
            case more(Side)
            case base
        }

        let id: String
        let kind: Kind
        let title: String
        let detail: String
        let help: String
    }

    /// The commit the branch is replayed onto.
    struct PlanBase: Equatable {
        /// "main", or the commit's id.
        let name: String
        let shortId: String
        let subject: String
    }

    /// A file in the table.
    struct FileRow: Identifiable, Equatable {
        let path: String
        let name: String
        /// "repository root" at the root.
        let directory: String
        let kind: String
        let state: ConflictState
        let stateLabel: String
        /// "2", or "—" when the file cannot be read.
        let conflicts: String
        /// Whether it opens from here, not staged yet: in the merge editor,
        /// or in its own view when `special` says which.
        let canOpen: Bool
        let special: SpecialKind?
        var id: String { path }
    }

    private(set) var snapshot: OverviewSnapshot?
    private(set) var error: String?
    private(set) var filter: Filter = .all
    /// Asking before giving up the whole operation.
    var confirmingAbort = false
    /// The view settling a special file, while it is shown.
    private(set) var special: SpecialFileViewModel?

    @ObservationIgnored private let source: OverviewSource
    @ObservationIgnored private let openFile: (OverviewFile) -> Void

    /// `openFile` opens a file in the merge editor; the root screen owns it.
    init(source: OverviewSource, openFile: @escaping (OverviewFile) -> Void) {
        self.source = source
        self.openFile = openFile
        refresh()
    }

    // MARK: Header

    var title: String { snapshot?.repository ?? "" }

    /// The operation in plain words, or that there is none.
    var summary: String {
        snapshot?.summary ?? "No merge, rebase, cherry-pick or revert in progress."
    }

    var hasOperation: Bool { snapshot?.operation != nil }

    // MARK: Merge history

    /// The branch checked out and the branch merged in; `nil` outside a
    /// merge.
    var historyNames: (ours: String, theirs: String)? {
        snapshot?.merge.map { ($0.ours.name, $0.theirs.name) }
    }

    /// "4 commits on feature/sync-v2 · 5 on main since they split".
    var historyCaption: String {
        guard let merge = snapshot?.merge else { return "" }
        let ours = merge.ours.commits.count + Int(merge.ours.more)
        let theirs = merge.theirs.commits.count + Int(merge.theirs.more)
        return "\(Self.commits(ours)) on \(merge.ours.name) · \(theirs) on \(merge.theirs.name) since they split"
    }

    /// The merge as `git log --graph` lists it: the merge commit to come,
    /// the branch checked out, the branch merged in, then their base.
    var historyRows: [HistoryRow] {
        guard let merge = snapshot?.merge else { return [] }
        var rows = [HistoryRow(
            id: "merge",
            kind: .mergeCommit,
            title: "Merge \(merge.theirs.name) into \(merge.ours.name)",
            detail: "To come, once every file is staged",
            help: "The merge commit Continue creates: it joins both branches."
        )]
        for (side, lane, role) in [(HistoryRow.Side.ours, merge.ours, "your branch"), (.theirs, merge.theirs, "the branch merged in")] {
            rows += lane.commits.map { commit in
                HistoryRow(
                    id: "\(side)-\(commit.shortId)",
                    kind: .commit(side),
                    title: commit.subject,
                    detail: "\(commit.shortId) · \(lane.name)",
                    help: "On \(lane.name), \(role), since the merge base."
                )
            }
            if lane.more > 0 {
                rows.append(HistoryRow(
                    id: "\(side)-more",
                    kind: .more(side),
                    title: "\(lane.more) more \(lane.more == 1 ? "commit" : "commits")",
                    detail: lane.name,
                    help: "Older commits of \(lane.name) since the merge base."
                ))
            }
        }
        rows.append(HistoryRow(
            id: "base",
            kind: .base,
            title: merge.base.subject,
            detail: "\(merge.base.shortId) · merge base",
            help: "The last commit both branches share: each side's changes are measured from it."
        ))
        return rows
    }

    // MARK: Rebase plan

    /// The todo list, last step first, as a log reads from the tip down
    /// to the commit replayed onto; empty outside a rebase.
    var planRows: [PlanRow] {
        guard let plan = snapshot?.plan else { return [] }
        return plan.steps.reversed().map { step in
            let joined = step.action == "fixup" || step.action == "squash"
            return PlanRow(
                number: Int(step.number),
                action: step.action,
                shortId: step.shortId,
                subject: step.subject,
                state: step.state,
                joined: joined,
                note: joined ? step.joins.map { "into \($0)" } : nil,
                dropped: step.action == "drop",
                detail: Self.detail(step)
            )
        }
    }

    var planBase: PlanBase? {
        snapshot?.plan.map { PlanBase(name: $0.ontoName, shortId: $0.ontoShortId, subject: $0.ontoSubject) }
    }

    /// The branch being rebased, at the tip of the graph.
    var planBranch: String? { snapshot?.plan?.branch }

    /// "Step 3 of 7 · 2 done".
    var planCaption: String {
        guard let steps = snapshot?.plan?.steps else { return "" }
        let done = steps.filter { $0.state == .done }.count
        guard let current = steps.first(where: { $0.state == .current }) else {
            return "\(steps.count) steps"
        }
        return "Step \(current.number) of \(steps.count) · \(done) done"
    }

    /// What git ran on its own since it last stopped, so the jump from one
    /// step to a later one is explained: "Git ran steps 2 to 4 (reword,
    /// fixup, drop) without conflicts, then stopped at step 5."
    var appliedNote: String? {
        guard let plan = snapshot?.plan, let applied = snapshot?.applied,
              let first = applied.first, let last = applied.last,
              let current = plan.steps.first(where: { $0.state == .current })
        else { return nil }
        let actions = applied.compactMap { number in
            plan.steps.first { $0.number == number }?.action
        }
        let steps = first == last ? "step \(first)" : last == first + 1 ? "steps \(first) and \(last)" : "steps \(first) to \(last)"
        return "Git ran \(steps) (\(actions.joined(separator: ", "))) without conflicts, then stopped at step \(current.number)."
    }

    /// "2 of 6 files ready".
    var progress: String {
        let total = files.count
        return "\(count(.resolved) + count(.staged)) of \(total) \(total == 1 ? "file" : "files") ready"
    }

    /// "3 unresolved · 1 to review".
    var progressDetail: String {
        var parts = ["\(count(.unresolved)) unresolved"]
        if count(.needsReview) > 0 { parts.append("\(count(.needsReview)) to review") }
        return parts.joined(separator: " · ")
    }

    /// The share of files in each state, for the segmented bar.
    var progressSegments: [(state: ConflictState, count: Int)] {
        [ConflictState.staged, .resolved, .needsReview, .unresolved]
            .map { ($0, count($0)) }
            .filter { $0.count > 0 }
    }

    // MARK: Filters and files

    var filters: [FilterRow] {
        Filter.allCases.map { filter in
            FilterRow(
                filter: filter,
                label: Self.label(filter),
                state: Self.state(filter),
                count: Self.state(filter).map(count) ?? files.count,
                selected: filter == self.filter
            )
        }
    }

    var rows: [FileRow] {
        files
            .filter { file in Self.state(filter).map { file.state == $0 } ?? true }
            .map { file in
                FileRow(
                    path: file.path,
                    name: file.name,
                    directory: file.directory.isEmpty ? "repository root" : file.directory,
                    kind: file.kind,
                    state: file.state,
                    stateLabel: Self.label(file.state),
                    conflicts: file.conflicts.map(String.init) ?? "—",
                    canOpen: (file.openable || file.special != nil) && file.state != .staged,
                    special: file.state == .staged ? nil : file.special
                )
            }
    }

    // MARK: Footer

    private var verb: String { snapshot?.operation ?? "operation" }

    var abortTitle: String { "Abort \(verb)…" }
    var abortMessage: String {
        "Gives up the whole \(verb) and puts the branch back as it was before. Choices made in its files are lost."
    }

    /// "3 files still unresolved".
    var unresolvedSummary: String {
        let left = count(.unresolved) + count(.needsReview)
        return left == 0 ? "" : "\(left) \(left == 1 ? "file" : "files") still unresolved"
    }

    var stageTitle: String { "Stage resolved (\(count(.resolved)))" }
    var canStage: Bool { count(.resolved) > 0 }
    var continueTitle: String { "Continue \(verb)" }
    var canContinue: Bool { snapshot?.canContinue ?? false }
    var canSkip: Bool { snapshot?.canSkip ?? false }

    // MARK: Intents

    /// Reads the operation again: files change as they are resolved in the
    /// editor, or by hand in another tool.
    func refresh() {
        do {
            snapshot = try source.snapshot()
            error = nil
        } catch {
            self.error = "\(error)"
        }
    }

    func select(_ filter: Filter) {
        self.filter = filter
    }

    /// Opens a file: its own view when it has one, else the merge editor.
    func open(_ path: String) {
        guard let file = files.first(where: { $0.path == path }), file.state != .staged else { return }
        if file.special != nil {
            special = SpecialFileViewModel(
                file: file,
                source: source,
                done: { [weak self] in
                    self?.closeSpecial()
                    self?.refresh()
                },
                openInEditor: { [weak self] file in
                    self?.closeSpecial()
                    self?.openFile(file)
                }
            )
        } else if file.openable {
            openFile(file)
        }
    }

    func closeSpecial() {
        special = nil
    }

    func stageResolved() {
        guard canStage else { return }
        run(.stageResolved)
    }

    func continueOperation() {
        guard canContinue else { return }
        run(.continue)
    }

    func skip() {
        guard canSkip else { return }
        run(.skip)
    }

    /// Gives up the operation, once the user confirmed.
    func abort() {
        guard hasOperation else { return }
        run(.abort)
    }

    // MARK: Private

    private var files: [OverviewFile] { snapshot?.files ?? [] }

    private func count(_ state: ConflictState) -> Int {
        files.filter { $0.state == state }.count
    }

    private func run(_ action: OverviewAction) {
        do {
            try source.apply(action: action)
            error = nil
        } catch {
            self.error = "\(error)"
        }
        refresh()
    }

    private static func commits(_ count: Int) -> String {
        "\(count) \(count == 1 ? "commit" : "commits")"
    }

    private static func detail(_ step: RebaseStep) -> String {
        let target = step.joins.map { "step \($0)" } ?? "the commit before"
        switch step.state {
        case .current:
            return step.action == "edit" || step.action == "break"
                ? "Git stopped here, as the plan asks. Continue when ready."
                : "Git stopped here: resolve its files, then continue."
        case .done:
            switch step.action {
            case "drop": return "Dropped: this commit was not replayed."
            case "fixup", "squash": return "Folded into \(target)."
            case "exec": return "Ran: \(step.subject)"
            default: return "Replayed."
            }
        case .pending:
            switch step.action {
            case "pick": return "To come: git replays this commit."
            case "reword": return "To come: replayed with its message kept as it is."
            case "edit": return "To come: git replays it, then stops so you can amend it."
            case "fixup": return "To come: folded into \(target), keeping that commit's message."
            case "squash": return "To come: folded into \(target), messages combined."
            case "drop": return "Dropped: this commit will not be replayed."
            case "exec": return "To come: git runs `\(step.subject)`."
            case "break": return "To come: git pauses here."
            default: return "To come."
            }
        }
    }

    private static func label(_ filter: Filter) -> String {
        switch filter {
        case .all: "All files"
        case .unresolved: "Unresolved"
        case .needsReview: "Needs review"
        case .resolved: "Resolved"
        case .staged: "Staged"
        }
    }

    private static func label(_ state: ConflictState) -> String {
        switch state {
        case .unresolved: "Unresolved"
        case .needsReview: "Needs review"
        case .resolved: "Resolved"
        case .staged: "Staged"
        }
    }

    private static func state(_ filter: Filter) -> ConflictState? {
        switch filter {
        case .all: nil
        case .unresolved: .unresolved
        case .needsReview: .needsReview
        case .resolved: .resolved
        case .staged: .staged
        }
    }
}
