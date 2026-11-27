import SwiftUI
import UniformTypeIdentifiers

/// S19: what Scry shows with nothing open. Open a repository with the
/// button, by dropping its folder, or from the recent ones.
struct OpenRepositoryView: View {
    @Bindable var viewModel: OpenRepositoryViewModel
    /// Why the last repository or file could not be opened.
    let error: String?
    let chooseRepository: () -> Void
    let chooseFile: () -> Void
    /// Opens a folder dropped on the window.
    let openFolder: (String) -> Void

    @State private var dropTargeted = false

    static let width: CGFloat = 560

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            VStack(alignment: .leading, spacing: 8) {
                Text("Open a repository")
                    .font(.title2.weight(.semibold))
                Text("Scry shows the merge, rebase, cherry-pick or revert in progress, or else what is not committed. In a terminal, git scry does the same from any folder of a repository.")
                    .font(Metrics.callout)
                    .foregroundStyle(Theme.Surfaces.textSecondary)
                    .fixedSize(horizontal: false, vertical: true)
                HStack {
                    // ⌘O and ⇧⌘O are the File menu's (`OpenCommands`).
                    Button("Open Repository…", action: chooseRepository)
                    Button("Open Conflicted File…", action: chooseFile)
                }
                .padding(.top, 4)
                if let error {
                    Text(error)
                        .font(Metrics.callout)
                        .foregroundStyle(Theme.Severity.error)
                }
            }
            .padding(.bottom, 20)

            Text("Recent")
                .font(Metrics.captionBold)
                .foregroundStyle(Theme.Surfaces.textSecondary)
                .padding(.bottom, 6)
            if viewModel.rows.isEmpty {
                Text("No repository opened yet.")
                    .font(Metrics.callout)
                    .foregroundStyle(Theme.Surfaces.textTertiary)
                    .padding(.vertical, 12)
            } else {
                VStack(spacing: 0) {
                    ForEach(viewModel.rows) { row in
                        RecentRepositoryRow(row: row, selected: row.path == viewModel.selection)
                            .contentShape(Rectangle())
                            .onTapGesture(count: 2) { viewModel.open(row) }
                            .onTapGesture { viewModel.selection = row.path }
                            .contextMenu {
                                Button("Open") { viewModel.open(row) }
                                    .disabled(row.isMissing)
                                Button("Remove from Recent") { viewModel.remove(row) }
                            }
                            .accessibilityAddTraits(row.path == viewModel.selection ? .isSelected : [])
                    }
                }
                .padding(4)
                .focusable()
                .focusEffectDisabled()
                .onMoveCommand { direction in
                    switch direction {
                    case .up: viewModel.moveSelection(by: -1)
                    case .down: viewModel.moveSelection(by: 1)
                    default: break
                    }
                }
                .background {
                    // Return opens the selected repository.
                    Button("Open") { viewModel.openSelected() }
                        .keyboardShortcut(.defaultAction)
                        .disabled(!viewModel.canOpenSelected)
                        .opacity(0)
                        .accessibilityHidden(true)
                }
                .background(RoundedRectangle(cornerRadius: Metrics.cardCornerRadius).fill(Theme.Surfaces.pane))
                .overlay(
                    RoundedRectangle(cornerRadius: Metrics.cardCornerRadius).stroke(Theme.Surfaces.hairline)
                )
            }
            Spacer(minLength: 0)
        }
        .frame(width: Self.width, alignment: .leading)
        .padding(40)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(Theme.Surfaces.window)
        .overlay {
            if dropTargeted {
                RoundedRectangle(cornerRadius: 10)
                    .stroke(Theme.Interaction.focus, style: StrokeStyle(lineWidth: 2, dash: [6, 4]))
                    .padding(12)
            }
        }
        .onDrop(of: [.fileURL], isTargeted: $dropTargeted) { providers in
            guard let provider = providers.first else { return false }
            _ = provider.loadObject(ofClass: URL.self) { url, _ in
                guard let url else { return }
                Task { @MainActor in openFolder(url.path) }
            }
            return true
        }
        .task { await viewModel.check() }
    }
}

struct RecentRepositoryRow: View {
    let row: OpenRepositoryViewModel.Row
    let selected: Bool

    var body: some View {
        HStack(spacing: 10) {
            Image(systemName: row.isMissing ? "questionmark.folder" : "folder")
                .foregroundStyle(Theme.Surfaces.textSecondary)
                .frame(width: 18)
            VStack(alignment: .leading, spacing: 1) {
                Text(row.name)
                    .font(Metrics.headline)
                    .lineLimit(1)
                Text(row.folder)
                    .font(Metrics.caption)
                    .foregroundStyle(Theme.Surfaces.textSecondary)
                    .lineLimit(1)
                    .truncationMode(.head)
            }
            Spacer(minLength: 12)
            HStack(spacing: 5) {
                if row.isResolving {
                    Image(systemName: "exclamationmark.triangle.fill")
                        .foregroundStyle(Theme.States.unresolved)
                        .accessibilityHidden(true)
                }
                Text(row.status)
                    .foregroundStyle(row.isResolving ? Theme.States.unresolved : Theme.Surfaces.textSecondary)
            }
            .font(Metrics.caption)
            .lineLimit(1)
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 7)
        .background(RoundedRectangle(cornerRadius: 6).fill(selected ? Theme.Interaction.selection : Color.clear))
        .help(row.path)
    }
}

#Preview("Open Repository", traits: .fixedLayout(width: 900, height: 600)) {
    let defaults = UserDefaults.preview
    let recents = RecentRepositories(defaults: defaults)
    for path in ["/Users/me/code/old-tool", "/Users/me/code/website", "/Users/me/code/offline-sync"] {
        recents.add(path)
    }
    let viewModel = OpenRepositoryViewModel(recents: recents, waiting: { path in
        switch URL(fileURLWithPath: path).lastPathComponent {
        case "offline-sync": .resolving(headline: "Rebasing feature onto main", unresolved: 3)
        case "website": .notCommitted(files: 12)
        default: .missing
        }
    }) { _ in }
    return OpenRepositoryView(
        viewModel: viewModel, error: nil, chooseRepository: {}, chooseFile: {}, openFolder: { _ in }
    )
}

#Preview("Open Repository, nothing opened yet", traits: .fixedLayout(width: 900, height: 600)) {
    OpenRepositoryView(
        viewModel: OpenRepositoryViewModel(recents: RecentRepositories(defaults: .preview)) { _ in },
        error: "notes is not in a git repository.", chooseRepository: {}, chooseFile: {}, openFolder: { _ in }
    )
    .preferredColorScheme(.dark)
}
