import SwiftUI

/// S1: the operation in progress and its conflicted files.
struct OverviewView: View {
    @Bindable var viewModel: OverviewViewModel

    var body: some View {
        VStack(spacing: 0) {
            OverviewHeader(
                summary: viewModel.summary,
                step: viewModel.stepLine,
                applied: viewModel.appliedNote,
                progress: viewModel.hasOperation && !viewModel.rewordOnly ? viewModel.progress : nil,
                detail: viewModel.progressDetail,
                segments: viewModel.progressSegments
            )
            Divider()
            HStack(spacing: 0) {
                if viewModel.rewordOnly {
                    RewordPanel(
                        title: viewModel.rewordTitle,
                        message: $viewModel.rewordMessage,
                        note: viewModel.rewordNote,
                        edited: viewModel.rewordEdited,
                        files: viewModel.rewordFiles,
                        selection: viewModel.rewordSelection,
                        diff: viewModel.rewordDiff,
                        commitDetail: viewModel.rewordCommitDetail,
                        select: { viewModel.selectRewordFile($0) },
                        reset: { viewModel.resetRewordMessage() }
                    )
                } else {
                    OverviewSidebar(filters: viewModel.filters) { viewModel.select($0) }
                        .frame(width: 210)
                    Divider()
                    table
                }
                if let names = viewModel.historyNames {
                    Divider()
                    MergeHistoryView(
                        ours: names.ours,
                        theirs: names.theirs,
                        caption: viewModel.historyCaption,
                        rows: viewModel.historyRows
                    )
                    .frame(width: 340)
                } else if let base = viewModel.planBase {
                    Divider()
                    RebasePlanView(
                        caption: viewModel.planCaption,
                        branch: viewModel.planBranch,
                        rows: viewModel.planRows,
                        base: base
                    )
                    .frame(width: 340)
                }
            }
            if viewModel.isRewording && !viewModel.rewordOnly {
                Divider()
                RewordDock(
                    title: viewModel.rewordDockTitle,
                    message: $viewModel.rewordMessage,
                    status: viewModel.rewordStatus,
                    edited: viewModel.rewordEdited,
                    reset: { viewModel.resetRewordMessage() }
                )
                .frame(height: 176)
            }
            Divider()
            footer
        }
        .overlay(alignment: .bottom) {
            HintStrip(shortcuts: viewModel.hintStrip)
                .padding(.bottom, 48)
        }
        .overlay(alignment: .top) { ToolbarHintRow(shortcuts: viewModel.toolbarHints) }
        .focusedSceneValue(\.fileRowActions, viewModel.selectionMenu.map { menu in
            FileRowCommandActions(
                menu: menu,
                open: { if let path = menu.paths.first { viewModel.open(path) } },
                stage: { viewModel.stageOrUnstage(menu.paths) }
            )
        })
        .background(Theme.Surfaces.window)
        .navigationTitle(viewModel.title)
        .navigationSubtitle(viewModel.hasOperation ? viewModel.snapshot?.operation ?? "" : "")
        .toolbar {
            ToolbarItem {
                Button { viewModel.refresh() } label: {
                    Image(systemName: "arrow.clockwise")
                }
                .help(Shortcut.refresh.help())
                .keyboardShortcut(Shortcut.refresh)
            }
        }
        .sheet(item: Binding(get: { viewModel.special }, set: { if $0 == nil { viewModel.closeSpecial() } })) { special in
            SpecialFileView(viewModel: special) { viewModel.closeSpecial() }
        }
        .alert(viewModel.abortTitle, isPresented: $viewModel.confirmingAbort) {
            Button("Abort", role: .destructive) { viewModel.abort() }
            Button("Cancel", role: .cancel) {}
        } message: {
            Text(viewModel.abortMessage)
        }
    }

    private var table: some View {
        Table(viewModel.rows, selection: $viewModel.selection) {
            TableColumn("") { row in
                if row.canOpen {
                    Button { viewModel.open(row.path) } label: {
                        Image(systemName: row.special?.symbol ?? "square.and.pencil")
                            .rowForeground(Theme.Interaction.accent)
                    }
                    .buttonStyle(.borderless)
                    .help("\(row.special?.help ?? "Resolve in the merge editor"): \(row.name) (double-click the row)")
                    .accessibilityLabel("Resolve \(row.name)")
                } else if row.canUnstage {
                    // Staged files do not open: git has no conflict left
                    // to show. Unstage is the explicit step.
                    Image(systemName: "square.and.pencil")
                        .rowForeground(Theme.Surfaces.textTertiary, secondary: true)
                        .help("Unstage to change it")
                        .accessibilityLabel("\(row.name) is staged: unstage to change it")
                }
            }
            .width(28)
            TableColumn("State") { row in
                Label(row.stateLabel, systemImage: row.state.symbol)
                    .rowForeground(row.state.color)
                    .font(Metrics.callout)
            }
            .width(min: 110, ideal: 130)
            TableColumn("File") { row in
                VStack(alignment: .leading, spacing: 1) {
                    Text(row.name)
                        .font(Metrics.code.weight(.semibold))
                    Text(row.directory)
                        .font(Metrics.caption)
                        .rowForeground(Theme.Surfaces.textSecondary, secondary: true)
                }
            }
            .width(min: 180, ideal: 260)
            TableColumn("Kind") { row in
                Text(row.kind)
                    .font(Metrics.callout)
                    .rowForeground(Theme.Surfaces.textSecondary, secondary: true)
            }
            .width(min: 90, ideal: 140)
            TableColumn("Conflicts") { row in
                Text(row.conflicts)
                    .font(Metrics.code)
                    .frame(maxWidth: .infinity, alignment: .trailing)
            }
            .width(70)
        }
        .contextMenu(forSelectionType: String.self) { paths in
            if let menu = viewModel.menu(for: paths) {
                FileRowMenuItems(
                    menu: menu,
                    showsReturn: true,
                    open: { if let path = menu.paths.first { viewModel.open(path) } },
                    stage: { viewModel.stageOrUnstage(menu.paths) }
                )
            }
        } primaryAction: { paths in
            // Double-click, or Return on the selected row.
            if let path = paths.first { viewModel.open(path) }
        }
        .overlay {
            if viewModel.rows.isEmpty {
                Text(viewModel.hasOperation ? "No files in this state." : "Nothing to resolve.")
                    .foregroundStyle(Theme.Surfaces.textSecondary)
            }
        }
    }

    private var footer: some View {
        HStack(spacing: 10) {
            Button(viewModel.abortTitle, role: .destructive) { viewModel.confirmingAbort = true }
                .disabled(!viewModel.hasOperation)
            if let error = viewModel.error {
                Text(error)
                    .font(Metrics.callout)
                    .foregroundStyle(Theme.Severity.error)
                    .lineLimit(2)
            }
            Spacer()
            Text(viewModel.continueReason ?? viewModel.unresolvedSummary)
                .font(Metrics.callout)
                .foregroundStyle(Theme.Surfaces.textSecondary)
            if viewModel.canSkip {
                Button("Skip commit") { viewModel.skip() }
            }
            if viewModel.showsStage {
                Button(viewModel.stageTitle) { viewModel.stageResolved() }
                    .disabled(!viewModel.canStage)
            }
            Button(viewModel.continueTitle) { viewModel.continueOperation() }
                .keyboardShortcut(Shortcut.continueOperation)
                .shortcutHint(Shortcut.continueOperation, edge: .top)
                .buttonStyle(.borderedProminent)
                .disabled(!viewModel.canContinue)
                .help(Shortcut.continueOperation.help(viewModel.isRewording ? "With this message" : "Once every file is staged"))
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 10)
        .background(Theme.Surfaces.chrome)
    }
}

/// The operation, its commits and how many files are ready.
struct OverviewHeader: View {
    let summary: String
    /// "Step 2 of 7 · reword 351410c “wip upload queue”".
    let step: String?
    /// What git ran on its own since it last stopped.
    let applied: String?
    /// "2 of 6 files ready"; `nil` without an operation.
    let progress: String?
    let detail: String
    let segments: [(state: ConflictState, count: Int)]

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            VStack(alignment: .leading, spacing: 3) {
                Text(summary)
                    .font(Metrics.headline)
                if let step {
                    Text(step)
                        .font(Metrics.callout)
                        .foregroundStyle(Theme.Surfaces.textSecondary)
                        .lineLimit(1)
                        .truncationMode(.tail)
                }
            }
            if let applied {
                AppliedNote(text: applied)
            }
            if let progress {
                HStack(alignment: .firstTextBaseline, spacing: 8) {
                    Text(progress).font(Metrics.captionBold)
                    Text(detail)
                        .font(Metrics.caption)
                        .foregroundStyle(Theme.Surfaces.textSecondary)
                }
                SegmentedBar(segments: segments)
                    .frame(height: 6)
            }
        }
        .padding(16)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(Theme.Surfaces.chrome)
    }
}

/// The files' states as one bar.
struct SegmentedBar: View {
    let segments: [(state: ConflictState, count: Int)]

    var body: some View {
        GeometryReader { geometry in
            let total = max(segments.reduce(0) { $0 + $1.count }, 1)
            HStack(spacing: 2) {
                ForEach(Array(segments.enumerated()), id: \.offset) { _, segment in
                    RoundedRectangle(cornerRadius: 2)
                        .fill(segment.state.color)
                        .frame(width: max(geometry.size.width * CGFloat(segment.count) / CGFloat(total) - 2, 2))
                }
            }
        }
    }
}

/// The filters, each with its state's glyph and count.
struct OverviewSidebar: View {
    let filters: [OverviewViewModel.FilterRow]
    let select: (OverviewViewModel.Filter) -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            Text("Files")
                .font(Metrics.captionBold)
                .foregroundStyle(Theme.Surfaces.textSecondary)
                .padding(.horizontal, 8)
                .padding(.bottom, 4)
            ForEach(filters) { row in
                Button { select(row.filter) } label: {
                    HStack {
                        Image(systemName: row.state?.symbol ?? "doc.on.doc")
                            .foregroundStyle(row.state?.color ?? Theme.Surfaces.textSecondary)
                            .frame(width: 16)
                        Text(row.label)
                        Spacer()
                        Text("\(row.count)")
                            .foregroundStyle(Theme.Surfaces.textSecondary)
                            .monospacedDigit()
                    }
                    .font(Metrics.callout)
                    .padding(.horizontal, 8)
                    .padding(.vertical, 5)
                    .background(
                        RoundedRectangle(cornerRadius: 6)
                            .fill(row.selected ? Theme.Interaction.selection : Color.clear)
                    )
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .accessibilityAddTraits(row.selected ? .isSelected : [])
            }
            Spacer()
        }
        .padding(10)
        .background(Theme.Surfaces.sidebar)
    }
}

/// Snapshots for previews, without touching a repository.
private struct PreviewOverview: OverviewSource {
    var operation: String? = "rebase"
    var files: [OverviewFile]
    /// The step git stopped on, and the steps it ran on its own before.
    var current: UInt32 = 3
    var applied: [UInt32] = [2]
    /// Stopped on a `reword` line.
    var reword: RewordInfo?

    func snapshot() throws -> OverviewSnapshot {
        OverviewSnapshot(
            repository: "offline-sync",
            root: "/Users/you/Code/offline-sync",
            operation: operation,
            summary: operation == nil ? nil : "Rebasing feature/sync-v2 onto main · commit \(current) of \(Self.steps.count): \(Self.steps[Int(current) - 1].shortId ?? "") \(Self.steps[Int(current) - 1].subject)",
            step: operation == nil ? nil : current,
            steps: operation == nil ? nil : UInt32(Self.steps.count),
            plan: operation == nil ? nil : RebasePlan(
                ontoName: "main",
                branch: "feature/sync-v2",
                ontoShortId: "9f3e2c1",
                ontoSubject: "Serve the sync status as JSON",
                steps: Self.steps.map { step in
                    var step = step
                    step.state = step.number < current ? .done : step.number == current ? .current : .pending
                    if step.state == .done, step.shortId != nil, step.action != "drop" {
                        step.replayedId = "6b7c8d\(step.number)"
                        if step.action == "reword" {
                            step.was = step.subject
                            step.subject = "Document the sync queue"
                        }
                    }
                    return step
                }
            ),
            merge: nil,
            applied: operation == nil ? [] : applied,
            reword: operation == nil ? nil : reword,
            files: files,
            canContinue: operation != nil && files.allSatisfy { $0.state == .staged },
            // As the core: no skipping a reword git already committed.
            canSkip: operation != nil && reword?.committed != true
        )
    }

    func apply(action: OverviewAction) throws {}
    func deletion(path: String) throws -> DeletionInfo { try SpecialSamples().deletion(path: path) }
    func binary(path: String) throws -> BinaryInfo { try SpecialSamples().binary(path: path) }
    func project(path: String) throws -> ProjectInfo { try SpecialSamples().project(path: path) }
    func catalog(path: String) throws -> CatalogInfo { try SpecialSamples().catalog(path: path) }
    func packages(path: String) throws -> PackagesInfo { try SpecialSamples().packages(path: path) }
    func lockfile(path: String) throws -> LockfileInfo { try SpecialSamples().lockfile(path: path) }
    func commitDiff(commit: String, path: String) throws -> [CommitDiffLine] { Self.diff }

    /// The same files, in a merge of main into the branch.
    struct Merge: OverviewSource {
        var files: [OverviewFile]

        func snapshot() throws -> OverviewSnapshot {
            let commit = { (id: String, subject: String) in HistoryCommit(shortId: id, subject: subject) }
            return OverviewSnapshot(
                repository: "atlas",
                root: "/Users/you/Code/atlas",
                operation: "merge",
                summary: "Merging main into feature/sync-v2",
                step: nil,
                steps: nil,
                plan: nil,
                merge: MergeHistory(
                    ours: HistoryLane(name: "feature/sync-v2", commits: PreviewOverview.steps.filter { $0.shortId != nil }.reversed().prefix(4).map {
                        commit($0.shortId ?? "", $0.subject)
                    }, more: 3),
                    theirs: HistoryLane(name: "main", commits: [
                        commit("9f3e2c1", "Serve the sync status as JSON, reword French strings"),
                        commit("71ab09e", "Patch dependencies, Xcode 15.4, thread-safe OldCache"),
                        commit("4c2d8f0", "Rename name to displayName and add avatars (API v2)"),
                        commit("b81e6a2", "Send changes in batches, with a bounded queue"),
                        commit("3e9f1c7", "Add a 30 s upload timeout; replace ForecastView with a forecast cache; version 2.0 (#412)"),
                    ], more: 0),
                    base: commit("a40c2d1", "Initial Atlas app")
                ),
                applied: [],
                reword: nil,
                files: files,
                canContinue: files.allSatisfy { $0.state == .staged },
                canSkip: false
            )
        }

        func apply(action: OverviewAction) throws {}
        func deletion(path: String) throws -> DeletionInfo { try SpecialSamples().deletion(path: path) }
        func binary(path: String) throws -> BinaryInfo { try SpecialSamples().binary(path: path) }
        func project(path: String) throws -> ProjectInfo { try SpecialSamples().project(path: path) }
        func catalog(path: String) throws -> CatalogInfo { try SpecialSamples().catalog(path: path) }
        func packages(path: String) throws -> PackagesInfo { try SpecialSamples().packages(path: path) }
        func lockfile(path: String) throws -> LockfileInfo { try SpecialSamples().lockfile(path: path) }
        func commitDiff(commit: String, path: String) throws -> [CommitDiffLine] { [] }
    }

    static func file(_ path: String, _ kind: String, _ state: ConflictState, _ conflicts: UInt32?) -> OverviewFile {
        let parts = path.split(separator: "/")
        return OverviewFile(
            path: path,
            absolutePath: "/Users/you/Code/offline-sync/" + path,
            name: String(parts.last ?? ""),
            directory: parts.dropLast().joined(separator: "/"),
            kind: kind,
            state: state,
            conflicts: conflicts,
            openable: kind != "Deleted on one side" && kind != "Binary",
            special: kind == "Deleted on one side" ? .deletedOnOneSide
                : kind == "Xcode project" ? .projectFile
                : kind == "String catalog" ? .stringCatalog
                : kind == "Binary" ? .binary : nil
        )
    }

    static func step(_ number: UInt32, _ action: String, _ id: String?, _ subject: String, joins: UInt32? = nil) -> RebaseStep {
        RebaseStep(
            number: number, action: action, shortId: id, subject: subject, replayedId: nil, was: nil,
            state: .pending, joins: joins
        )
    }

    /// The commit a reword stopped on, as git replayed it.
    static let rewordCommit = RewordCommitInfo(
        id: "6b7c8d9e0f", author: "You", byYou: true,
        authoredAt: Int64(Date().addingTimeInterval(-3 * 86_400).timeIntervalSince1970),
        files: [
            ChangedFileInfo(path: "README.md", name: "README.md", directory: "", added: 14, removed: 2),
            ChangedFileInfo(path: "docs/sync-queue.md", name: "sync-queue.md", directory: "docs/", added: 40, removed: 0),
            ChangedFileInfo(path: "docs/queue.png", name: "queue.png", directory: "docs/", added: nil, removed: nil),
        ]
    )

    static let diff: [CommitDiffLine] = {
        let line = { (kind: DiffLineKind, old: UInt32?, new: UInt32?, text: String) in
            CommitDiffLine(kind: kind, oldNumber: old, newNumber: new, text: text)
        }
        return [
            line(.hunk, nil, nil, "## Offline sync"),
            line(.context, 12, 12, "Uploads made offline wait in a queue."),
            line(.removed, 13, nil, "They are sent later."),
            line(.added, nil, 13, "They are sent when the network returns, oldest first,"),
            line(.added, nil, 14, "in batches of 20; a failed batch is retried with backoff."),
            line(.context, 14, 15, ""),
            line(.hunk, nil, nil, "## Settings"),
            line(.added, nil, 31, "Settings › Sync lists the uploads still waiting."),
        ]
    }()

    static let steps = [
        step(1, "pick", "0572f71", "Queue uploads while offline"),
        step(2, "reword", "e076115", "Explain the sync queue in the README"),
        step(3, "pick", "d66f4d4", "Show the user's full name"),
        step(4, "fixup", "9d65bc2", "fixup! Show the user's full name", joins: 3),
        step(5, "pick", "1d18a33", "Upgrade dependencies and CI"),
        step(6, "drop", "e3e1c07", "WIP debug logging"),
        step(7, "exec", nil, "make test"),
        step(8, "pick", "c439f41", "Translate the sync messages"),
    ]

    static let mixed = [
        file("Sources/Sync/SyncEngine.swift", "Swift", .unresolved, 2),
        file("App.xcodeproj/project.pbxproj", "Xcode project", .unresolved, 3),
        file("Resources/Localizable.xcstrings", "String catalog", .resolved, 0),
        file("Sources/Legacy/OldCache.swift", "Deleted on one side", .unresolved, nil),
        file("Resources/AppIcon.png", "Binary", .unresolved, nil),
        file("README.md", "Text", .staged, nil),
    ]
}

#Preview("Overview, rebase in progress", traits: .fixedLayout(width: 1300, height: 640)) {
    OverviewView(viewModel: OverviewViewModel(source: PreviewOverview(files: PreviewOverview.mixed)) { _ in })
}

#Preview("Overview, a row selected", traits: .fixedLayout(width: 1300, height: 640)) {
    let viewModel = OverviewViewModel(source: PreviewOverview(files: PreviewOverview.mixed)) { _ in }
    viewModel.selection = ["Sources/Sync/SyncEngine.swift"]
    return OverviewView(viewModel: viewModel)
}

#Preview("Overview, git ran steps on its own", traits: .fixedLayout(width: 1300, height: 640)) {
    OverviewView(viewModel: OverviewViewModel(
        source: PreviewOverview(files: PreviewOverview.mixed.map {
            PreviewOverview.file($0.path, $0.kind, .staged, nil)
        }, current: 8, applied: [6, 7])
    ) { _ in })
    .preferredColorScheme(.dark)
}

#Preview("Overview, stopped on a reword", traits: .fixedLayout(width: 1300, height: 700)) {
    OverviewView(viewModel: OverviewViewModel(
        source: PreviewOverview(
            files: [],
            current: 2,
            applied: [],
            reword: RewordInfo(
                message: "Explain the sync queue in the README while the device is offline and how it drains\n\nUploads wait in a queue until the network returns.",
                committed: true,
                commit: PreviewOverview.rewordCommit
            )
        )
    ) { _ in })
}

#Preview("Overview, reword with conflicts", traits: .fixedLayout(width: 1300, height: 700)) {
    OverviewView(viewModel: OverviewViewModel(
        source: PreviewOverview(
            files: Array(PreviewOverview.mixed.prefix(2)),
            current: 2,
            applied: [],
            reword: RewordInfo(message: "Explain the sync queue in the README", committed: false, commit: nil)
        )
    ) { _ in })
}

#Preview("Overview, nothing in progress", traits: .fixedLayout(width: 1100, height: 640)) {
    OverviewView(viewModel: OverviewViewModel(source: PreviewOverview(operation: nil, files: [])) { _ in })
}

#Preview("Rebase plan, first stop", traits: .fixedLayout(width: 340, height: 480)) {
    let viewModel = OverviewViewModel(source: PreviewOverview(files: PreviewOverview.mixed, current: 1, applied: [])) { _ in }
    RebasePlanView(
        caption: viewModel.planCaption,
        branch: viewModel.planBranch,
        rows: viewModel.planRows,
        base: viewModel.planBase!
    )
}

#Preview("Overview, merge", traits: .fixedLayout(width: 1300, height: 640)) {
    OverviewView(viewModel: OverviewViewModel(source: PreviewOverview.Merge(files: PreviewOverview.mixed)) { _ in })
}

/// A table cell's colour, which turns to the selection's text colour on a
/// selected row: the theme's red or grey on the system's blue cannot be
/// read.
private struct RowForeground: ViewModifier {
    let color: Color
    let secondary: Bool
    @Environment(\.backgroundProminence) private var prominence

    func body(content: Content) -> some View {
        if prominence == .increased {
            content.foregroundStyle(secondary ? AnyShapeStyle(.secondary) : AnyShapeStyle(.primary))
        } else {
            content.foregroundStyle(color)
        }
    }
}

private extension View {
    func rowForeground(_ color: Color, secondary: Bool = false) -> some View {
        modifier(RowForeground(color: color, secondary: secondary))
    }
}
