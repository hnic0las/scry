import Foundation
import Observation

/// Where the overview reads the operation and sends its steps: the core
/// (`RepositoryOverview`) in the app, a fake in tests and previews, which
/// must never stage, continue or abort in a real repository.
/// Its calls run git, so the view models make them off the main actor.
protocol OverviewSource: Sendable {
    func snapshot() throws -> OverviewSnapshot
    /// Runs the action: what it did, and the overview read again.
    func apply(action: OverviewAction) throws -> OverviewOutcome
    func deletion(path: String) throws -> DeletionInfo
    func binary(path: String) throws -> BinaryInfo
    func project(path: String) throws -> ProjectInfo
    func catalog(path: String) throws -> CatalogInfo
    func packages(path: String) throws -> PackagesInfo
    func lockfile(path: String) throws -> LockfileInfo
    func unusual(path: String) throws -> UnusualInfo
    func commitDiff(commit: String, path: String) throws -> [CommitDiffLine]
    /// Where Explain stands: files explained or waiting, and whether a
    /// Claude Code session watches.
    func explanations() throws -> ExplainOverview
}

extension OverviewSource {
    /// Nothing asked, nobody watching: previews and tests that do not
    /// look at Explain.
    func explanations() throws -> ExplainOverview { ExplainOverview(watching: false, files: []) }
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
        let kind: StepAction
        /// The todo list's id, the one the user wrote.
        let shortId: String?
        let subject: String
        /// "Replayed as 6b7c8d9", the id's tooltip once done.
        let idHelp: String?
        /// "was “wip upload queue”" under a reworded step.
        let was: String?
        let state: RebaseStepState
        /// A fixup or squash, drawn beside the commit it joins.
        let joined: Bool
        /// "into 3" for a fixup or squash.
        let note: String?
        let dropped: Bool
        /// Git stopped on this line for conflicts: its node is the
        /// unresolved glyph, as everywhere else a conflict is.
        let conflicted: Bool
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
        /// "1 of 2 reviewed", under the state of a file resolved outside
        /// Scry and not fully reviewed; `nil` for any other.
        let review: String?
        /// "1 explained · 1 waiting", while one of its conflicts waits for
        /// Claude Code's explanation (SPEC); `nil` otherwise.
        let explanation: String?
        /// What staging it leaves unreviewed: "1 conflict not reviewed",
        /// "changed outside its conflicts"; `nil` when nothing.
        let notReviewed: String?
        /// Whether it opens from here, not staged yet: in the merge editor,
        /// or in its own view when `special` says which.
        let canOpen: Bool
        let special: SpecialKind?
        /// Resolved, so git can take it as it is.
        let canStage: Bool
        /// Staged: its conflict can come back, to change it.
        let canUnstage: Bool
        /// Deleted on one side, binary, project or catalog: settled by a
        /// choice in its own view, even once staged.
        let isSpecial: Bool
        let absolutePath: String
        var id: String { path }
    }

    /// An item of the file row menu. An item that does not apply stays,
    /// disabled, with the reason as its second line, so the menu keeps
    /// its shape.
    struct MenuItem: Equatable {
        let title: String
        let detail: String?
        let enabled: Bool
    }

    /// The file row menu for some rows: the right-clicked ones, or the
    /// selection for the File menu (SPEC: File row context menu).
    struct RowMenu: Equatable {
        /// The default open action, ↩.
        let open: MenuItem
        let stage: MenuItem
        /// The rows, in the table's order.
        let paths: [String]
        let absolutePaths: [String]
    }

    /// A file of the reworded commit, in "In this commit".
    struct ChangedFileRow: Identifiable, Equatable {
        let path: String
        let name: String
        let directory: String
        /// "+54", or "binary"; `nil` when nothing was added.
        let added: String?
        /// "−2"; `nil` when nothing was removed.
        let removed: String?
        var id: String { path }
    }

    /// A line of the selected file's diff.
    struct DiffRow: Identifiable, Equatable {
        let id: Int
        let kind: DiffLineKind
        /// The line in the new file, or in the old one for a removed line.
        let number: String
        /// "@@ final class SyncEngine" for a hunk, else the line.
        let text: String
    }

    /// A remark under the message editor; `warning` draws it in the
    /// warning colour.
    struct MessageNote: Equatable {
        let text: String
        let warning: Bool
    }

    private(set) var snapshot: OverviewSnapshot?
    private(set) var error: String?
    /// Git is running an action or a read: the steps wait for it, so a
    /// second click never lands on the next stop.
    private(set) var isRunning = false
    private(set) var filter: Filter = .all
    /// Asking before giving up the whole operation.
    var confirmingAbort = false
    /// Asking before leaving the current commit out.
    var confirmingSkip = false

    /// Asking before staging files whose review is not done.
    struct StageConfirmation: Equatable {
        let title: String
        let message: String
        let paths: [String]
    }

    /// The files waiting for the user's word before they are staged.
    private(set) var stageConfirmation: StageConfirmation?
    var confirmingStage: Bool {
        get { stageConfirmation != nil }
        set { if !newValue { stageConfirmation = nil } }
    }
    /// The view settling a special file, while it is shown.
    private(set) var special: SpecialFileViewModel?
    /// The message being written at a `reword` step; Continue gives it to
    /// git.
    var rewordMessage = ""
    /// The reword step the message was started for, so a refresh keeps
    /// what the user typed and a new step starts from its own message.
    @ObservationIgnored private var rewordStep: String?
    /// The rows selected in the table, by path.
    var selection: Set<String> = []
    /// The file of the reworded commit whose diff shows.
    private(set) var rewordSelection: String?
    private(set) var rewordDiff: [DiffRow] = []
    /// Gives "3 days ago"; tests pass a fixed date.
    @ObservationIgnored private let now: () -> Date

    @ObservationIgnored private let source: OverviewSource
    /// Holds whether Explain and Scry's plugin are on (Settings › Agent).
    @ObservationIgnored private let defaults: UserDefaults
    @ObservationIgnored private let openFile: (OverviewFile) -> Void
    /// The git action or read in flight.
    @ObservationIgnored private var work: Task<Void, Never>?

    /// Starts from `snapshot`, which the app reads off the main actor;
    /// without one (previews, tests), reads it here, at once. `openFile`
    /// opens a file in the merge editor; the root screen owns it.
    init(
        source: OverviewSource,
        snapshot: OverviewSnapshot? = nil,
        now: @escaping () -> Date = Date.init,
        defaults: UserDefaults = .standard,
        openFile: @escaping (OverviewFile) -> Void
    ) {
        self.source = source
        self.now = now
        self.defaults = defaults
        self.openFile = openFile
        do {
            show(try snapshot ?? source.snapshot())
        } catch {
            self.error = error.message
        }
    }

    // MARK: Header

    var title: String { snapshot?.repository ?? "" }

    /// The operation in plain words, or that there is none.
    var summary: String {
        guard let summary = snapshot?.summary else {
            // `git stash pop` leaves conflicts with no operation to go on.
            return rows.isEmpty
                ? "No merge, rebase, cherry-pick or revert in progress."
                : "Conflicts with no operation in progress, as git stash pop leaves them. Stage each file once resolved."
        }
        // The step line under it names the commit: the core's headline,
        // without " · commit 2 of 7: …".
        if stepLine != nil, let headline = snapshot?.headline {
            return headline
        }
        return summary
    }

    var hasOperation: Bool { snapshot?.operation != nil }

    /// The step git stopped on: "Step 2 of 7 · reword 351410c “wip upload
    /// queue”"; `nil` outside a rebase.
    var stepLine: String? {
        guard let steps = snapshot?.plan?.steps, let step = currentStep else { return nil }
        let id = step.shortId.map { " \($0)" } ?? ""
        return "Step \(step.number) of \(steps.count) · \(step.actionName)\(id) “\(step.subject)”"
    }

    // MARK: Reword

    /// Whether git stopped on a `reword` line, for its message.
    var isRewording: Bool { snapshot?.reword != nil }

    /// The subject's length that git log, GitHub and the plan show whole.
    static let subjectLimit = Int(commitSubjectLimit())

    /// The core's check of the message being written.
    private var messageCheck: MessageCheck { checkMessage(message: rewordMessage) }

    /// "Commit message · 351410c", over the editor in place of the files.
    var rewordTitle: String {
        "Commit message" + (currentStep?.shortId.map { " · \($0)" } ?? "")
    }

    /// "Message for 351410c · used when you continue", over the editor
    /// docked under the files.
    var rewordDockTitle: String {
        "Message for \(currentStep?.shortId ?? "this commit") · used when you continue"
    }

    var subjectLength: Int { Int(messageCheck.subjectLength) }
    var subjectLimit: Int { Self.subjectLimit }
    var subjectTooLong: Bool { messageCheck.subjectTooLong }

    /// What the editor's note says: a problem first, then whether the
    /// message is the commit's own.
    var rewordNote: MessageNote {
        if rewordMessageIsEmpty {
            return MessageNote(text: "Continue needs a message.", warning: true)
        }
        if subjectTooLong {
            return MessageNote(
                text: "The subject is \(subjectLength) characters. git log, GitHub and the rebase plan cut it at about \(subjectLimit); consider moving the end to the body.",
                warning: true
            )
        }
        if secondLineIsText {
            return MessageNote(
                text: "Leave line 2 empty: git takes everything before the first blank line as the subject.",
                warning: true
            )
        }
        return MessageNote(text: rewordEdited ? "Edited." : "Original message, unchanged.", warning: false)
    }

    /// The docked editor's short status, beside Revert to Original.
    var rewordStatus: MessageNote {
        if rewordMessageIsEmpty { return MessageNote(text: "No message", warning: true) }
        if subjectTooLong { return MessageNote(text: "Subject: \(subjectLength) of \(subjectLimit)", warning: true) }
        if secondLineIsText { return MessageNote(text: "Line 2 not empty", warning: true) }
        return MessageNote(text: rewordEdited ? "Edited" : "Original message", warning: false)
    }

    /// The files the reworded commit changes, once git made it.
    var rewordFiles: [ChangedFileRow] {
        (snapshot?.reword?.commit?.files ?? []).map { file in
            let binary = file.added == nil
            return ChangedFileRow(
                path: file.path,
                name: file.name,
                directory: file.directory.isEmpty ? "repository root" : file.directory,
                added: binary ? "binary" : file.added.flatMap { $0 > 0 ? "+\($0)" : nil },
                removed: file.removed.flatMap { $0 > 0 ? "−\($0)" : nil }
            )
        }
    }

    /// "You · 3 days ago · replayed without conflicts".
    var rewordCommitDetail: String {
        guard let commit = snapshot?.reword?.commit else { return "" }
        let formatter = RelativeDateTimeFormatter()
        formatter.unitsStyle = .full
        let date = Date(timeIntervalSince1970: TimeInterval(commit.authoredAt))
        let who = commit.byYou ? "You" : commit.author
        return "\(who) · \(formatter.localizedString(for: date, relativeTo: now())) · replayed without conflicts"
    }

    /// Git made the commit and left no files: the message is all there is
    /// to do, so it takes the place of the file list.
    var rewordOnly: Bool {
        snapshot?.reword?.committed == true && files.isEmpty
    }

    /// The message differs from the commit's own.
    var rewordEdited: Bool {
        guard let reword = snapshot?.reword else { return false }
        return Self.trimmed(rewordMessage) != Self.trimmed(reword.message)
    }

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
            let joined = step.action == .fixup || step.action == .squash
            return PlanRow(
                number: Int(step.number),
                action: step.actionName,
                kind: step.action,
                shortId: step.shortId,
                subject: step.subject,
                idHelp: step.replayedId.map { "Replayed as \($0)" },
                was: step.was.map { "was “\($0)”" },
                state: step.state,
                joined: joined,
                note: joined ? step.joins.map { "into \($0)" } : nil,
                dropped: step.action == .drop,
                conflicted: step.state == .current && (
                    step.action == .reword ? snapshot?.reword?.committed == false
                        : step.action != .edit && step.action != .break
                ),
                detail: detail(step)
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
            plan.steps.first { $0.number == number }?.actionName
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
                    // Only while there is something to review: a file resolved
                    // in Scry, or fully reviewed, reads Resolved alone.
                    review: file.state == .needsReview ? file.review.map { review in
                        review.changedOutside ? "Changed outside its conflicts"
                            : "\(review.reviewed) of \(review.total) reviewed"
                    } : nil,
                    explanation: explanationLine(file.path),
                    notReviewed: Self.notReviewed(file),
                    canOpen: file.canOpen,
                    special: file.state == .staged ? nil : file.special,
                    canStage: file.canStage,
                    canUnstage: file.canUnstage,
                    isSpecial: file.special != nil,
                    absolutePath: file.absolutePath
                )
            }
    }

    // MARK: Explain

    /// Where Explain stands in the repository, as last read.
    private(set) var explanations: ExplainOverview?

    /// Reads it again: the view does it every two seconds while it shows,
    /// since Claude Code answers from outside the app.
    func refreshExplanations() async {
        let source = source
        explanations = try? await Task.detached { try source.explanations() }.value
    }

    /// "Claude Code is here" in the toolbar; `nil` before the first read,
    /// or once Settings › Agent found Scry's plugin off.
    var watching: Bool? {
        guard ClaudeCodePlugin.isOn(in: defaults), let explanations else { return nil }
        return explanations.watching
    }

    /// The line under a file's state while one of its conflicts waits for
    /// an explanation or a proposal; none with Explain and Suggest off.
    private func explanationLine(_ path: String) -> String? {
        guard InspectorViewModel.explainEnabled(in: defaults),
              let file = explanations?.files.first(where: { $0.path == path }),
              file.waiting + file.suggesting > 0
        else { return nil }
        var parts: [String] = []
        if file.waiting > 0 {
            if file.explained > 0 { parts.append("\(file.explained) explained") }
            parts.append("\(file.waiting) waiting")
        }
        if file.suggesting > 0 {
            parts.append(file.suggesting == 1 ? "1 suggestion waiting" : "\(file.suggesting) suggestions waiting")
        }
        return parts.joined(separator: " · ")
    }

    // MARK: Row menu

    /// The menu for these rows; `nil` when none is in the table.
    func menu(for paths: Set<String>) -> RowMenu? {
        let rows = rows.filter { paths.contains($0.path) }
        guard let first = rows.first else { return nil }
        return RowMenu(
            open: rows.count == 1 ? Self.openItem(first) : MenuItem(
                title: "Open in Merge Editor", detail: "Select one file to open it", enabled: false
            ),
            stage: Self.stageItem(rows),
            paths: rows.map(\.path),
            absolutePaths: rows.map(\.absolutePath)
        )
    }

    /// The selection's menu, for the File menu.
    var selectionMenu: RowMenu? { menu(for: selection) }

    /// The menu's second item: stages the rows, or unstages them.
    func stageOrUnstage(_ paths: [String]) {
        let rows = rows.filter { paths.contains($0.path) }
        guard !rows.isEmpty, Self.stageItem(rows).enabled else { return }
        let paths = rows.map(\.path)
        if rows.allSatisfy(\.canUnstage) {
            run(.unstage(paths: paths))
        } else {
            stageAsking(rows)
        }
    }

    private static func openItem(_ row: FileRow) -> MenuItem {
        if row.state == .staged {
            return MenuItem(
                title: row.isSpecial ? "Show Choice" : "Open in Merge Editor",
                detail: "Unstage to change it",
                enabled: false
            )
        }
        let title = row.state == .needsReview ? (row.review != nil ? "Review in Merge Editor" : "Review Auto-resolution")
            : row.special != nil ? "Open…" : "Open in Merge Editor"
        return MenuItem(title: title, detail: row.canOpen ? nil : "Scry cannot read this file", enabled: row.canOpen)
    }

    private static func stageItem(_ rows: [FileRow]) -> MenuItem {
        // No row: nothing to stage, rather than reading the first row below.
        guard let first = rows.first else {
            return MenuItem(title: "Stage", detail: nil, enabled: false)
        }
        let many = rows.count > 1
        let files = "\(rows.count) Files"
        if rows.allSatisfy(\.canUnstage) {
            if many {
                return MenuItem(
                    title: "Unstage \(files)",
                    detail: "Git gets the conflicts back; each file keeps your result.",
                    enabled: true
                )
            }
            return first.isSpecial
                ? MenuItem(
                    title: "Unstage and Choose Again",
                    detail: "Git gets the conflict back and asks again. Your choice is not kept.",
                    enabled: true
                )
                : MenuItem(
                    title: "Unstage",
                    detail: "Git gets the conflict back; the file keeps your result, now shown as Resolved.",
                    enabled: true
                )
        }
        let unreviewed = rows.filter { $0.notReviewed != nil }
        let title = many ? "Stage \(files)"
            : first.notReviewed != nil ? "Stage…"
            : first.state == .needsReview ? "Accept and Stage" : "Stage"
        if rows.allSatisfy(\.canStage) {
            let detail: String? = if let reason = first.notReviewed, !many {
                reason.prefix(1).uppercased() + reason.dropFirst()
            } else if !unreviewed.isEmpty {
                "\(unreviewed.count) of them not fully reviewed"
            } else if !many && first.state == .needsReview {
                "Keeps the rule’s result as it is"
            } else {
                nil
            }
            return MenuItem(title: title, detail: detail, enabled: true)
        }
        if rows.contains(where: \.canUnstage) {
            return MenuItem(title: title, detail: "Some of these files are staged already", enabled: false)
        }
        return MenuItem(title: title, detail: many ? "Resolve every file first" : "Resolve the file first", enabled: false)
    }

    // MARK: Footer

    private var verb: String { snapshot?.operation ?? "operation" }

    var abortTitle: String { "Abort \(verb)…" }
    var abortMessage: String {
        "Gives up the whole \(verb) and puts the branch back as it was before. Choices made in its files are lost."
    }

    var skipTitle: String { "Skip this commit?" }
    var skipMessage: String {
        "Leaves the commit being applied out of the \(verb): its changes and the choices made in its files are lost. Git keeps the original commit in its reflog."
    }

    /// "3 files still unresolved · 1 to review".
    var unresolvedSummary: String {
        let unresolved = count(.unresolved)
        let review = count(.needsReview)
        var parts: [String] = []
        if unresolved > 0 { parts.append("\(unresolved) \(unresolved == 1 ? "file" : "files") still unresolved") }
        if review > 0 { parts.append("\(review) to review") }
        return parts.joined(separator: " · ")
    }

    /// The footer's sentence: git at work, why Continue waits for the
    /// message, or what is left.
    var footerNote: String {
        isRunning ? "Git is running…" : continueReason ?? unresolvedSummary
    }

    /// Why Continue is disabled, when the message is the reason: the
    /// only sentence the footer has beside it.
    var continueReason: String? {
        isRewording && rewordMessageIsEmpty ? "Write a message to continue" : nil
    }

    /// A reword git committed has nothing to stage or skip.
    var showsStage: Bool { !rewordOnly }

    var stageTitle: String { "Stage resolved (\(count(.resolved)))" }
    var canStage: Bool { !isRunning && count(.resolved) > 0 }
    var continueTitle: String { "Continue \(verb)" }
    /// Every file staged, and a message when rewording: git refuses an
    /// empty one.
    /// The toolbar's shortcuts, for the hint row under it while ⌘ is held.
    var toolbarHints: [Shortcut] { [.refresh] }

    /// The shortcuts with no visible control, for the hint strip while ⌘
    /// is held.
    var hintStrip: [Shortcut] {
        var shortcuts: [Shortcut] = rows.contains(where: \.canOpen) ? [.openFile] : []
        shortcuts += [.openRepository, .openConflictedFile, .keyboardShortcuts]
        return shortcuts
    }

    var canContinue: Bool {
        !isRunning
            && (snapshot?.canContinue ?? false)
            && (!isRewording || !rewordMessageIsEmpty)
    }
    var canSkip: Bool { snapshot?.canSkip ?? false }
    var canAbort: Bool { hasOperation && !isRunning }

    // MARK: Intents

    /// Reads the operation again, off the main actor: files change as they
    /// are resolved in the editor, or by hand in another tool. Skipped while
    /// git runs, whose result is read anyway.
    func refresh() {
        guard !isRunning else { return }
        let source = source
        perform {
            do {
                return (try source.snapshot(), nil)
            } catch {
                return (nil, error.message)
            }
        }
    }

    /// Waits for the git action or read in flight, if any.
    func idle() async {
        await work?.value
    }

    /// Shows a snapshot read elsewhere: by a special file's view, or after
    /// an action.
    func show(_ snapshot: OverviewSnapshot) {
        self.snapshot = snapshot
        let step = snapshot.reword.map { "\(snapshot.step ?? 0) \($0.message)" }
        if step != rewordStep {
            rewordStep = step
            rewordMessage = snapshot.reword?.message ?? ""
        }
        let files = rewordFiles
        if !files.contains(where: { $0.path == rewordSelection }) {
            selectRewordFile(files.first?.path)
        }
    }

    /// Shows a file of the reworded commit in the diff.
    func selectRewordFile(_ path: String?) {
        rewordSelection = path
        guard let path, let commit = snapshot?.reword?.commit else {
            rewordDiff = []
            return
        }
        do {
            rewordDiff = try source.commitDiff(commit: commit.id, path: path).enumerated().map { index, line in
                DiffRow(
                    id: index,
                    kind: line.kind,
                    number: (line.kind == .removed ? line.oldNumber : line.newNumber).map(String.init) ?? "",
                    text: line.kind == .hunk ? "@@ \(line.text)" : line.text
                )
            }
        } catch {
            rewordDiff = []
            self.error = error.message
        }
    }

    /// Back to the commit's own message.
    func resetRewordMessage() {
        rewordMessage = snapshot?.reword?.message ?? ""
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
                changed: { [weak self] snapshot in self?.show(snapshot) },
                done: { [weak self] in self?.special = nil },
                openInEditor: { [weak self] file in
                    self?.special = nil
                    self?.openFile(file)
                }
            )
        } else if file.openable {
            openFile(file)
        }
    }

    /// Cancel: closes the sheet and reads the list again, as the user may
    /// have changed the file meanwhile, in a terminal or with Take Yours.
    func closeSpecial() {
        guard special != nil else { return }
        special = nil
        refresh()
    }

    func stageResolved() {
        guard canStage else { return }
        run(.stageResolved)
    }

    /// Stages one resolved file, after asking when its review is not
    /// done.
    func stage(_ path: String) {
        guard let row = rows.first(where: { $0.path == path }), row.canStage else { return }
        stageAsking([row])
    }

    /// Stages the files the user was asked about, once they agreed.
    func confirmStage() {
        guard let confirmation = stageConfirmation else { return }
        stageConfirmation = nil
        run(.stage(paths: confirmation.paths))
    }

    /// Gives a staged file its conflict back, keeping what was resolved.
    func unstage(_ path: String) {
        guard rows.first(where: { $0.path == path })?.canUnstage == true else { return }
        run(.unstage(paths: [path]))
    }

    func continueOperation() {
        guard canContinue else { return }
        run(.continue(message: isRewording ? Self.trimmed(rewordMessage) : nil))
    }

    /// Leaves the commit out, once the user confirmed.
    func skip() {
        guard canSkip else { return }
        run(.skip)
    }

    /// Gives up the operation, once the user confirmed.
    func abort() {
        guard canAbort else { return }
        run(.abort)
    }

    // MARK: Private

    private var files: [OverviewFile] { snapshot?.files ?? [] }

    private var currentStep: RebaseStep? {
        snapshot?.plan?.steps.first { $0.state == .current }
    }

    private var rewordMessageIsEmpty: Bool { messageCheck.empty }

    /// Text on line 2, where git expects the blank line after the subject.
    private var secondLineIsText: Bool { messageCheck.secondLineIsText }

    /// Without the trailing blank lines and spaces git drops anyway.
    private static func trimmed(_ message: String) -> String {
        var message = message
        while message.last?.isWhitespace == true { message.removeLast() }
        return message
    }

    /// Stages `rows`, or asks first when some were resolved outside Scry
    /// and not fully reviewed (decision 4: it asks, it does not block).
    private func stageAsking(_ rows: [FileRow]) {
        let paths = rows.map(\.path)
        let unreviewed = rows.filter { $0.notReviewed != nil }
        guard let first = unreviewed.first, let reason = first.notReviewed else {
            run(.stage(paths: paths))
            return
        }
        let message = unreviewed.count == 1
            ? "\(first.name): \(reason). Staging takes the file as it is."
            : "\(unreviewed.count) files are not fully reviewed. Staging takes them as they are."
        stageConfirmation = StageConfirmation(
            title: unreviewed.count == 1 ? "Stage \(first.name) without reviewing?" : "Stage without reviewing?",
            message: message,
            paths: paths
        )
    }

    /// What a file's review leaves: "2 conflicts not reviewed", "changed
    /// outside its conflicts"; `nil` once reviewed, or for a file not
    /// resolved outside Scry.
    private static func notReviewed(_ file: OverviewFile) -> String? {
        guard file.state == .needsReview, let review = file.review else { return nil }
        if review.changedOutside { return "changed outside its conflicts, not reviewed" }
        let left = Int(review.total) - Int(review.reviewed)
        guard left > 0 else { return nil }
        return "\(left) \(left == 1 ? "conflict" : "conflicts") not reviewed"
    }

    private func count(_ state: ConflictState) -> Int {
        files.filter { $0.state == state }.count
    }

    /// Runs the action off the main actor, one at a time; the core reads
    /// the operation again after it. A failure stays shown with the
    /// operation read again, as git may have moved anyway.
    private func run(_ action: OverviewAction) {
        guard !isRunning else { return }
        let source = source
        perform {
            do {
                return (try source.apply(action: action).snapshot, nil)
            } catch {
                return (try? source.snapshot(), error.message)
            }
        }
    }

    /// Runs `body` off the main actor while `isRunning`, then shows the
    /// snapshot it read and its failure, if any.
    private func perform(_ body: @escaping @Sendable () -> (OverviewSnapshot?, String?)) {
        isRunning = true
        work = Task { [weak self] in
            let (snapshot, failure) = await Task.detached { body() }.value
            guard let self else { return }
            isRunning = false
            if let snapshot { show(snapshot) }
            error = failure
        }
    }

    private static func commits(_ count: Int) -> String {
        "\(count) \(count == 1 ? "commit" : "commits")"
    }

    private func detail(_ step: RebaseStep) -> String {
        let target = step.joins.map { "step \($0)" } ?? "the commit before"
        switch step.state {
        case .current:
            switch step.action {
            case .edit, .break: return "Git stopped here, as the plan asks. Continue when ready."
            case .reword where snapshot?.reword?.committed == false:
                return "Git stopped on conflicts: resolve and stage the files, write the commit’s message, then continue."
            case .reword: return "Git stopped here: write the commit’s message, then continue."
            default: return "Git stopped here: resolve its files, then continue."
            }
        case .done:
            switch step.action {
            case .drop: return "Dropped: this commit was not replayed."
            case .fixup, .squash: return "Folded into \(target)."
            case .exec: return "Ran: \(step.subject)"
            default: return "Replayed."
            }
        case .pending:
            switch step.action {
            case .pick: return "To come: git replays this commit."
            case .reword: return "To come: git replays it, then stops so you can write its message."
            case .edit: return "To come: git replays it, then stops so you can amend it."
            case .fixup: return "To come: folded into \(target), keeping that commit's message."
            case .squash: return "To come: folded into \(target), messages combined."
            case .drop: return "Dropped: this commit will not be replayed."
            case .exec: return "To come: git runs `\(step.subject)`."
            case .break: return "To come: git pauses here."
            case .other: return "To come."
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
