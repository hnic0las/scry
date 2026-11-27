import Foundation
import Observation

/// Reads and writes git's merge tool and the `git scry` command: the core,
/// through the FFI, in the app; a fake in tests and previews, which must
/// never touch the user's git configuration or folders.
@MainActor
protocol MergetoolInstaller {
    /// Whether the app carries the `git-scry` the command runs.
    var isAvailable: Bool { get }
    /// Where the app's `git-scry` is.
    var cli: String { get }
    /// The command git would run, to show it.
    var command: String { get }
    func setup() throws -> MergetoolSetup
    func install() throws
    /// The merge tool Scry replaced, put back by `uninstall`.
    func previousTool() throws -> String?
    func uninstall() throws
    /// What `git scry` runs in the user's terminal. Reads their shell's
    /// `PATH`, which can take a second.
    func cliCommandState() async -> CliCommandState
    /// Links `link` to the app's `git-scry`.
    func installCommand(link: String) throws
}

/// Reads and writes git's diff tool: the core in the app, a fake in tests
/// and previews.
@MainActor
protocol DifftoolInstaller {
    /// The command git would run, to show it.
    var command: String { get }
    func setup() throws -> MergetoolSetup
    func install() throws
    /// The diff tool Scry replaced, put back by `uninstall`.
    func previousTool() throws -> String?
    func uninstall() throws
}

/// The app's settings: making Scry git's merge tool, so `git mergetool`
/// opens conflicts in the app.
@MainActor
@Observable
final class SettingsViewModel {
    /// What git's configuration says, once read.
    private(set) var setup: MergetoolSetup?
    /// The merge tool Scry replaced, if any.
    private(set) var previousTool: String?
    /// Why git's configuration could not be read or written.
    private(set) var error: String?
    /// What git's configuration says about the diff tool, once read.
    private(set) var diffSetup: MergetoolSetup?
    /// The diff tool Scry replaced, if any.
    private(set) var diffPreviousTool: String?
    /// Why git's configuration could not be read or written for it.
    private(set) var diffError: String?
    /// What `git scry` runs, once read.
    private(set) var commandState: CliCommandState?
    /// Why the `git scry` link could not be made.
    private(set) var commandError: String?

    /// The merge editor's layout, the one ⌥⌘L and the toolbar switch:
    /// kept per user, read by every editor window.
    var editorLayout: MergeEditorViewModel.EditorLayout {
        didSet {
            guard editorLayout != oldValue else { return }
            defaults.set(editorLayout.rawValue, forKey: MergeEditorViewModel.layoutKey)
        }
    }

    @ObservationIgnored private let installer: MergetoolInstaller
    /// `nil` hides the diff tool: tests and previews of the merge tool
    /// alone.
    @ObservationIgnored private let diffInstaller: DifftoolInstaller?
    @ObservationIgnored private var commandWork: Task<Void, Never>?
    @ObservationIgnored private let defaults: UserDefaults

    /// `defaults` keeps the layout choice; tests pass their own suite.
    init(
        installer: MergetoolInstaller, diffInstaller: DifftoolInstaller? = nil, defaults: UserDefaults = .standard
    ) {
        self.installer = installer
        self.diffInstaller = diffInstaller
        self.defaults = defaults
        editorLayout = Self.storedLayout(in: defaults)
    }

    private static func storedLayout(in defaults: UserDefaults) -> MergeEditorViewModel.EditorLayout {
        defaults.string(forKey: MergeEditorViewModel.layoutKey)
            .flatMap(MergeEditorViewModel.EditorLayout.init(rawValue:)) ?? .columns
    }

    /// The command git will run for each conflicted file.
    var command: String { installer.command }

    /// The app's own `git-scry`, which the command runs.
    var cliPath: String { installer.cli }

    var status: String {
        guard installer.isAvailable else {
            return "This build of Scry has no git-scry inside: build the app with Xcode."
        }
        switch setup {
        case nil:
            return "Cannot read git's configuration."
        case .notSet:
            return "git has no merge tool set: git mergetool asks for one."
        case .other(let tool):
            return "git's merge tool is \(tool). Scry will replace it."
        case .scry(current: true):
            return "Scry is git's merge tool: git mergetool opens conflicts here."
                + (previousTool.map { " Stop Using Scry puts \($0) back." } ?? "")
        case .scry(current: false):
            return "git runs Scry from another place, moved or deleted since."
        }
    }

    var isInstalled: Bool { setup == .scry(current: true) }

    var canInstall: Bool { installer.isAvailable && !isInstalled }

    /// Scry is git's merge tool, from here or from an app moved since.
    var canUninstall: Bool {
        if case .scry = setup { return true }
        return false
    }

    var buttonTitle: String {
        setup == .scry(current: false) ? "Update Merge Tool" : "Use Scry as git's Merge Tool"
    }

    /// Reads git's configuration again: it may have changed since the
    /// settings were last open.
    func refresh() {
        // An editor may have switched the layout since.
        let stored = Self.storedLayout(in: defaults)
        if stored != editorLayout { editorLayout = stored }
        do {
            setup = try installer.setup()
            previousTool = try installer.previousTool()
            error = nil
        } catch {
            setup = nil
            self.error = error.message
        }
        refreshDiffTool()
        commandWork?.cancel()
        commandWork = Task { [weak self] in await self?.refreshCommand() }
    }

    // MARK: The diff tool

    /// Whether the settings offer the diff tool at all.
    var offersDiffTool: Bool { diffInstaller != nil }

    /// The command git will run for each file, or once with `--dir-diff`.
    var diffCommand: String { diffInstaller?.command ?? "" }

    private func refreshDiffTool() {
        guard let diffInstaller else { return }
        do {
            diffSetup = try diffInstaller.setup()
            diffPreviousTool = try diffInstaller.previousTool()
            diffError = nil
        } catch {
            diffSetup = nil
            diffError = error.message
        }
    }

    var diffStatus: String {
        guard installer.isAvailable else {
            return "This build of Scry has no git-scry inside: build the app with Xcode."
        }
        switch diffSetup {
        case nil:
            return "Cannot read git's configuration."
        case .notSet:
            return "git has no diff tool set: git difftool asks for one."
        case .other(let tool):
            return "git's diff tool is \(tool). Scry will replace it."
        case .scry(current: true):
            return "Scry is git's diff tool: git difftool opens files here."
                + (diffPreviousTool.map { " Stop Using Scry puts \($0) back." } ?? "")
        case .scry(current: false):
            return "git runs Scry from another place, moved or deleted since."
        }
    }

    var isDiffToolInstalled: Bool { diffSetup == .scry(current: true) }

    var canInstallDiffTool: Bool { installer.isAvailable && diffInstaller != nil && !isDiffToolInstalled }

    /// Scry is git's diff tool, from here or from an app moved since.
    var canUninstallDiffTool: Bool {
        if case .scry = diffSetup { return true }
        return false
    }

    var diffButtonTitle: String {
        diffSetup == .scry(current: false) ? "Update Diff Tool" : "Use Scry as git's Diff Tool"
    }

    func installDiffTool() {
        guard canInstallDiffTool, let diffInstaller else { return }
        do {
            try diffInstaller.install()
            refreshDiffTool()
        } catch {
            diffError = error.message
        }
    }

    /// Puts back the diff tool Scry replaced, or none.
    func uninstallDiffTool() {
        guard canUninstallDiffTool, let diffInstaller else { return }
        do {
            try diffInstaller.uninstall()
            refreshDiffTool()
        } catch {
            diffError = error.message
        }
    }

    // MARK: The git scry command

    /// Reads what `git scry` runs again: the user may have changed their
    /// `PATH` or removed another git-scry since.
    func refreshCommand() async {
        let state = await installer.cliCommandState()
        guard !Task.isCancelled else { return }
        commandState = state
    }

    var isCommandInstalled: Bool {
        if case .installed = commandState { return true }
        return false
    }

    /// Where the button links the app's git-scry; `nil` when it cannot.
    private var commandLink: String? {
        switch commandState {
        case .notInstalled(let link), .other(_, let link): link
        default: nil
        }
    }

    var canInstallCommand: Bool { installer.isAvailable && commandLink != nil }

    var commandStatus: String {
        switch commandState {
        case nil:
            "Looking for git-scry on your PATH…"
        case .installed(let path):
            "git scry runs this app's git-scry, linked at \(path)."
        case .notInstalled:
            "git scry is not installed: git finds no git-scry on your PATH."
        case .other(let other, _):
            "git scry runs \(other), not this app's."
        case .blocked(let other, _):
            "git scry runs \(other), which Scry does not replace. Remove it in a terminal, then come back:"
        case .noFolder:
            "None of ~/.local/bin, /opt/homebrew/bin or /usr/local/bin is on your PATH and writable. In a terminal, link git-scry and add ~/.local/bin to your PATH, then open a new terminal:"
        }
    }

    var commandButtonTitle: String {
        if case .other = commandState { return "Use This App's git-scry" }
        return "Install git scry"
    }

    /// What the button does, under it.
    var commandDetail: String? {
        switch commandState {
        case .notInstalled(let link):
            "Links \(link) to the app's git-scry."
        case .other(let other, let link) where other != link:
            "Links \(link) to the app's git-scry; it comes first on your PATH."
        case .other(_, let link):
            "Replaces the link \(link) with one to the app's git-scry."
        default:
            nil
        }
    }

    /// Commands to copy into a terminal when the app cannot link itself.
    var commandsToRun: [String] {
        switch commandState {
        case .blocked(_, let remove):
            [remove]
        case .noFolder:
            [
                "mkdir -p ~/.local/bin && ln -sf '\(installer.cli.replacingOccurrences(of: "'", with: "'\\''"))' ~/.local/bin/git-scry",
                "echo 'export PATH=\"$HOME/.local/bin:$PATH\"' >> ~/.zshrc",
            ]
        default:
            []
        }
    }

    func installCommand() async {
        guard canInstallCommand, let link = commandLink else { return }
        do {
            try installer.installCommand(link: link)
            commandError = nil
        } catch {
            commandError = error.message
        }
        await refreshCommand()
    }

    /// Puts back the merge tool Scry replaced, or none.
    func uninstall() {
        guard canUninstall else { return }
        do {
            try installer.uninstall()
            refresh()
        } catch {
            self.error = error.message
        }
    }

    func install() {
        guard canInstall else { return }
        do {
            try installer.install()
            refresh()
        } catch {
            self.error = error.message
        }
    }
}

/// The app's own `git-scry` and bundle, through the core.
struct AppMergetoolInstaller: MergetoolInstaller {
    let cli: String
    let app: String

    init(bundle: Bundle = .main) {
        app = bundle.bundlePath
        cli = bundle.bundleURL.appending(path: "Contents/Helpers/git-scry").path
    }

    var isAvailable: Bool { FileManager.default.isExecutableFile(atPath: cli) }

    var command: String { mergetoolCommand(cli: cli, app: app) }

    func setup() throws -> MergetoolSetup {
        try mergetoolSetup(cli: cli, app: app)
    }

    func install() throws {
        try installMergetool(cli: cli, app: app)
    }

    func previousTool() throws -> String? {
        try mergetoolPrevious()
    }

    func uninstall() throws {
        try uninstallMergetool()
    }

    func cliCommandState() async -> CliCommandState {
        let cli = cli
        return await Task.detached { Scry.cliCommandState(cli: cli) }.value
    }

    func installCommand(link: String) throws {
        try installCliCommand(cli: cli, link: link)
    }
}

/// Git's diff tool for the app's own `git-scry` and bundle, through the
/// core.
struct AppDifftoolInstaller: DifftoolInstaller {
    let cli: String
    let app: String

    init(bundle: Bundle = .main) {
        app = bundle.bundlePath
        cli = bundle.bundleURL.appending(path: "Contents/Helpers/git-scry").path
    }

    var command: String { difftoolCommand(cli: cli, app: app) }

    func setup() throws -> MergetoolSetup {
        try difftoolSetup(cli: cli, app: app)
    }

    func install() throws {
        try installDifftool(cli: cli, app: app)
    }

    func previousTool() throws -> String? {
        try difftoolPrevious()
    }

    func uninstall() throws {
        try uninstallDifftool()
    }
}
