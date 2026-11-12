import SwiftUI

/// S1: the operation in progress and its conflicted files.
struct OverviewView: View {
    @Bindable var viewModel: OverviewViewModel
    /// The table's selected row, by path: purely visual.
    @State private var selection: String?

    var body: some View {
        VStack(spacing: 0) {
            OverviewHeader(
                summary: viewModel.summary,
                applied: viewModel.appliedNote,
                progress: viewModel.hasOperation && !viewModel.rewordOnly ? viewModel.progress : nil,
                detail: viewModel.progressDetail,
                segments: viewModel.progressSegments
            )
            Divider()
            HStack(spacing: 0) {
                if viewModel.rewordOnly {
                    rewordPanel
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
                rewordPanel
                    .frame(height: 170)
            }
            Divider()
            footer
        }
        .background(Theme.Surfaces.window)
        .navigationTitle(viewModel.title)
        .navigationSubtitle(viewModel.hasOperation ? viewModel.snapshot?.operation ?? "" : "")
        .toolbar {
            ToolbarItem {
                Button { viewModel.refresh() } label: {
                    Image(systemName: "arrow.clockwise")
                }
                .help("Read the files again (⌘R)")
                .keyboardShortcut("r")
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

    private var rewordPanel: some View {
        RewordPanel(
            title: viewModel.rewordTitle,
            help: viewModel.rewordHelp,
            message: $viewModel.rewordMessage,
            edited: viewModel.rewordEdited,
            reset: { viewModel.resetRewordMessage() }
        )
    }

    private var table: some View {
        Table(viewModel.rows, selection: $selection) {
            TableColumn("") { row in
                if row.canOpen {
                    Button { viewModel.open(row.path) } label: {
                        Image(systemName: row.special?.symbol ?? "square.and.pencil")
                            .foregroundStyle(Theme.Interaction.accent)
                    }
                    .buttonStyle(.borderless)
                    .help("\(row.special?.help ?? "Resolve in the merge editor"): \(row.name) (double-click the row)")
                    .accessibilityLabel("Resolve \(row.name)")
                }
            }
            .width(28)
            TableColumn("State") { row in
                Label(row.stateLabel, systemImage: row.state.symbol)
                    .foregroundStyle(row.state.color)
                    .font(Metrics.callout)
            }
            .width(min: 110, ideal: 130)
            TableColumn("File") { row in
                VStack(alignment: .leading, spacing: 1) {
                    Text(row.name)
                        .font(Metrics.code.weight(.semibold))
                    Text(row.directory)
                        .font(Metrics.caption)
                        .foregroundStyle(Theme.Surfaces.textSecondary)
                }
            }
            .width(min: 180, ideal: 260)
            TableColumn("Kind") { row in
                Text(row.kind)
                    .font(Metrics.callout)
                    .foregroundStyle(Theme.Surfaces.textSecondary)
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
            if let path = paths.first, let row = viewModel.rows.first(where: { $0.path == path }) {
                if row.canOpen {
                    Button(row.special?.help ?? "Resolve in Merge Editor") { viewModel.open(path) }
                }
                if row.canStage {
                    Button("Stage") { viewModel.stage(path) }
                }
                if row.canUnstage {
                    Button("Unstage") { viewModel.unstage(path) }
                }
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
            Text(viewModel.unresolvedSummary)
                .font(Metrics.callout)
                .foregroundStyle(Theme.Surfaces.textSecondary)
            if viewModel.canSkip {
                Button("Skip commit") { viewModel.skip() }
            }
            Button(viewModel.stageTitle) { viewModel.stageResolved() }
                .disabled(!viewModel.canStage)
            Button(viewModel.continueTitle) { viewModel.continueOperation() }
                .keyboardShortcut(.return, modifiers: [.shift, .command])
                .buttonStyle(.borderedProminent)
                .disabled(!viewModel.canContinue)
                .help(viewModel.isRewording ? "With this message (⇧⌘↩)" : "Once every file is staged (⇧⌘↩)")
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 10)
        .background(Theme.Surfaces.chrome)
    }
}

/// The message of the commit a `reword` line stopped on, written before
/// continuing.
struct RewordPanel: View {
    let title: String
    let help: String
    @Binding var message: String
    /// The message differs from the commit's own.
    let edited: Bool
    let reset: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack(alignment: .firstTextBaseline, spacing: 8) {
                Label(title, systemImage: "text.cursor")
                    .font(Metrics.captionBold)
                Text(help)
                    .font(Metrics.caption)
                    .foregroundStyle(Theme.Surfaces.textSecondary)
                Spacer()
                Button("Revert to Original", action: reset)
                    .disabled(!edited)
                    .help("The commit's own message")
            }
            TextEditor(text: $message)
                .font(Metrics.code)
                .scrollContentBackground(.hidden)
                .padding(6)
                .background(
                    RoundedRectangle(cornerRadius: 6)
                        .fill(Theme.Surfaces.window)
                )
                .overlay(
                    RoundedRectangle(cornerRadius: 6)
                        .strokeBorder(Theme.Surfaces.textSecondary.opacity(0.3))
                )
                .accessibilityLabel("Commit message")
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 10)
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)
        .background(Theme.Surfaces.chrome)
    }
}

/// The operation, its commits and how many files are ready.
struct OverviewHeader: View {
    let summary: String
    /// What git ran on its own since it last stopped.
    let applied: String?
    /// "2 of 6 files ready"; `nil` without an operation.
    let progress: String?
    let detail: String
    let segments: [(state: ConflictState, count: Int)]

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text(summary)
                .font(Metrics.headline)
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
                    return step
                }
            ),
            merge: nil,
            applied: operation == nil ? [] : applied,
            reword: operation == nil ? nil : reword,
            files: files,
            canContinue: operation != nil && files.allSatisfy { $0.state == .staged },
            canSkip: operation != nil
        )
    }

    func apply(action: OverviewAction) throws {}
    func deletion(path: String) throws -> DeletionInfo { try SpecialSamples().deletion(path: path) }
    func binary(path: String) throws -> BinaryInfo { try SpecialSamples().binary(path: path) }
    func project(path: String) throws -> ProjectInfo { try SpecialSamples().project(path: path) }
    func catalog(path: String) throws -> CatalogInfo { try SpecialSamples().catalog(path: path) }
    func packages(path: String) throws -> PackagesInfo { try SpecialSamples().packages(path: path) }
    func lockfile(path: String) throws -> LockfileInfo { try SpecialSamples().lockfile(path: path) }

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
        RebaseStep(number: number, action: action, shortId: id, subject: subject, state: .pending, joins: joins)
    }

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
            reword: RewordInfo(message: "Explain the sync queue in the README", committed: true)
        )
    ) { _ in })
}

#Preview("Overview, reword with conflicts", traits: .fixedLayout(width: 1300, height: 700)) {
    OverviewView(viewModel: OverviewViewModel(
        source: PreviewOverview(
            files: Array(PreviewOverview.mixed.prefix(2)),
            current: 2,
            applied: [],
            reword: RewordInfo(message: "Explain the sync queue in the README", committed: false)
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
