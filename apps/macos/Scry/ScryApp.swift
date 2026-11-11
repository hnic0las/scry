import AppKit
import SwiftUI

@main
struct ScryApp: App {
    @NSApplicationDelegateAdaptor private var delegate: ScryAppDelegate

    /// One window per merge: a second window on the same file would be a
    /// second session, each writing over the other's choices.
    var body: some Scene {
        Window("Scry", id: ScryApp.window) {
            RootView()
        }
        .windowToolbarStyle(.unified)
    }

    static let window = "merge"
}

/// The app is the merge: it quits with its window, so `git mergetool`
/// moves on, and quitting goes through the window, which asks before
/// losing choices not written yet.
final class ScryAppDelegate: NSObject, NSApplicationDelegate {
    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool {
        true
    }

    func applicationShouldTerminate(_ sender: NSApplication) -> NSApplication.TerminateReply {
        guard let window = sender.windows.first(where: \.isVisible) else { return .terminateNow }
        // Closing the last window quits the app once the user agreed.
        window.performClose(nil)
        return .terminateCancel
    }
}

/// Opens the file given on the command line, or asks for one.
struct RootView: View {
    @State private var viewModel = RootViewModel()
    @Environment(\.dismissWindow) private var dismissWindow

    var body: some View {
        Group {
            if let editor = viewModel.editor {
                MergeEditorView(viewModel: editor)
            } else if let opening = viewModel.opening {
                OpeningView(fileName: opening)
            } else {
                EmptyStateView(error: viewModel.openError, open: chooseFile)
            }
        }
        .frame(minWidth: 900, minHeight: 600)
        .background { WindowCloseGuard { viewModel.shouldClose() } }
        .alert("Close without writing?", isPresented: $viewModel.confirmingClose) {
            Button("Close Without Writing", role: .destructive) {
                viewModel.closeWithoutWriting()
                dismissWindow(id: ScryApp.window)
            }
            Button("Cancel", role: .cancel) {}
        } message: {
            Text(viewModel.closeMessage)
        }
        .task { await viewModel.openLaunchFile() }
    }

    private func chooseFile() {
        let panel = NSOpenPanel()
        panel.canChooseFiles = true
        panel.canChooseDirectories = false
        panel.allowsMultipleSelection = false
        panel.message = "Choose a file with conflict markers"
        if panel.runModal() == .OK, let url = panel.url {
            Task { await viewModel.open(url.path) }
        }
    }
}

/// While a file opens: its candidates are checked and git is asked for
/// the lens commits, which takes a moment on a large file.
struct OpeningView: View {
    let fileName: String

    var body: some View {
        VStack(spacing: 12) {
            ProgressView()
                .controlSize(.large)
            Text("Opening \(fileName)…")
                .foregroundStyle(Theme.Surfaces.textSecondary)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(Theme.Surfaces.window)
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

#Preview("Opening a file", traits: .fixedLayout(width: 900, height: 600)) {
    OpeningView(fileName: "Localizable.xcstrings")
}
