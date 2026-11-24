import Foundation
import Observation

/// Installs Scry's plugin in Claude Code: the user's `claude`, through the
/// core, in the app; a fake in tests and previews, which must never change
/// the user's Claude Code configuration.
@MainActor
protocol AgentRegistrar {
    /// Where Claude Code stands. Runs the user's shell and `claude`, which
    /// can take seconds.
    func setup() async -> AgentSetup
    /// Installs the plugin, or turns it on, and removes an earlier Scry's
    /// server.
    func install() async throws
    func uninstall() async throws
    func removeOldServer() async throws
    /// Allows the plugin's read tools, `explain` and `suggest` in Claude Code's user
    /// settings.
    func allowTools() async throws
    /// Who watches the repository holding `path`; `nil` outside one.
    func watch(path: String) async -> RepositoryWatch?
}

/// Whether Scry's plugin is on in Claude Code, as Settings › Agent last
/// found it: the "Claude Code is here" indicator shows only then (SPEC),
/// without running `claude` each time a window opens.
enum ClaudeCodePlugin {
    static let key = "claudeCodePluginOn"

    /// On until Settings › Agent finds otherwise: a user who never opened
    /// it sees "Claude Code is not here", which leads there.
    static func isOn(in defaults: UserDefaults) -> Bool {
        defaults.object(forKey: key) as? Bool ?? true
    }

    /// Without `claude`, or when its answer cannot be read, what was
    /// known stays.
    static func remember(_ state: AgentSetupState, in defaults: UserDefaults) {
        guard state != .noClaude else { return }
        defaults.set(state == .installed, forKey: key)
    }
}

/// The repository open in Scry's window, for Settings › Agent's
/// "Watching now": a file or a folder in it; set by the window.
@MainActor
enum OpenRepository {
    static var path: String?
}

/// Settings › Agent: the `git-scry` plugin in Claude Code, for every
/// project.
@MainActor
@Observable
final class AgentSettingsViewModel {
    /// What a button asked `claude` to do.
    enum Action: Equatable {
        case install, turnOn, uninstall, removeOldServer, allowTools

        /// "Installing Scry's plugin…", while it runs.
        var progress: String {
            switch self {
            case .install: "Installing Scry's plugin…"
            case .turnOn: "Turning on Scry's plugin…"
            case .uninstall: "Uninstalling Scry's plugin…"
            case .removeOldServer: "Removing the earlier server…"
            case .allowTools: "Allowing Scry's tools…"
            }
        }

        /// "Claude Code could not install the plugin", when it failed.
        var failure: String {
            switch self {
            case .install: "Claude Code could not install the plugin"
            case .turnOn: "Claude Code could not turn on the plugin"
            case .uninstall: "Claude Code could not uninstall the plugin"
            case .removeOldServer: "Claude Code could not remove the earlier server"
            case .allowTools: "Scry could not allow its tools"
            }
        }
    }

    /// Where Claude Code stands, once read.
    private(set) var setup: AgentSetup?
    /// The action running, until `claude` answers.
    private(set) var working: Action?
    /// The last action that failed, and what was answered.
    private(set) var failure: (action: Action, output: String)?
    /// Who watches the repository open in Scry, as last read.
    private(set) var watch: RepositoryWatch?
    /// Asking before uninstalling.
    var confirmingUninstall = false

    @ObservationIgnored private let registrar: AgentRegistrar
    @ObservationIgnored private var work: Task<Void, Never>?
    @ObservationIgnored private let defaults: UserDefaults
    @ObservationIgnored private let repository: () -> String?

    /// Explain in the conflict inspector (⌃⌘E): on by default; off, there
    /// is no Explain anywhere, and answers already received are hidden.
    var explainEnabled: Bool {
        didSet {
            guard explainEnabled != oldValue else { return }
            defaults.set(explainEnabled, forKey: InspectorViewModel.explainKey)
        }
    }

    /// `setup`: where Claude Code stands when already known, for previews;
    /// `defaults` keeps the Explain switch, tests pass their own suite;
    /// `repository` gives the repository open in Scry.
    init(
        registrar: AgentRegistrar,
        setup: AgentSetup? = nil,
        defaults: UserDefaults = .standard,
        repository: @escaping () -> String? = { OpenRepository.path }
    ) {
        self.registrar = registrar
        self.setup = setup
        self.defaults = defaults
        self.repository = repository
        explainEnabled = InspectorViewModel.explainEnabled(in: defaults)
    }

    var isWorking: Bool { working != nil }
    var isInstalled: Bool { setup?.state == .installed }

    /// Reads Claude Code's configuration again: the user may have changed
    /// it in a terminal since. Check Again, and the window coming forward.
    func refresh() {
        guard !isWorking else { return }
        work?.cancel()
        work = Task { [weak self] in await self?.read() }
    }

    func read() async {
        let setup = await registrar.setup()
        guard !Task.isCancelled else { return }
        self.setup = setup
        ClaudeCodePlugin.remember(setup.state, in: defaults)
    }

    // MARK: Claude Code

    /// The status line's glyph.
    enum Glyph: Equatable {
        case looking, info, working, failed, off, installed, warning
    }

    struct Headline: Equatable {
        let glyph: Glyph
        let title: String
        /// The line under it; `nil` when what follows says it.
        let detail: String?
    }

    var headline: Headline {
        if let working {
            return Headline(glyph: .working, title: working.progress, detail: nil)
        }
        if let failure {
            return Headline(glyph: .failed, title: failure.action.failure, detail: "claude answered:")
        }
        switch setup?.state {
        case nil:
            return Headline(
                glyph: .looking, title: "Looking for Claude Code…",
                detail: "Scry runs claude from your shell to see what is installed. This takes a few seconds."
            )
        case .noClaude:
            return Headline(
                glyph: .info, title: "Claude Code is not installed",
                detail: "Scry did not find the claude command in your shell."
            )
        case .notInstalled:
            return Headline(
                glyph: .info, title: "Scry's plugin is not installed",
                detail: "Claude Code is here. The plugin connects it to Scry."
            )
        case .disabled:
            return Headline(
                glyph: .off, title: "Scry's plugin is installed but turned off",
                detail: version.map { "\($0) · Claude Code does not load it until you turn it on." }
                    ?? "Claude Code does not load it until you turn it on."
            )
        case .installed:
            if setup?.pluginScry == nil {
                return Headline(glyph: .warning, title: "Scry's plugin is installed, but will not find Scry", detail: version)
            }
            return Headline(glyph: .installed, title: "Scry's plugin is installed", detail: version)
        }
    }

    /// "git-scry 0.3.0", once Claude Code says.
    private var version: String? {
        setup?.pluginVersion.map { "git-scry \($0)" }
    }

    /// Without Claude Code, its detection can be run again once installed.
    var canCheckAgain: Bool { !isWorking && setup?.state == .noClaude }

    /// Installing is offered once `claude` is found and the plugin is not
    /// on.
    var canInstall: Bool {
        guard !isWorking else { return false }
        switch setup?.state {
        case .notInstalled, .disabled: return true
        default: return false
        }
    }

    var installTitle: String {
        setup?.state == .disabled ? "Turn On Scry's Plugin" : "Install Scry's Plugin"
    }

    var canUninstall: Bool {
        guard !isWorking else { return false }
        switch setup?.state {
        case .installed, .disabled: return true
        default: return false
        }
    }

    /// The commands that do what the button does, to copy instead, one
    /// per line; none once installed.
    var command: String? {
        guard let setup, setup.state == .notInstalled || setup.state == .noClaude || failure?.action == .install
        else { return nil }
        return setup.command
    }

    /// Above the commands.
    var commandIntro: String {
        setup?.state == .noClaude
            ? "Once Claude Code is installed, run these in a terminal, or come back here:"
            : "Or run it yourself in a terminal:"
    }

    /// A callout: a title and what to do.
    struct Callout: Equatable {
        let title: String
        let text: String
    }

    /// An earlier Scry's server, which Claude Code shows next to the
    /// plugin's: installing removes it, and once installed a button does.
    var oldServerWarning: Callout? {
        guard setup?.oldServer == true else { return nil }
        return Callout(
            title: "Claude Code shows Scry twice",
            text: "An earlier version of Scry registered its server another way. Remove it so Claude Code keeps only the plugin."
        )
    }

    var canRemoveOldServer: Bool {
        !isWorking && setup?.oldServer == true && setup?.state != .noClaude
    }

    /// Said when the plugin's server would find no Scry to start.
    var missingScry: Callout? {
        guard let setup, setup.state != .noClaude, setup.pluginScry == nil else { return nil }
        return Callout(
            title: "The plugin will not find Scry",
            text: "Scry is not in the Applications folder and the git scry command is not installed, so Claude Code has no way to reach it. Move Scry to Applications, or install the command in Settings › CLI."
        )
    }

    /// Said when the plugin is installed and Claude Code would ask before
    /// each tool the Explain agent runs: the user's own session would stop
    /// on a question each time.
    var toolsWarning: Callout? {
        guard let setup, setup.state == .installed, !setup.toolsAllowed else { return nil }
        return Callout(
            title: "Claude Code asks before each Explain",
            text: "Claude Code asks before Scry's agent reads a conflict or answers an Explain. Allow Scry's tools so Explain is answered without a question; resolving still asks."
        )
    }

    var canAllowTools: Bool { !isWorking && toolsWarning != nil }

    /// Try Again runs the action that failed.
    func tryAgain() async {
        guard let action = failure?.action else { return }
        switch action {
        case .install, .turnOn: await install()
        case .uninstall: await uninstall()
        case .removeOldServer: await removeOldServer()
        case .allowTools: await allowTools()
        }
    }

    func allowTools() async {
        await run(.allowTools) { try await self.registrar.allowTools() }
    }

    func install() async {
        await run(setup?.state == .disabled ? .turnOn : .install) { try await self.registrar.install() }
    }

    func uninstall() async {
        await run(.uninstall) { try await self.registrar.uninstall() }
    }

    func removeOldServer() async {
        await run(.removeOldServer) { try await self.registrar.removeOldServer() }
    }

    private func run(_ action: Action, _ body: () async throws -> Void) async {
        working = action
        defer { working = nil }
        do {
            try await body()
            failure = nil
        } catch {
            failure = (action, error.message)
        }
        await read()
    }

    // MARK: The rows the plugin gives

    /// Watching now, Explain: shown dimmed until the plugin is on.
    var pluginOn: Bool { isInstalled }

    /// Under the dimmed rows.
    static let unavailable = "Available once the plugin is installed and on."

    /// Reads who watches the repository open in Scry: the view does it
    /// every two seconds while it shows.
    func refreshWatch() async {
        guard let path = repository() else {
            watch = nil
            return
        }
        let registrar = registrar
        watch = await registrar.watch(path: path)
    }

    /// "Watching now:" for the repository open in Scry.
    struct Watching: Equatable {
        /// The repository's name; `nil` when none is open.
        let name: String?
        let watching: Bool
        /// "1 session, since 14:02", "no session", or what to do.
        let detail: String
    }

    var watching: Watching {
        guard let watch else {
            return Watching(name: nil, watching: false, detail: "No repository is open in Scry.")
        }
        guard let first = watch.sessions.first else {
            return Watching(name: watch.name, watching: false, detail: "no session")
        }
        let since = Date(timeIntervalSince1970: Double(first)).formatted(date: .omitted, time: .shortened)
        let count = watch.sessions.count == 1 ? "1 session" : "\(watch.sessions.count) sessions"
        return Watching(name: watch.name, watching: true, detail: "\(count), since \(since)")
    }

    /// Under "Watching now:".
    var watchingNote: String {
        guard pluginOn else { return Self.unavailable }
        if watch == nil || watch?.sessions.isEmpty == true {
            return "Open Claude Code in the repository's folder and it shows here within a few seconds. Scry never starts Claude Code."
        }
        return "A session watches the repository it was opened in."
    }

    var explainDetail: String {
        guard pluginOn else { return Self.unavailable }
        return explainEnabled
            ? "Adds Explain to the conflict inspector (⌃⌘E). Read-only: it never changes a result. Uses your Claude Code plan."
            : "Off: no Explain action anywhere in Scry. Explanations already received are hidden, not deleted."
    }
}

/// The user's `claude`, through the core.
struct AppAgentRegistrar: AgentRegistrar {
    func setup() async -> AgentSetup {
        await Task.detached { agentSetup() }.value
    }

    func install() async throws {
        try await Task.detached { try installPlugin() }.value
    }

    func uninstall() async throws {
        try await Task.detached { try uninstallPlugin() }.value
    }

    func removeOldServer() async throws {
        try await Task.detached { try Scry.removeOldServer() }.value
    }

    func allowTools() async throws {
        try await Task.detached { try allowPluginTools() }.value
    }

    func watch(path: String) async -> RepositoryWatch? {
        await Task.detached { repositoryWatch(path: path) }.value
    }
}
