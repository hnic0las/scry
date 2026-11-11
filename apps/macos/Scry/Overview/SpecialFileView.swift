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
            ScrollView {
                content
                    .padding(16)
                    .frame(maxWidth: .infinity, alignment: .leading)
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
        case .binary:
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
            VStack(alignment: .leading, spacing: 10) {
                if !viewModel.catalogRows.isEmpty {
                    HStack {
                        Text("Take for every value:")
                            .font(Metrics.callout)
                            .foregroundStyle(Theme.Surfaces.textSecondary)
                        Button("Yours") { viewModel.chooseAll(.ours) }
                        Button("Theirs") { viewModel.chooseAll(.theirs) }
                    }
                    .controlSize(.small)
                }
                ForEach(viewModel.catalogRows) { row in
                    CatalogConflictRow(
                        row: row,
                        ours: viewModel.oursLabel,
                        theirs: viewModel.theirsLabel
                    ) { version in viewModel.choose(version, for: row.id) }
                }
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
            }
            Spacer()
            if viewModel.canOpenInEditor {
                Button("Open in Merge Editor") { viewModel.openEditor() }
            }
            Button("Cancel", role: .cancel) { close() }
                .keyboardShortcut(.cancelAction)
            switch viewModel.kind {
            case .projectFile:
                Button(viewModel.keepBothTitle) { viewModel.keepBoth() }
                    .buttonStyle(.borderedProminent)
                    .keyboardShortcut(.defaultAction)
                    .disabled(!viewModel.canKeepBoth)
            case .stringCatalog:
                Button("Write Catalog") { viewModel.writeCatalog() }
                    .buttonStyle(.borderedProminent)
                    .keyboardShortcut(.defaultAction)
                    .disabled(!viewModel.canWriteCatalog)
            case .deletedOnOneSide, .binary:
                EmptyView()
            }
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 10)
        .background(Theme.Surfaces.chrome)
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
        }
    }

    /// What the button does.
    var help: String {
        switch self {
        case .deletedOnOneSide: "Keep or delete it"
        case .binary: "Take one version"
        case .projectFile: "Keep both sides' additions"
        case .stringCatalog: "Merge it key by key"
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

/// Sample answers for previews, without a repository.
struct SpecialSamples: OverviewSource {
    func snapshot() throws -> OverviewSnapshot {
        throw CancellationError()
    }

    func apply(action: OverviewAction) throws {}

    func deletion(path: String) throws -> DeletionInfo {
        DeletionInfo(
            deletedBy: .ours, deletedLabel: "feature/sync-v2 · current branch", keptLabel: "main · being merged", keptLines: 18,
            changes: "@@ -4,7 +4,9 @@\n \n - (nullable NSData *)dataForKey:(NSString *)key {\n-    return [self.storage objectForKey:key];\n+    @synchronized (self) {\n+        return [self.storage objectForKey:key];\n+    }\n }\n"
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

    static func file(_ path: String, _ kind: SpecialKind) -> OverviewFile {
        OverviewFile(
            path: path,
            absolutePath: "/Users/you/Code/atlas/" + path,
            name: String(path.split(separator: "/").last ?? ""),
            directory: path.split(separator: "/").dropLast().joined(separator: "/"),
            kind: "",
            state: .unresolved,
            conflicts: nil,
            openable: kind == .projectFile || kind == .stringCatalog,
            special: kind
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

#Preview("Xcode project") {
    SpecialFileView(viewModel: SpecialSamples.viewModel("ios/Atlas.xcodeproj/project.pbxproj", .projectFile)) {}
        .preferredColorScheme(.dark)
}

#Preview("String catalog") {
    let viewModel = SpecialSamples.viewModel("ios/Resources/Localizable.xcstrings", .stringCatalog)
    viewModel.choose(.theirs, for: 0)
    return SpecialFileView(viewModel: viewModel) {}
}
