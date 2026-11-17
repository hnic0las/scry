import Foundation
import Observation

/// Registers Scry's MCP server in Claude Code: the user's `claude`, through
/// the core, in the app; a fake in tests and previews, which must never
/// change the user's Claude Code configuration.
@MainActor
protocol AgentRegistrar {
    /// Where Claude Code stands. Runs the user's shell and `claude`, which
    /// can take seconds.
    func setup() async -> AgentSetup
    func register() async throws
    func unregister() async throws
}

/// Settings › Agent: Scry's MCP server in Claude Code, for every project.
@MainActor
@Observable
final class AgentSettingsViewModel {
    /// Where Claude Code stands, once read.
    private(set) var setup: AgentSetup?
    /// Registering or removing, until `claude` answers.
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

    var isRegistered: Bool { setup?.state == .registered }

    var status: String {
        switch setup?.state {
        case nil:
            "Looking for Claude Code…"
        case .noClaude:
            "Claude Code is not installed, or not on your terminal's PATH. Once it is, add Scry from here, or run in a terminal:"
        case .notRegistered:
            "Claude Code does not know Scry yet."
        case .registered:
            "Claude Code can read your conflicts through Scry, in every project."
        case .other(let command):
            "Claude Code runs Scry's server as \(command), not with this app."
        }
    }

    /// Adding is offered once `claude` is found and does not run this app.
    var canRegister: Bool {
        guard !isWorking else { return false }
        switch setup?.state {
        case .notRegistered, .other: return true
        default: return false
        }
    }

    var registerTitle: String {
        if case .other = setup?.state { return "Use This App's Scry" }
        return "Add Scry to Claude Code"
    }

    var canUnregister: Bool {
        guard !isWorking else { return false }
        switch setup?.state {
        case .registered, .other: return true
        default: return false
        }
    }

    /// The command that does what the button does, to copy instead.
    var command: String? { setup?.command }

    func register() async {
        await run { try await self.registrar.register() }
    }

    func unregister() async {
        await run { try await self.registrar.unregister() }
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
    /// The app's own `git-scry`.
    let cli: String

    init(bundle: Bundle = .main) {
        cli = bundle.bundleURL.appending(path: "Contents/Helpers/git-scry").path
    }

    func setup() async -> AgentSetup {
        let cli = cli
        return await Task.detached { agentSetup(cli: cli) }.value
    }

    func register() async throws {
        let cli = cli
        try await Task.detached { try registerAgent(cli: cli) }.value
    }

    func unregister() async throws {
        let cli = cli
        try await Task.detached { try unregisterAgent(cli: cli) }.value
    }
}
