import AppKit
import SwiftUI

@main
struct ScryApp: App {
    @NSApplicationDelegateAdaptor private var delegate: ScryAppDelegate

    init() {
        // A path on the command line is the file to merge, which the window
        // opens itself (`RootViewModel.launchPath`). AppKit would take it
        // for a document to open instead, and SwiftUI then opens no window.
        UserDefaults.standard.register(defaults: ["NSTreatUnknownArgumentsAsOpen": "NO"])
    }

    /// One window per merge: a second window on the same file would be a
    /// second session, each writing over the other's choices. A
    /// `WindowGroup` without New Window rather than a `Window` scene, which
    /// does not open at launch once it was closed.
    var body: some Scene {
        WindowGroup(id: ScryApp.window) {
            RootView()
        }
        .windowToolbarStyle(.unified)
        .restorationBehavior(.disabled)
        .commands {
            CommandGroup(replacing: .newItem) {
                OpenCommands()
            }
            CommandGroup(after: .help) {
                KeyboardShortcutsCommand()
            }
        }

        Settings {
            SettingsView()
        }
    }

    static let window = "merge"
}

/// The app is the merge: it quits with its window, so `git mergetool`
/// moves on, and quitting goes through the window, which asks before
/// losing choices not written yet.
final class ScryAppDelegate: NSObject, NSApplicationDelegate {
    /// The code the app exits with, from the window's view model: what
    /// `git mergetool` reads. Until the window says otherwise, the merge
    /// tool's file is not written.
    @MainActor static var exitStatus: () -> Int32 = {
        RootViewModel.mergetoolFiles(in: CommandLine.arguments) == nil ? 0 : 1
    }

    func applicationDidFinishLaunching(_ notification: Notification) {
        // Started by `git-scry resolve` from a terminal, the app would open
        // behind it.
        NSApp.activate()
    }

    func applicationWillTerminate(_ notification: Notification) {
        let status = Self.exitStatus()
        if let path = RootViewModel.resultPath(in: CommandLine.arguments) {
            try? "\(status)\n".write(toFile: path, atomically: true, encoding: .utf8)
        }
        exit(status)
    }

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

/// What the File menu's Open items do in the window that has the focus.
struct OpenActions {
    /// No while a file is in the editor, or under `git mergetool`.
    let enabled: Bool
    let repository: () -> Void
    let file: () -> Void
}

extension FocusedValues {
    @Entry var openActions: OpenActions?
    @Entry var shortcutActions: ShortcutActions?
}

/// What Help > Keyboard Shortcuts does in the window that has the focus;
/// a struct, as SwiftUI cannot compare a bare closure.
struct ShortcutActions {
    let show: () -> Void
}

/// Help > Keyboard Shortcuts (⌘/), on the window that has the focus.
struct KeyboardShortcutsCommand: View {
    @FocusedValue(\.shortcutActions) private var actions

    var body: some View {
        Button(Shortcut.keyboardShortcuts.title) { actions?.show() }
            .keyboardShortcut(Shortcut.keyboardShortcuts)
            .disabled(actions == nil)
    }
}

/// File > Open Repository… (⌘O) and Open Conflicted File… (⇧⌘O).
struct OpenCommands: View {
    @FocusedValue(\.openActions) private var actions

    var body: some View {
        Button(Shortcut.openRepository.title) { actions?.repository() }
            .keyboardShortcut(Shortcut.openRepository)
            .disabled(actions?.enabled != true)
        Button(Shortcut.openConflictedFile.title) { actions?.file() }
            .keyboardShortcut(Shortcut.openConflictedFile)
            .disabled(actions?.enabled != true)
    }
}

/// Opens the file given on the command line, or asks for one.
struct RootView: View {
    @State private var viewModel = RootViewModel()
    /// ⌘ held alone, for the shortcut hints.
    @State private var commandHold = CommandHold()
    @Environment(\.controlActiveState) private var activeState
    @Environment(\.dismissWindow) private var dismissWindow

    var body: some View {
        Group {
            if let editor = viewModel.editor {
                MergeEditorView(viewModel: editor)
            } else if let opening = viewModel.opening {
                OpeningView(fileName: opening)
            } else if let overview = viewModel.overview {
                OverviewView(viewModel: overview)
            } else {
                EmptyStateView(error: viewModel.openError, open: chooseFile, openRepository: chooseRepository)
            }
        }
        .frame(minWidth: 900, minHeight: 600)
        // Only the window in front shows hints.
        .environment(\.showsShortcutHints, commandHold.isHeld && activeState == .key)
        .onAppear { commandHold.start() }
        .onDisappear { commandHold.stop() }
        .focusedSceneValue(\.openActions, OpenActions(
            enabled: viewModel.canOpen,
            repository: chooseRepository,
            file: chooseFile
        ))
        .focusedSceneValue(\.shortcutActions, ShortcutActions { viewModel.showShortcuts() })
        .sheet(item: Binding(get: { viewModel.shortcuts }, set: { if $0 == nil { viewModel.closeShortcuts() } })) { shortcuts in
            KeyboardShortcutsView(viewModel: shortcuts) { viewModel.closeShortcuts() }
        }
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
        .onChange(of: viewModel.finished) { _, finished in
            if finished { dismissWindow(id: ScryApp.window) }
        }
        .onChange(of: viewModel.returnsToOverview) { _, returns in
            if returns { viewModel.leaveEditor() }
        }
        .task {
            ScryAppDelegate.exitStatus = { [viewModel] in viewModel.exitStatus }
            await viewModel.openLaunchFile()
        }
    }

    private func chooseRepository() {
        let panel = NSOpenPanel()
        panel.canChooseFiles = false
        panel.canChooseDirectories = true
        panel.allowsMultipleSelection = false
        panel.message = "Choose a repository with a merge, rebase, cherry-pick or revert in progress"
        if panel.runModal() == .OK, let url = panel.url {
            Task { await viewModel.openRepository(url.path) }
        }
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
    let openRepository: () -> Void

    var body: some View {
        VStack(spacing: 12) {
            Image(systemName: "arrow.triangle.merge")
                .font(.system(size: 40))
                .foregroundStyle(Theme.Surfaces.textTertiary)
            Text("No file open")
                .font(.title3.weight(.semibold))
            Text("Open a repository with an operation in progress, a file with conflict markers, or run Scry from git mergetool.")
                .foregroundStyle(Theme.Surfaces.textSecondary)
            HStack {
                // ⌘O and ⇧⌘O are the File menu's (`OpenCommands`), which
                // work whatever the window shows.
                Button("Open Repository…", action: openRepository)
                Button("Open Conflicted File…", action: open)
            }
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
    EmptyStateView(error: nil, open: {}, openRepository: {})
}

#Preview("Empty state, file not opened", traits: .fixedLayout(width: 900, height: 600)) {
    EmptyStateView(error: "Uploader.swift has no conflict markers.", open: {}, openRepository: {})
}

#Preview("Opening a file", traits: .fixedLayout(width: 900, height: 600)) {
    OpeningView(fileName: "Localizable.xcstrings")
}
