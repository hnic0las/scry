import Foundation
import Testing
@testable import Scry

/// Claude Code's configuration in memory: these tests must never run the
/// user's `claude` or change its configuration.
private final class FakeRegistrar: AgentRegistrar {
    var state: AgentSetupState
    var failure: ScryError?
    private(set) var registrations = 0

    init(_ state: AgentSetupState) {
        self.state = state
    }

    func setup() async -> AgentSetup {
        AgentSetup(state: state, command: "claude mcp add --scope user scry -- /H/.local/bin/git-scry mcp")
    }

    func register() async throws {
        if let failure { throw failure }
        registrations += 1
        state = .registered
    }

    func unregister() async throws {
        if let failure { throw failure }
        state = .notRegistered
    }
}

@MainActor
struct AgentSettingsViewModelTests {
    /// Claude Code without Scry: the button adds it for every project, after which the
    /// settings say so and offer to remove it instead; the command to copy stays shown.
    @Test func addsScryToClaudeCode() async {
        let registrar = FakeRegistrar(.notRegistered)
        let agent = AgentSettingsViewModel(registrar: registrar)
        #expect(agent.status == "Looking for Claude Code…")
        #expect(!agent.canRegister)
        await agent.read()
        #expect(agent.status == "Claude Code does not know Scry yet.")
        #expect(agent.canRegister && !agent.canUnregister)
        #expect(agent.registerTitle == "Add Scry to Claude Code")
        #expect(agent.command == "claude mcp add --scope user scry -- /H/.local/bin/git-scry mcp")

        await agent.register()
        #expect(registrar.registrations == 1)
        #expect(agent.isRegistered)
        #expect(agent.status == "Claude Code can read your conflicts through Scry, in every project.")
        #expect(!agent.canRegister && agent.canUnregister)
        #expect(!agent.isWorking)

        await agent.unregister()
        #expect(agent.canRegister && !agent.isRegistered)
    }

    /// Another Scry registered (a build moved since): the settings name what Claude Code
    /// runs, and offer to use this app instead, or to remove it.
    @Test func replacesAnotherScry() async {
        let agent = AgentSettingsViewModel(registrar: FakeRegistrar(.other(command: "/old/git-scry mcp")))
        await agent.read()
        #expect(agent.status == "Claude Code runs Scry's server as /old/git-scry mcp, not with this app.")
        #expect(agent.registerTitle == "Use This App's Scry")
        #expect(agent.canRegister && agent.canUnregister)
    }

    /// Without Claude Code there is nothing to press, only the command to run once it is
    /// installed.
    @Test func withoutClaudeCodeOnlyTheCommandIsShown() async {
        let agent = AgentSettingsViewModel(registrar: FakeRegistrar(.noClaude))
        await agent.read()
        #expect(!agent.canRegister && !agent.canUnregister)
        #expect(agent.status.hasPrefix("Claude Code is not installed"))
        #expect(agent.command != nil)
    }

    /// When `claude` fails, its reason is shown and what Claude Code says is read again,
    /// rather than the settings claiming it worked; the next success clears the error.
    @Test func showsWhyClaudeFailed() async {
        let registrar = FakeRegistrar(.notRegistered)
        registrar.failure = .Failed(message: "claude mcp add failed: permission denied")
        let agent = AgentSettingsViewModel(registrar: registrar)
        await agent.read()
        await agent.register()
        #expect(agent.error == "claude mcp add failed: permission denied")
        #expect(!agent.isRegistered && agent.canRegister)

        registrar.failure = nil
        await agent.register()
        #expect(agent.error == nil)
        #expect(agent.isRegistered)
    }
}
