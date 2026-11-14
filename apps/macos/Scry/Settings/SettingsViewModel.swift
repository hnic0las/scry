import Foundation
import Observation

/// Reads and writes git's merge tool: the core, through the FFI, in the
/// app; a fake in tests and previews, which must never touch the user's
/// git configuration.
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

    /// The merge editor's layout, the one ⌥⌘L and the toolbar switch:
    /// kept per user, read by every editor window.
    var editorLayout: MergeEditorViewModel.EditorLayout {
        didSet {
            guard editorLayout != oldValue else { return }
            defaults.set(editorLayout.rawValue, forKey: MergeEditorViewModel.layoutKey)
        }
    }

    @ObservationIgnored private let installer: MergetoolInstaller
    @ObservationIgnored private let defaults: UserDefaults

    /// `defaults` keeps the layout choice; tests pass their own suite.
    init(installer: MergetoolInstaller, defaults: UserDefaults = .standard) {
        self.installer = installer
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
}
