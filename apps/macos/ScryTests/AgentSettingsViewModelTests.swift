import Foundation
import Testing
@testable import Scry

/// Claude Code's configuration in memory: these tests must never run the
/// user's `claude` or change its configuration.
private final class FakeRegistrar: AgentRegistrar {
    var state: AgentSetupState
    var oldServer = false
    var pluginScry: String? = "/Applications/Scry.app/Contents/Helpers/git-scry"
    var failure: ScryError?
    private(set) var installs = 0

    init(_ state: AgentSetupState) {
        self.state = state
    }

    func setup() async -> AgentSetup {
        AgentSetup(
            state: state,
            command: "claude plugin marketplace add hnic0las/scry && claude plugin install git-scry@scry",
            oldServer: oldServer,
            pluginScry: pluginScry
        )
    }

    func install() async throws {
        if let failure { throw failure }
        installs += 1
        state = .installed
        oldServer = false
    }

    func uninstall() async throws {
        if let failure { throw failure }
        state = .notInstalled
    }

    func removeOldServer() async throws {
        if let failure { throw failure }
        oldServer = false
    }
}

@MainActor
struct AgentSettingsViewModelTests {
    /// Claude Code without the plugin: the button installs it for every project, after
    /// which the settings say so and offer to uninstall it instead; the commands to copy
    /// are shown only while there is something to install.
    @Test func installsThePlugin() async {
        let registrar = FakeRegistrar(.notInstalled)
        let agent = AgentSettingsViewModel(registrar: registrar)
        #expect(agent.status == "Looking for Claude Code…")
        #expect(!agent.canInstall)
        await agent.read()
        #expect(agent.status == "Claude Code does not have Scry's plugin yet.")
        #expect(agent.canInstall && !agent.canUninstall)
        #expect(agent.installTitle == "Install Scry's Plugin")
        #expect(agent.command == "claude plugin marketplace add hnic0las/scry && claude plugin install git-scry@scry")

        await agent.install()
        #expect(registrar.installs == 1)
        #expect(agent.isInstalled)
        #expect(agent.status == "Claude Code reads and resolves your conflicts through Scry's plugin, in every project.")
        #expect(!agent.canInstall && agent.canUninstall)
        #expect(agent.command == nil)
        #expect(!agent.isWorking)

        await agent.uninstall()
        #expect(agent.canInstall && !agent.isInstalled)
    }

    /// A plugin turned off is turned on by the same button, and can be uninstalled.
    @Test func turnsOnADisabledPlugin() async {
        let agent = AgentSettingsViewModel(registrar: FakeRegistrar(.disabled))
        await agent.read()
        #expect(agent.status == "Scry's plugin is installed in Claude Code, and turned off.")
        #expect(agent.installTitle == "Turn On Scry's Plugin")
        #expect(agent.canInstall && agent.canUninstall)
        #expect(agent.command == nil)
    }

    /// The server an earlier Scry added is named, so Claude Code shows Scry once: installing
    /// removes it, and with the plugin already installed its own button does.
    @Test func removesTheEarlierServer() async {
        let registrar = FakeRegistrar(.notInstalled)
        registrar.oldServer = true
        let agent = AgentSettingsViewModel(registrar: registrar)
        await agent.read()
        #expect(agent.oldServerWarning != nil)
        await agent.install()
        #expect(agent.oldServerWarning == nil && !agent.canRemoveOldServer)

        registrar.oldServer = true
        await agent.read()
        #expect(agent.canRemoveOldServer)
        await agent.removeOldServer()
        #expect(agent.oldServerWarning == nil)
        #expect(agent.isInstalled)
    }

    /// When the plugin's server would find no Scry to start, the settings say where to put
    /// it; without Claude Code that is not said, only the commands to run once it is there.
    @Test func saysWhenThePluginWouldNotFindScry() async {
        let registrar = FakeRegistrar(.notInstalled)
        registrar.pluginScry = nil
        let agent = AgentSettingsViewModel(registrar: registrar)
        await agent.read()
        #expect(agent.missingScry?.hasPrefix("The plugin will not find Scry") == true)

        registrar.state = .noClaude
        await agent.read()
        #expect(agent.missingScry == nil)
    }

    /// Without Claude Code there is nothing to press, only the commands to run once it is
    /// installed.
    @Test func withoutClaudeCodeOnlyTheCommandIsShown() async {
        let agent = AgentSettingsViewModel(registrar: FakeRegistrar(.noClaude))
        await agent.read()
        #expect(!agent.canInstall && !agent.canUninstall && !agent.canRemoveOldServer)
        #expect(agent.status.hasPrefix("Claude Code is not installed"))
        #expect(agent.command != nil)
    }

    /// When `claude` fails, its reason is shown and what Claude Code says is read again,
    /// rather than the settings claiming it worked; the next success clears the error.
    @Test func showsWhyClaudeFailed() async {
        let registrar = FakeRegistrar(.notInstalled)
        registrar.failure = .Failed(message: "claude plugin install failed: marketplace not found")
        let agent = AgentSettingsViewModel(registrar: registrar)
        await agent.read()
        await agent.install()
        #expect(agent.error == "claude plugin install failed: marketplace not found")
        #expect(!agent.isInstalled && agent.canInstall)

        registrar.failure = nil
        await agent.install()
        #expect(agent.error == nil)
        #expect(agent.isInstalled)
    }
}
