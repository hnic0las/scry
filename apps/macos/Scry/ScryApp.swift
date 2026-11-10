import AppKit
import SwiftUI

@main
struct ScryApp: App {
    var body: some Scene {
        WindowGroup {
            RootView()
        }
        .windowToolbarStyle(.unified)
    }
}

/// Opens the file given on the command line, or asks for one.
struct RootView: View {
    @State private var viewModel = RootViewModel()

    var body: some View {
        Group {
            if let editor = viewModel.editor {
                MergeEditorView(viewModel: editor)
            } else {
                EmptyStateView(error: viewModel.openError, open: chooseFile)
            }
        }
        .frame(minWidth: 900, minHeight: 600)
        .task { viewModel.openLaunchFile() }
    }

    private func chooseFile() {
        let panel = NSOpenPanel()
        panel.canChooseFiles = true
        panel.canChooseDirectories = false
        panel.allowsMultipleSelection = false
        panel.message = "Choose a file with conflict markers"
        if panel.runModal() == .OK, let url = panel.url {
            viewModel.open(url.path)
        }
    }
}

struct EmptyStateView: View {
    let error: String?
    let open: () -> Void

    var body: some View {
        VStack(spacing: 12) {
            Image(systemName: "arrow.triangle.merge")
                .font(.system(size: 40))
                .foregroundStyle(Theme.Surfaces.textTertiary)
            Text("No file open")
                .font(.title3.weight(.semibold))
            Text("Open a file with conflict markers, or run Scry from git mergetool.")
                .foregroundStyle(Theme.Surfaces.textSecondary)
            Button("Open Conflicted File…", action: open)
                .keyboardShortcut("o")
            if let error {
                Text(error)
                    .foregroundStyle(Theme.Severity.error)
                    .font(Metrics.callout)
            }
        }
        .padding(40)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(Theme.Surfaces.window)
    }
}

#Preview("Empty state", traits: .fixedLayout(width: 900, height: 600)) {
    EmptyStateView(error: nil, open: {})
}

#Preview("Empty state, file not opened", traits: .fixedLayout(width: 900, height: 600)) {
    EmptyStateView(error: "Uploader.swift has no conflict markers.", open: {})
}
