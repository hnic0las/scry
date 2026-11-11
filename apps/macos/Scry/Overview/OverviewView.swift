import SwiftUI

/// S1: the operation in progress and its conflicted files.
struct OverviewView: View {
    @Bindable var viewModel: OverviewViewModel

    var body: some View {
        VStack(spacing: 0) {
            OverviewHeader(
                summary: viewModel.summary,
                applied: viewModel.appliedNote,
                progress: viewModel.hasOperation ? viewModel.progress : nil,
                detail: viewModel.progressDetail,
                segments: viewModel.progressSegments
            )
            Divider()
            HStack(spacing: 0) {
                OverviewSidebar(filters: viewModel.filters) { viewModel.select($0) }
                    .frame(width: 210)
                Divider()
                table
                if let base = viewModel.planBase {
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
        .alert(viewModel.abortTitle, isPresented: $viewModel.confirmingAbort) {
            Button("Abort", role: .destructive) { viewModel.abort() }
            Button("Cancel", role: .cancel) {}
        } message: {
            Text(viewModel.abortMessage)
        }
    }

    private var table: some View {
        Table(viewModel.rows) {
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
            TableColumn("") { row in
                if let action = row.action {
                    Button(action) { viewModel.open(row.path) }
                        .controlSize(.small)
                }
            }
            .width(70)
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
                .help("Once every file is staged (⇧⌘↩)")
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 10)
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
            applied: operation == nil ? [] : applied,
            files: files,
            canContinue: operation != nil && files.allSatisfy { $0.state == .staged },
            canSkip: operation != nil
        )
    }

    func apply(action: OverviewAction) throws {}

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
            openable: kind != "Deleted on one side"
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
