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
    /// Allows the plugin's read tools and `explain` in Claude Code's user
    /// settings.
    func allowTools() async throws
}

/// Settings › Agent: the `git-scry` plugin in Claude Code, for every
/// project.
@MainActor
@Observable
final class AgentSettingsViewModel {
    /// Where Claude Code stands, once read.
    private(set) var setup: AgentSetup?
    /// Installing or removing, until `claude` answers.
    private(set) var isWorking = false
    /// Why `claude` failed.
    private(set) var error: String?

    @ObservationIgnored private let registrar: AgentRegistrar
    @ObservationIgnored private var work: Task<Void, Never>?

    /// `setup`: where Claude Code stands when already known, for previews.
    init(registrar: AgentRegistrar, setup: AgentSetup? = nil) {
        self.registrar = registrar
        self.setup = setup
    }

    /// Reads Claude Code's configuration again: the user may have changed
    /// it in a terminal since.
    func refresh() {
        work?.cancel()
        work = Task { [weak self] in await self?.read() }
    }

    func read() async {
        let setup = await registrar.setup()
        guard !Task.isCancelled else { return }
        self.setup = setup
    }

    var isInstalled: Bool { setup?.state == .installed }

    var status: String {
        switch setup?.state {
        case nil:
            "Looking for Claude Code…"
        case .noClaude:
            "Claude Code is not installed, or not on your terminal's PATH. Once it is, install Scry's plugin from here, or run in a terminal:"
        case .notInstalled:
            "Claude Code does not have Scry's plugin yet."
        case .disabled:
            "Scry's plugin is installed in Claude Code, and turned off."
        case .installed:
            "Claude Code reads and resolves your conflicts through Scry's plugin, in every project."
        }
    }

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

    /// The commands that do what the button does, to copy instead; none
    /// once installed.
    var command: String? {
        guard let setup, setup.state == .notInstalled || setup.state == .noClaude else { return nil }
        return setup.command
    }

    /// An earlier Scry's server, which Claude Code shows next to the
    /// plugin's: installing removes it, and once installed a button does.
    var oldServerWarning: String? {
        guard setup?.oldServer == true else { return nil }
        return "An earlier Scry added its server to Claude Code with claude mcp add, so Claude Code shows Scry twice. The plugin replaces it."
    }

    var canRemoveOldServer: Bool {
        !isWorking && setup?.oldServer == true && setup?.state != .noClaude
    }

    /// Said when the plugin's server would find no Scry to start.
    var missingScry: String? {
        guard let setup, setup.state != .noClaude, setup.pluginScry == nil else { return nil }
        return "The plugin will not find Scry: move Scry to the Applications folder, or install the git scry command in Settings › CLI."
    }

    /// Said when the plugin is installed and Claude Code would ask before
    /// each tool the Explain agent runs: the user's own session would stop
    /// on a question each time.
    var toolsWarning: String? {
        guard let setup, setup.state == .installed, !setup.toolsAllowed else { return nil }
        return "Claude Code asks before Scry's agent reads a conflict or answers an Explain. Allow Scry's tools so Explain is answered without a question; resolving still asks."
    }

    var canAllowTools: Bool { !isWorking && toolsWarning != nil }

    func allowTools() async {
        await run { try await self.registrar.allowTools() }
    }

    func install() async {
        await run { try await self.registrar.install() }
    }

    func uninstall() async {
        await run { try await self.registrar.uninstall() }
    }

    func removeOldServer() async {
        await run { try await self.registrar.removeOldServer() }
    }

    private func run(_ action: () async throws -> Void) async {
        isWorking = true
        defer { isWorking = false }
        do {
            try await action()
            error = nil
        } catch {
            self.error = error.message
        }
        await read()
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
}
