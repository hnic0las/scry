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
                FileRowCommands()
            }
            CommandGroup(replacing: .undoRedo) {
                UndoCommands()
            }
            CommandGroup(after: .toolbar) {
                LayoutCommands()
            }
            // Scry has no help book: the default "Scry Help" item only
            // says help is unavailable.
            CommandGroup(replacing: .help) {
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

    /// Whether the window lets the app quit now; when it does not, it
    /// asks the user first.
    @MainActor static var shouldQuit: () -> Bool = { true }

    func applicationWillFinishLaunching(_ notification: Notification) {
        // Window ▸ Merge All Windows and the tab bar's + would give the
        // merge a second window, a second session on the same file.
        NSWindow.allowsAutomaticWindowTabbing = false
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

    /// Quits at once without unwritten choices, so a logout or restart
    /// goes on. Otherwise the merge window comes forward, even minimised
    /// or hidden, and asks; confirming closes it, which quits the app.
    func applicationShouldTerminate(_ sender: NSApplication) -> NSApplication.TerminateReply {
        if Self.shouldQuit() { return .terminateNow }
        let window = sender.windows.first { $0.identifier?.rawValue.hasPrefix(ScryApp.window) == true }
            ?? sender.windows.first { $0.canBecomeMain && $0.contentViewController != nil }
        sender.unhide(nil)
        if window?.isMiniaturized == true { window?.deminiaturize(nil) }
        window?.makeKeyAndOrderFront(nil)
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

/// What View ▸ Layout does in the merge editor that has the focus.
struct EditorLayoutActions {
    let layout: MergeEditorViewModel.EditorLayout
    /// No in a window too narrow for three columns.
    let enabled: Bool
    let set: (MergeEditorViewModel.EditorLayout) -> Void
    let toggle: () -> Void
    let showsBase: Bool
    let canShowBase: Bool
    let toggleBase: () -> Void
}

/// View ▸ Show Base (⌥⌘B), Layout and Switch Layout (⌥⌘L), for the merge
/// editor.
struct LayoutCommands: View {
    @FocusedValue(\.editorLayoutActions) private var actions
    /// The Changes window's own layouts, unified or split: the same item
    /// and the same keys switch them.
    @FocusedValue(\.changesActions) private var changes

    var body: some View {
        Button(actions?.showsBase == true ? "Hide Base" : "Show Base") { actions?.toggleBase() }
            .keyboardShortcut(Shortcut.toggleBase)
            .disabled(actions?.canShowBase != true)
        Divider()
        Picker("Layout", selection: Binding(
            get: { actions?.layout ?? .columns },
            set: { actions?.set($0) }
        )) {
            Text("Three Columns").tag(MergeEditorViewModel.EditorLayout.columns)
            Text("Stacked: Sides Above the Result").tag(MergeEditorViewModel.EditorLayout.stacked)
        }
        .disabled(actions?.enabled != true)
        Button("Switch Layout") {
            if let changes { changes.toggleLayout() } else { actions?.toggle() }
        }
        .keyboardShortcut(Shortcut.switchLayout)
        .disabled(actions?.enabled != true && changes == nil)
    }
}

/// What Edit ▸ Undo and Redo do in the merge editor that has the focus:
/// the core's history of choices.
struct UndoActions {
    let canUndo: Bool
    let canRedo: Bool
    let undo: () -> Void
    let redo: () -> Void
}

/// Edit ▸ Undo (⌘Z) and Redo (⇧⌘Z): the merge editor's choices, or else
/// what AppKit's own items do, sent to the text being typed (a conflict
/// edited by hand, a commit message, a search field).
struct UndoCommands: View {
    @FocusedValue(\.undoActions) private var actions

    var body: some View {
        Button(Shortcut.undo.title) {
            if let actions {
                actions.undo()
            } else {
                NSApp.sendAction(Selector(("undo:")), to: nil, from: nil)
            }
        }
        .keyboardShortcut(Shortcut.undo)
        .disabled(actions.map { !$0.canUndo } ?? false)
        Button(Shortcut.redo.title) {
            if let actions {
                actions.redo()
            } else {
                NSApp.sendAction(Selector(("redo:")), to: nil, from: nil)
            }
        }
        .keyboardShortcut(Shortcut.redo)
        .disabled(actions.map { !$0.canRedo } ?? false)
    }
}

extension FocusedValues {
    @Entry var undoActions: UndoActions?
    @Entry var editorLayoutActions: EditorLayoutActions?
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
    @State private var viewModel: RootViewModel
    /// The recent repositories, for the window with nothing open.
    @State private var repositories: OpenRepositoryViewModel

    init() {
        let viewModel = RootViewModel()
        _viewModel = State(initialValue: viewModel)
        _repositories = State(initialValue: viewModel.openRepositoryList())
    }
    /// ⌘ held alone, for the shortcut hints.
    @State private var commandHold = CommandHold()
    @Environment(\.controlActiveState) private var activeState
    @Environment(\.dismissWindow) private var dismissWindow

    var body: some View {
        Group {
            if let editor = viewModel.editor {
                MergeEditorView(viewModel: editor)
                    // A new file starts a new view: its scroll state and opening on a conflict.
                    .id(ObjectIdentifier(editor))
            } else if let opening = viewModel.opening {
                OpeningView(fileName: opening)
            } else if let changes = viewModel.changes {
                ChangesView(viewModel: changes)
                    // Another place is another view: its own task and scroll.
                    .id(ObjectIdentifier(changes))
            } else if let history = viewModel.history {
                HistoryView(viewModel: history)
                    .id(ObjectIdentifier(history))
            } else if let overview = viewModel.overview {
                OverviewView(viewModel: overview)
            } else if viewModel.canOpen {
                OpenRepositoryView(
                    viewModel: repositories,
                    error: viewModel.openError,
                    chooseRepository: chooseRepository,
                    chooseFile: chooseFile,
                    openFolder: { path in Task { await viewModel.openRepository(path) } }
                )
            } else {
                // Under git mergetool, whose file could not be opened.
                EmptyStateView(
                    message: viewModel.emptyMessage,
                    error: viewModel.openError,
                    canOpen: viewModel.canOpen,
                    open: chooseFile,
                    openRepository: chooseRepository
                )
            }
        }
        .frame(minWidth: 900, minHeight: 600)
        .environment(\.trail, TrailActions(
            canGoBack: viewModel.canGoBack, canGoForward: viewModel.canGoForward,
            backHelp: viewModel.backHelp, forwardHelp: viewModel.forwardHelp,
            back: { viewModel.goBack() }, forward: { viewModel.goForward() }
        ))
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
        .background { TitleBarDoubleClick() }
        .alert("Close without writing?", isPresented: $viewModel.confirmingClose) {
            Button("Close Without Writing", role: .destructive) {
                viewModel.closeWithoutWriting()
                dismissWindow(id: ScryApp.window)
            }
            Button("Cancel", role: .cancel) {}
        } message: {
            Text(viewModel.closeMessage)
        }
        .alert(
            "Scry could not open it",
            isPresented: Binding(
                get: { viewModel.failedOverOverview },
                set: { if !$0 { viewModel.dismissOpenError() } }
            )
        ) {
            Button("OK") { viewModel.dismissOpenError() }
        } message: {
            Text(viewModel.openError ?? "")
        }
        .onChange(of: viewModel.finished) { _, finished in
            if finished { dismissWindow(id: ScryApp.window) }
        }
        .onChange(of: viewModel.returnsToOverview) { _, returns in
            if returns { viewModel.leaveEditor() }
        }
        .task {
            ScryAppDelegate.exitStatus = { [viewModel] in viewModel.exitStatus }
            ScryAppDelegate.shouldQuit = { [viewModel] in viewModel.shouldClose() }
            await viewModel.openLaunchFile()
        }
    }

    private func chooseRepository() {
        let panel = NSOpenPanel()
        panel.canChooseFiles = false
        panel.canChooseDirectories = true
        panel.allowsMultipleSelection = false
        panel.message = "Choose a repository: Scry shows its operation in progress, or what is not committed"
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
    let message: String
    let error: String?
    /// No under `git mergetool`: the buttons would open a file git did not
    /// ask about.
    let canOpen: Bool
    let open: () -> Void
    let openRepository: () -> Void

    var body: some View {
        VStack(spacing: 12) {
            Image(systemName: "arrow.triangle.merge")
                .font(.system(size: 40))
                .foregroundStyle(Theme.Surfaces.textTertiary)
            Text("No file open")
                .font(.title3.weight(.semibold))
            Text(message)
                .foregroundStyle(Theme.Surfaces.textSecondary)
                .multilineTextAlignment(.center)
            if canOpen {
                HStack {
                    // ⌘O and ⇧⌘O are the File menu's (`OpenCommands`),
                    // which work whatever the window shows.
                    Button("Open Repository…", action: openRepository)
                    Button("Open Conflicted File…", action: open)
                }
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
    EmptyStateView(message: RootViewModel().emptyMessage, error: nil, canOpen: true, open: {}, openRepository: {})
}

#Preview("Empty state, file not opened", traits: .fixedLayout(width: 900, height: 600)) {
    EmptyStateView(
        message: RootViewModel().emptyMessage, error: "Uploader.swift has no conflict markers.",
        canOpen: true, open: {}, openRepository: {}
    )
}

#Preview("Empty state, under git mergetool", traits: .fixedLayout(width: 900, height: 600)) {
    EmptyStateView(
        message: "git mergetool gave Scry a file it could not open. Quit Scry to hand it back to git, which leaves it conflicted.",
        error: "Uploader.swift has no conflict markers.", canOpen: false, open: {}, openRepository: {}
    )
}

#Preview("Opening a file", traits: .fixedLayout(width: 900, height: 600)) {
    OpeningView(fileName: "Localizable.xcstrings")
}
