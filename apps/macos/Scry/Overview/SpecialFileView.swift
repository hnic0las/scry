import AppKit
import SwiftUI

/// S4: the sheet that settles a special file from the overview.
struct SpecialFileView: View {
    @Bindable var viewModel: SpecialFileViewModel
    let close: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            SpecialFileHeader(
                symbol: viewModel.kind.symbol,
                title: viewModel.title,
                subtitle: viewModel.subtitle,
                explanation: viewModel.explanation
            )
            Divider()
            if viewModel.isLoading {
                ProgressView()
                    .controlSize(.small)
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else {
                ScrollView {
                    content
                        .padding(16)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .disabled(viewModel.isRunning)
                }
            }
            Divider()
            footer
        }
        .frame(width: 720, height: 520)
        .background(Theme.Surfaces.window)
    }

    @ViewBuilder
    private var content: some View {
        switch viewModel.kind {
        case .deletedOnOneSide:
            VStack(alignment: .leading, spacing: 14) {
                HStack(spacing: 12) {
                    ChoiceCard(
                        symbol: "doc.badge.plus",
                        title: "Keep the file",
                        detail: "As the side that changed it left it.",
                        color: Theme.States.resolved
                    ) { viewModel.keepFile() }
                    ChoiceCard(
                        symbol: "trash",
                        title: "Delete the file",
                        detail: "As the side that deleted it wanted.",
                        color: Theme.Severity.error
                    ) { viewModel.deleteFile() }
                }
                if !viewModel.changeLines.isEmpty {
                    ChangesList(title: viewModel.changesTitle, lines: viewModel.changeLines)
                }
            }
        case .binary, .notMerged:
            HStack(alignment: .top, spacing: 12) {
                ForEach(viewModel.versionCards) { card in
                    BinaryVersionCard(card: card) { viewModel.take(card.version) }
                }
            }
        case .projectFile:
            VStack(alignment: .leading, spacing: 10) {
                if let warning = viewModel.projectWarning {
                    VStack(alignment: .leading, spacing: 6) {
                        Label(warning, systemImage: "exclamationmark.triangle.fill")
                            .font(Metrics.callout)
                            .foregroundStyle(Theme.Severity.warning)
                            .fixedSize(horizontal: false, vertical: true)
                        if viewModel.canRemoveMissing {
                            Toggle("Remove those references too", isOn: $viewModel.removeMissing)
                                .font(Metrics.callout)
                        }
                    }
                    .padding(10)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .background(RoundedRectangle(cornerRadius: Metrics.cardCornerRadius).fill(Theme.Severity.warning.opacity(0.10)))
                }
                ForEach(viewModel.projectRows) { row in
                    ProjectConflictRow(row: row, ours: viewModel.oursLabel, theirs: viewModel.theirsLabel)
                }
            }
        case .stringCatalog:
            choices(viewModel.catalogRows, every: "value")
        case .packageResolved:
            VStack(alignment: .leading, spacing: 12) {
                if let note = viewModel.originHashNote {
                    NoteBanner(text: note, symbol: "info.circle.fill", color: Theme.Interaction.accent)
                }
                if !viewModel.pinRows.isEmpty {
                    PinChangesList(rows: viewModel.pinRows, ours: viewModel.oursLabel, theirs: viewModel.theirsLabel)
                }
                choices(viewModel.packageRows, every: "package")
            }
        case .lockfile:
            VStack(alignment: .leading, spacing: 12) {
                if let warning = viewModel.manifestWarning {
                    NoteBanner(text: warning, symbol: "exclamationmark.triangle.fill", color: Theme.Severity.warning)
                }
                HStack(spacing: 12) {
                    ChoiceCard(
                        symbol: viewModel.taken == .ours ? "checkmark.circle.fill" : "arrow.down.doc",
                        title: "Take Yours",
                        detail: viewModel.oursLabel,
                        color: Theme.Sides.ours
                    ) { viewModel.takeLockfile(.ours) }
                    ChoiceCard(
                        symbol: viewModel.taken == .theirs ? "checkmark.circle.fill" : "arrow.down.doc",
                        title: "Take Theirs",
                        detail: viewModel.theirsLabel,
                        color: Theme.Sides.theirs
                    ) { viewModel.takeLockfile(.theirs) }
                }
                CommandBox(command: viewModel.command, highlighted: viewModel.taken != nil)
                if let caveat = viewModel.commandCaveat {
                    NoteBanner(text: caveat, symbol: "exclamationmark.triangle.fill", color: Theme.Severity.warning)
                }
                if !viewModel.lockfileRows.isEmpty {
                    PackageTable(rows: viewModel.lockfileRows, ours: viewModel.oursLabel, theirs: viewModel.theirsLabel)
                }
            }
        case .unusual:
            if let note = viewModel.folderNote {
                NoteBanner(text: note, symbol: "folder", color: Theme.Interaction.accent)
            }
        }
    }

    /// Values both sides changed, one pick each, with a pick for all.
    private func choices(_ rows: [SpecialFileViewModel.CatalogRow], every noun: String) -> some View {
        VStack(alignment: .leading, spacing: 10) {
            if !rows.isEmpty {
                HStack {
                    Text("Take for every \(noun):")
                        .font(Metrics.callout)
                        .foregroundStyle(Theme.Surfaces.textSecondary)
                    Button("Yours") { viewModel.chooseAll(.ours) }
                    Button("Theirs") { viewModel.chooseAll(.theirs) }
                }
                .controlSize(.small)
            }
            ForEach(rows) { row in
                CatalogConflictRow(
                    row: row,
                    ours: viewModel.oursLabel,
                    theirs: viewModel.theirsLabel
                ) { version in viewModel.choose(version, for: row.id) }
            }
        }
    }

    private var footer: some View {
        HStack(spacing: 10) {
            if let error = viewModel.error {
                Text(error)
                    .font(Metrics.callout)
                    .foregroundStyle(Theme.Severity.error)
                    .lineLimit(2)
            } else if let note = viewModel.actionNote {
                Text(note)
                    .font(Metrics.callout)
                    .foregroundStyle(Theme.Surfaces.textSecondary)
                    .lineLimit(2)
            }
            Spacer()
            if viewModel.isRunning {
                ProgressView()
                    .controlSize(.small)
            }
            if viewModel.canOpenInEditor {
                Button("Open in Merge Editor") { viewModel.openEditor() }
                    .disabled(viewModel.isRunning)
            }
            Button("Cancel", role: .cancel) { close() }
                .keyboardShortcut(Shortcut.cancelSheet)
            actions
                .disabled(viewModel.isRunning || viewModel.isLoading)
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 10)
        .background(Theme.Surfaces.chrome)
    }

    /// The footer's buttons for this kind of file.
    @ViewBuilder
    private var actions: some View {
        switch viewModel.kind {
        case .projectFile:
            Button(viewModel.keepBothTitle) { viewModel.keepBoth() }
                .buttonStyle(.borderedProminent)
                .keyboardShortcut(Shortcut.confirmSheet)
                .disabled(!viewModel.canKeepBoth)
        case .stringCatalog, .packageResolved:
            Button(viewModel.writeTitle) { viewModel.writeChoices() }
                .buttonStyle(.borderedProminent)
                .keyboardShortcut(Shortcut.confirmSheet)
                .disabled(!viewModel.canWriteChoices)
        case .lockfile:
            if viewModel.canMergeLines && viewModel.taken == nil {
                Button("Keep Both Sides' Lines") { viewModel.mergeLines() }
            }
            if viewModel.taken != nil {
                Button("Done") { viewModel.finish() }
                    .buttonStyle(.borderedProminent)
                    .keyboardShortcut(Shortcut.confirmSheet)
            }
        case .unusual:
            Button("Mark Resolved") { viewModel.markResolved() }
                .buttonStyle(.borderedProminent)
                .keyboardShortcut(Shortcut.confirmSheet)
                .disabled(!viewModel.canMarkResolved)
        case .deletedOnOneSide, .binary, .notMerged:
            EmptyView()
        }
    }
}

extension SpecialKind {
    /// The glyph of the button that opens the view, and of its header.
    var symbol: String {
        switch self {
        case .deletedOnOneSide: "trash.slash"
        case .binary: "photo.on.rectangle"
        case .projectFile: "hammer"
        case .stringCatalog: "character.bubble"
        case .packageResolved: "shippingbox"
        case .lockfile: "lock.doc"
        case .notMerged: "doc.on.doc"
        case .unusual: "questionmark.folder"
        }
    }

    /// What the button does.
    var help: String {
        switch self {
        case .deletedOnOneSide: "Keep or delete it"
        case .binary: "Take one version"
        case .projectFile: "Keep both sides' additions"
        case .stringCatalog: "Merge it key by key"
        case .packageResolved: "Merge it package by package"
        case .lockfile: "Take a side, then write it again"
        case .notMerged: "Take one version"
        case .unusual: "Settle it in the folder, then mark it resolved"
        }
    }
}

/// The file, what it is and what to decide.
struct SpecialFileHeader: View {
    let symbol: String
    let title: String
    let subtitle: String
    let explanation: String

    var body: some View {
        HStack(alignment: .top, spacing: 12) {
            Image(systemName: symbol)
                .font(.system(size: 22))
                .foregroundStyle(Theme.Interaction.accent)
                .frame(width: 32)
            VStack(alignment: .leading, spacing: 4) {
                Text(title).font(Metrics.headline)
                Text(subtitle)
                    .font(Metrics.caption)
                    .foregroundStyle(Theme.Surfaces.textSecondary)
                Text(explanation)
                    .font(Metrics.callout)
                    .fixedSize(horizontal: false, vertical: true)
                    .padding(.top, 2)
            }
            Spacer(minLength: 0)
        }
        .padding(16)
        .background(Theme.Surfaces.chrome)
    }
}

/// A large button for one of two choices.
struct ChoiceCard: View {
    let symbol: String
    let title: String
    let detail: String
    let color: Color
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            VStack(spacing: 8) {
                Image(systemName: symbol)
                    .font(.system(size: 28))
                    .foregroundStyle(color)
                Text(title).font(Metrics.headline)
                Text(detail)
                    .font(Metrics.caption)
                    .foregroundStyle(Theme.Surfaces.textSecondary)
                    .multilineTextAlignment(.center)
            }
            .frame(maxWidth: .infinity, minHeight: 140)
            .background(
                RoundedRectangle(cornerRadius: Metrics.cardCornerRadius)
                    .fill(Theme.Surfaces.pane)
            )
            .overlay(
                RoundedRectangle(cornerRadius: Metrics.cardCornerRadius)
                    .strokeBorder(color.opacity(0.4))
            )
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
    }
}

/// The changes a deletion would throw away, as a small diff.
struct ChangesList: View {
    let title: String
    let lines: [SpecialFileViewModel.DiffLine]

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text(title)
                .font(Metrics.captionBold)
                .foregroundStyle(Theme.Surfaces.textSecondary)
            VStack(alignment: .leading, spacing: 0) {
                ForEach(lines) { line in
                    HStack(spacing: 6) {
                        Text(mark(line.kind))
                            .frame(width: 10)
                            .foregroundStyle(color(line.kind))
                        Text(line.text)
                            .foregroundStyle(line.kind == .hunk ? Theme.Surfaces.textTertiary : Theme.Surfaces.text)
                        Spacer(minLength: 0)
                    }
                    .font(Metrics.code)
                    .padding(.horizontal, 8)
                    .padding(.vertical, 1)
                    .background(background(line.kind))
                }
            }
            .padding(.vertical, 4)
            .background(RoundedRectangle(cornerRadius: 6).fill(Theme.Surfaces.pane))
            .clipShape(RoundedRectangle(cornerRadius: 6))
        }
    }

    private func mark(_ kind: SpecialFileViewModel.DiffLine.Kind) -> String {
        switch kind {
        case .added: "+"
        case .removed: "−"
        case .hunk, .context: ""
        }
    }

    private func color(_ kind: SpecialFileViewModel.DiffLine.Kind) -> Color {
        switch kind {
        case .added: Theme.Diff.addedMark
        case .removed: Theme.Diff.removedMark
        case .hunk, .context: Theme.Surfaces.textTertiary
        }
    }

    private func background(_ kind: SpecialFileViewModel.DiffLine.Kind) -> Color {
        switch kind {
        case .added: Theme.Diff.added
        case .removed: Theme.Diff.removed
        case .hunk, .context: Color.clear
        }
    }
}

/// One version of a binary file: an image preview when it is one, its
/// size, and the button that takes it.
struct BinaryVersionCard: View {
    let card: SpecialFileViewModel.VersionCard
    let take: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(card.side)
                .font(Metrics.captionBold)
                .foregroundStyle(card.version == .ours ? Theme.Sides.ours : Theme.Sides.theirs)
            Text(card.label)
                .font(Metrics.callout)
                .lineLimit(2)
            ZStack {
                RoundedRectangle(cornerRadius: 6).fill(Theme.Surfaces.pane)
                if let data = card.data, let image = NSImage(data: data) {
                    Image(nsImage: image)
                        .resizable()
                        .interpolation(.high)
                        .scaledToFit()
                        .padding(8)
                } else {
                    Image(systemName: card.available ? "doc" : "nosign")
                        .font(.system(size: 28))
                        .foregroundStyle(Theme.Surfaces.textTertiary)
                }
            }
            .frame(height: 200)
            Text(card.size)
                .font(Metrics.caption)
                .foregroundStyle(Theme.Surfaces.textSecondary)
            Button("Use \(card.side)") { take() }
                .disabled(!card.available)
                .frame(maxWidth: .infinity, alignment: .trailing)
        }
        .padding(12)
        .frame(maxWidth: .infinity)
        .background(RoundedRectangle(cornerRadius: Metrics.cardCornerRadius).fill(Theme.Surfaces.chrome))
    }
}

/// A conflict of a project file: what each side adds, removes and
/// changes, and whether keeping both is safe.
struct ProjectConflictRow: View {
    let row: SpecialFileViewModel.ProjectRow
    let ours: String
    let theirs: String

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack(alignment: .firstTextBaseline) {
                Text("Conflict \(row.id + 1)").font(Metrics.captionBold)
                Spacer()
                Label(
                    row.keepsBoth ? "Safe to keep both" : "For the merge editor",
                    systemImage: row.keepsBoth ? "checkmark.circle.fill" : "pencil.circle.fill"
                )
                .font(Metrics.caption)
                .foregroundStyle(row.keepsBoth ? Theme.States.resolved : Theme.Severity.warning)
            }
            if let reason = row.reason {
                Text(reason)
                    .font(Metrics.caption)
                    .foregroundStyle(Theme.Severity.warning)
            }
            HStack(alignment: .top, spacing: 12) {
                side(ours, row.oursAdds, row.oursRemoves, row.oursChanges, Theme.Sides.ours)
                side(theirs, row.theirsAdds, row.theirsRemoves, row.theirsChanges, Theme.Sides.theirs)
            }
        }
        .padding(10)
        .background(RoundedRectangle(cornerRadius: Metrics.cardCornerRadius).fill(Theme.Surfaces.chrome))
    }

    private func side(_ name: String, _ adds: [String], _ removes: [String], _ changes: [String], _ color: Color) -> some View {
        VStack(alignment: .leading, spacing: 3) {
            Text(name)
                .font(Metrics.caption)
                .foregroundStyle(color)
                .lineLimit(1)
            if adds.isEmpty && removes.isEmpty && changes.isEmpty {
                Text("No change")
                    .font(Metrics.callout)
                    .foregroundStyle(Theme.Surfaces.textTertiary)
            }
            ForEach(adds, id: \.self) { entry in
                Label(entry, systemImage: "plus").font(Metrics.code)
            }
            ForEach(removes, id: \.self) { entry in
                Label(entry, systemImage: "minus")
                    .font(Metrics.code)
                    .strikethrough()
                    .foregroundStyle(Theme.Surfaces.textSecondary)
            }
            ForEach(changes, id: \.self) { entry in
                Label(entry, systemImage: "arrow.right").font(Metrics.code)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}

/// A catalog value both sides changed: the base, each side, and the pick.
struct CatalogConflictRow: View {
    let row: SpecialFileViewModel.CatalogRow
    let ours: String
    let theirs: String
    let choose: (FileVersion) -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack(spacing: 6) {
                Text(row.key).font(Metrics.code.weight(.semibold))
                if !row.detail.isEmpty {
                    Text(row.detail)
                        .font(Metrics.caption)
                        .foregroundStyle(Theme.Surfaces.textSecondary)
                }
                Spacer()
                Text("Base: \(row.base)")
                    .font(Metrics.caption)
                    .foregroundStyle(Theme.Surfaces.textTertiary)
                    .lineLimit(1)
            }
            HStack(spacing: 8) {
                option(.ours, ours, row.ours, Theme.Sides.ours)
                option(.theirs, theirs, row.theirs, Theme.Sides.theirs)
            }
        }
        .padding(10)
        .background(RoundedRectangle(cornerRadius: Metrics.cardCornerRadius).fill(Theme.Surfaces.chrome))
    }

    private func option(_ version: FileVersion, _ side: String, _ value: String, _ color: Color) -> some View {
        let picked = row.pick == version
        return Button { choose(version) } label: {
            VStack(alignment: .leading, spacing: 2) {
                Text(side)
                    .font(Metrics.caption)
                    .foregroundStyle(color)
                    .lineLimit(1)
                Text(value)
                    .font(Metrics.callout)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
            .padding(8)
            .background(
                RoundedRectangle(cornerRadius: 6)
                    .fill(picked ? color.opacity(0.16) : Theme.Surfaces.pane)
            )
            .overlay(
                RoundedRectangle(cornerRadius: 6)
                    .strokeBorder(picked ? color : Theme.Surfaces.hairline, lineWidth: picked ? 2 : 1)
            )
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityAddTraits(picked ? .isSelected : [])
    }
}

/// A note above a sheet's content.
struct NoteBanner: View {
    let text: String
    let symbol: String
    let color: Color

    var body: some View {
        Label(text, systemImage: symbol)
            .font(Metrics.callout)
            .foregroundStyle(color)
            .fixedSize(horizontal: false, vertical: true)
            .padding(10)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(RoundedRectangle(cornerRadius: Metrics.cardCornerRadius).fill(color.opacity(0.10)))
    }
}

/// The command that writes a lockfile again, to copy into a terminal.
struct CommandBox: View {
    let command: String
    /// The side is taken: the command is the next step.
    let highlighted: Bool

    var body: some View {
        HStack(spacing: 8) {
            Image(systemName: "terminal")
                .foregroundStyle(Theme.Surfaces.textSecondary)
            Text(command)
                .font(Metrics.code)
                .textSelection(.enabled)
            Spacer()
            Button {
                NSPasteboard.general.clearContents()
                NSPasteboard.general.setString(command, forType: .string)
            } label: {
                Label("Copy", systemImage: "doc.on.doc")
            }
            .controlSize(.small)
        }
        .padding(10)
        .background(RoundedRectangle(cornerRadius: Metrics.cardCornerRadius).fill(Theme.Surfaces.pane))
        .overlay(
            RoundedRectangle(cornerRadius: Metrics.cardCornerRadius)
                .strokeBorder(highlighted ? Theme.Interaction.accent : Theme.Surfaces.hairline, lineWidth: highlighted ? 2 : 1)
        )
    }
}

/// The packages each side changed in a lockfile, by version.
struct PackageTable: View {
    let rows: [SpecialFileViewModel.PackageRow]
    let ours: String
    let theirs: String

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text("What each side changed")
                .font(Metrics.captionBold)
                .foregroundStyle(Theme.Surfaces.textSecondary)
            Grid(alignment: .leading, horizontalSpacing: 14, verticalSpacing: 4) {
                GridRow {
                    Text("Package")
                    Text("Base")
                    Text(ours).foregroundStyle(Theme.Sides.ours).lineLimit(1)
                    Text(theirs).foregroundStyle(Theme.Sides.theirs).lineLimit(1)
                }
                .font(Metrics.caption)
                .foregroundStyle(Theme.Surfaces.textSecondary)
                Divider()
                ForEach(rows) { row in
                    GridRow {
                        HStack(spacing: 4) {
                            Text(row.name)
                            if row.bothChanged {
                                Image(systemName: "exclamationmark.triangle.fill")
                                    .foregroundStyle(Theme.Severity.warning)
                                    .help("Both sides moved it: the side you take keeps its version, unless the merged manifest rules it out")
                            }
                        }
                        Text(row.base).foregroundStyle(Theme.Surfaces.textTertiary)
                        version(row.ours, changed: row.ours != row.base, color: Theme.Sides.ours)
                        version(row.theirs, changed: row.theirs != row.base, color: Theme.Sides.theirs)
                    }
                    .font(Metrics.code)
                }
            }
            .padding(10)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(RoundedRectangle(cornerRadius: Metrics.cardCornerRadius).fill(Theme.Surfaces.chrome))
        }
    }

    private func version(_ text: String, changed: Bool, color: Color) -> some View {
        Text(text)
            .fontWeight(changed ? .semibold : .regular)
            .foregroundStyle(changed ? color : Theme.Surfaces.textTertiary)
    }
}

/// The Swift packages one side alone moved, merged by themselves.
struct PinChangesList: View {
    let rows: [SpecialFileViewModel.PinRow]
    let ours: String
    let theirs: String

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text("Merged by themselves")
                .font(Metrics.captionBold)
                .foregroundStyle(Theme.Surfaces.textSecondary)
            VStack(alignment: .leading, spacing: 4) {
                ForEach(rows) { row in
                    HStack(spacing: 8) {
                        Image(systemName: "checkmark.circle.fill")
                            .foregroundStyle(Theme.States.resolved)
                        Text(row.identity).font(Metrics.code.weight(.semibold))
                        Text(row.change).font(Metrics.code)
                        Spacer()
                        Text(row.side == .ours ? ours : theirs)
                            .font(Metrics.caption)
                            .foregroundStyle(row.side == .ours ? Theme.Sides.ours : Theme.Sides.theirs)
                            .lineLimit(1)
                    }
                }
            }
            .padding(10)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(RoundedRectangle(cornerRadius: Metrics.cardCornerRadius).fill(Theme.Surfaces.chrome))
        }
    }
}

/// Sample answers for previews, without a repository.
struct SpecialSamples: OverviewSource {
    func snapshot() throws -> OverviewSnapshot {
        throw CancellationError()
    }

    func apply(action: OverviewAction) throws -> OverviewOutcome { OverviewOutcome(events: [], snapshot: try snapshot()) }

    func deletion(path: String) throws -> DeletionInfo {
        DeletionInfo(
            deletedBy: .ours, deletedLabel: "feature/sync-v2 · current branch", keptLabel: "main · being merged", keptLines: 18,
            changes: [
                CommitDiffLine(kind: .hunk, oldNumber: nil, newNumber: nil, text: "@implementation OldCache"),
                CommitDiffLine(kind: .context, oldNumber: 4, newNumber: 4, text: ""),
                CommitDiffLine(kind: .context, oldNumber: 5, newNumber: 5, text: "- (nullable NSData *)dataForKey:(NSString *)key {"),
                CommitDiffLine(kind: .removed, oldNumber: 6, newNumber: nil, text: "    return [self.storage objectForKey:key];"),
                CommitDiffLine(kind: .added, oldNumber: nil, newNumber: 6, text: "    @synchronized (self) {"),
                CommitDiffLine(kind: .added, oldNumber: nil, newNumber: 7, text: "        return [self.storage objectForKey:key];"),
                CommitDiffLine(kind: .added, oldNumber: nil, newNumber: 8, text: "    }"),
                CommitDiffLine(kind: .context, oldNumber: 7, newNumber: 9, text: "}"),
            ]
        )
    }

    func binary(path: String) throws -> BinaryInfo {
        let image = { (symbol: String) -> Data? in
            guard let image = NSImage(systemSymbolName: symbol, accessibilityDescription: nil)?
                .withSymbolConfiguration(.init(pointSize: 120, weight: .regular).applying(.init(paletteColors: [.systemOrange, .systemBlue]))),
                let tiff = image.tiffRepresentation
            else { return nil }
            return tiff
        }
        return BinaryInfo(
            ours: BinaryVersionInfo(label: "feature/sync-v2 · current branch", size: 48_213, data: image("cloud.sun")),
            theirs: BinaryVersionInfo(label: "main · being merged", size: 51_870, data: image("cloud.bolt.rain"))
        )
    }

    func project(path: String) throws -> ProjectInfo {
        let entries = { (suffix: String) in
            ProjectConflictInfo(
                oursAdds: ["OfflineQueue.swift" + suffix], theirsAdds: ["ForecastCache.swift" + suffix],
                oursRemoves: [], theirsRemoves: ["ForecastView.swift" + suffix],
                oursChanges: [], theirsChanges: [], keepsBoth: true, reason: nil
            )
        }
        return ProjectInfo(
            oursLabel: "feature/sync-v2 · current branch",
            theirsLabel: "main · being merged",
            conflicts: [
                entries(" in Sources"), entries(""), entries(""),
                ProjectConflictInfo(
                    oursAdds: [], theirsAdds: [], oursRemoves: [], theirsRemoves: [],
                    oursChanges: ["CURRENT_PROJECT_VERSION 41 → 42", "MARKETING_VERSION 1.3 → 1.4"],
                    theirsChanges: ["CURRENT_PROJECT_VERSION 41 → 50", "IPHONEOS_DEPLOYMENT_TARGET 16.0 → 16.4", "MARKETING_VERSION 1.3 → 2.0"],
                    keepsBoth: false, reason: "CURRENT_PROJECT_VERSION: 42 on one side, 50 on the other"
                ),
            ],
            resolvable: 3,
            missing: ["A2000002 ForecastView.swift"],
            missingRemovable: true
        )
    }

    func catalog(path: String) throws -> CatalogInfo {
        CatalogInfo(oursLabel: "feature/sync-v2 · current branch", theirsLabel: "main · being merged", automatic: 4, conflicts: [
            CatalogConflictInfo(key: "sync.done", detail: "fr › value", base: "Synchronisé", ours: "Tout est synchronisé", theirs: "Synchronisation terminée"),
            CatalogConflictInfo(key: "sync.pending", detail: "fr › value", base: "%lld modifications en attente", ours: "%lld changements à envoyer", theirs: "%lld modifications à synchroniser"),
        ])
    }

    func packages(path: String) throws -> PackagesInfo {
        PackagesInfo(
            oursLabel: "feature/sync-v2 · current branch", theirsLabel: "main · being merged",
            automatic: [
                PinChangeInfo(identity: "kingfisher", base: "7.9.0", ours: "7.9.0", theirs: "7.10.0", movedByOurs: false),
                PinChangeInfo(identity: "lottie-ios", base: nil, ours: "4.4.1", theirs: nil, movedByOurs: true),
                PinChangeInfo(identity: "swift-collections", base: "1.0.6", ours: "1.0.6", theirs: nil, movedByOurs: false),
            ],
            conflicts: [
                PinChangeInfo(identity: "alamofire", base: "5.8.0", ours: "5.8.1", theirs: "5.9.0", movedByOurs: true),
                PinChangeInfo(identity: "swift-snapshot-testing", base: "1.15.0", ours: "main @ 3f2a9c1", theirs: "1.16.0", movedByOurs: true),
            ],
            originHashStale: true
        )
    }

    func unusual(path: String) throws -> UnusualInfo {
        UnusualInfo(
            what: "Both sides renamed this file, each to its own name.",
            advice: "The new names are listed as files of their own. Mark this one resolved to drop the old name, unless you want to keep it in the folder.",
            inFolder: false
        )
    }

    func lockfile(path: String) throws -> LockfileInfo {
        LockfileInfo(
            oursLabel: "feature/sync-v2 · current branch", theirsLabel: "main · being merged",
            tool: "CocoaPods", manifest: "ios/Podfile", manifestConflicted: true, command: "cd ios && pod install",
            caveat: nil, mergesByLines: false,
            changes: [
                PackageChangeInfo(name: "Alamofire", base: "5.8.0", ours: "5.8.1", theirs: "5.9.0", bothChanged: true),
                PackageChangeInfo(name: "Firebase/Analytics", base: "10.20.0", ours: "10.20.0", theirs: "10.22.0", bothChanged: false),
                PackageChangeInfo(name: "Lottie", base: nil, ours: "4.4.1", theirs: nil, bothChanged: false),
                PackageChangeInfo(name: "SVProgressHUD", base: "2.3.1", ours: "2.3.1", theirs: nil, bothChanged: false),
            ]
        )
    }

    func commitDiff(commit: String, path: String) throws -> [CommitDiffLine] { [] }

    static func file(_ path: String, _ kind: SpecialKind) -> OverviewFile {
        OverviewFile(
            path: path,
            absolutePath: "/Users/you/Code/atlas/" + path,
            name: String(path.split(separator: "/").last ?? ""),
            directory: path.split(separator: "/").dropLast().joined(separator: "/"),
            kind: kind == .unusual ? "Renamed on both sides" : "",
            state: .unresolved,
            conflicts: nil,
            openable: ![.deletedOnOneSide, .binary, .notMerged, .unusual].contains(kind),
            special: kind,
            canOpen: true,
            canStage: false,
            canUnstage: false
        )
    }

    @MainActor
    static func viewModel(_ path: String, _ kind: SpecialKind) -> SpecialFileViewModel {
        SpecialFileViewModel(file: file(path, kind), source: SpecialSamples(), done: {}, openInEditor: { _ in })
    }
}

#Preview("Deleted on one side") {
    SpecialFileView(viewModel: SpecialSamples.viewModel("ios/Sources/Legacy/OldCache.m", .deletedOnOneSide)) {}
}

#Preview("Binary image") {
    SpecialFileView(viewModel: SpecialSamples.viewModel("ios/Resources/AppIcon.png", .binary)) {}
}

#Preview("Not merged by git") {
    SpecialFileView(viewModel: SpecialSamples.viewModel("config/flags.json", .notMerged)) {}
}

#Preview("Renamed on both sides") {
    SpecialFileView(viewModel: SpecialSamples.viewModel("ios/Sources/Sync/Engine.swift", .unusual)) {}
}

#Preview("Xcode project") {
    SpecialFileView(viewModel: SpecialSamples.viewModel("ios/Atlas.xcodeproj/project.pbxproj", .projectFile)) {}
        .preferredColorScheme(.dark)
}

#Preview("String catalog") {
    let viewModel = SpecialSamples.viewModel("ios/Resources/Localizable.xcstrings", .stringCatalog)
    viewModel.choose(.theirs, for: 0)
    return SpecialFileView(viewModel: viewModel) {}
}

#Preview("Package.resolved") {
    let viewModel = SpecialSamples.viewModel("ios/Atlas.xcworkspace/xcshareddata/swiftpm/Package.resolved", .packageResolved)
    viewModel.choose(.theirs, for: 0)
    return SpecialFileView(viewModel: viewModel) {}
}

#Preview("Lockfile") {
    SpecialFileView(viewModel: SpecialSamples.viewModel("ios/Podfile.lock", .lockfile)) {}
}

#Preview("Lockfile taken") {
    let viewModel = SpecialSamples.viewModel("ios/Podfile.lock", .lockfile)
    viewModel.takeLockfile(.theirs)
    return SpecialFileView(viewModel: viewModel) {}
        .preferredColorScheme(.dark)
}
