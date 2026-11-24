import Foundation
import Testing
@testable import Scry

/// Claude Code's configuration in memory: these tests must never run the
/// user's `claude` or change its configuration.
private final class FakeRegistrar: AgentRegistrar {
    var state: AgentSetupState
    var oldServer = false
    var pluginScry: String? = "/Applications/Scry.app/Contents/Helpers/git-scry"
    var toolsAllowed = false
    var failure: ScryError?
    var version: String? = "0.3.0"
    var watched: [String: RepositoryWatch] = [:]
    private(set) var installs = 0

    init(_ state: AgentSetupState) {
        self.state = state
    }

    func setup() async -> AgentSetup {
        AgentSetup(
            state: state,
            command: "claude plugin marketplace add hnic0las/scry\nclaude plugin install git-scry@scry",
            oldServer: oldServer,
            pluginScry: pluginScry,
            toolsAllowed: toolsAllowed,
            pluginVersion: state == .installed || state == .disabled ? version : nil
        )
    }

    func install() async throws {
        if let failure { throw failure }
        installs += 1
        state = .installed
        oldServer = false
        toolsAllowed = true
    }

    func uninstall() async throws {
        if let failure { throw failure }
        state = .notInstalled
    }

    func removeOldServer() async throws {
        if let failure { throw failure }
        oldServer = false
    }

    func allowTools() async throws {
        if let failure { throw failure }
        toolsAllowed = true
    }

    func watch(path: String) async -> RepositoryWatch? { watched[path] }
}

@MainActor
struct AgentSettingsViewModelTests {
    /// Claude Code without the plugin: the button installs it for every project, after
    /// which the settings say so with the plugin's version and offer to uninstall it; the
    /// commands to copy, one per line, are shown only while there is something to install.
    @Test func installsThePlugin() async {
        let registrar = FakeRegistrar(.notInstalled)
        let agent = AgentSettingsViewModel(registrar: registrar, defaults: .isolated)
        #expect(agent.headline.glyph == .looking)
        #expect(agent.headline.title == "Looking for Claude Code…")
        #expect(!agent.canInstall)
        await agent.read()
        #expect(agent.headline == .init(
            glyph: .info, title: "Scry's plugin is not installed",
            detail: "Claude Code is here. The plugin connects it to Scry."
        ))
        #expect(agent.canInstall && !agent.canUninstall && !agent.canCheckAgain)
        #expect(agent.installTitle == "Install Scry's Plugin")
        #expect(agent.command == "claude plugin marketplace add hnic0las/scry\nclaude plugin install git-scry@scry")
        #expect(agent.commandIntro == "Or run it yourself in a terminal:")
        #expect(!agent.pluginOn)

        await agent.install()
        #expect(registrar.installs == 1)
        #expect(agent.isInstalled && agent.pluginOn)
        #expect(agent.headline == .init(glyph: .installed, title: "Scry's plugin is installed", detail: "git-scry 0.3.0"))
        #expect(!agent.canInstall && agent.canUninstall)
        #expect(agent.command == nil)
        #expect(!agent.isWorking)

        await agent.uninstall()
        #expect(agent.canInstall && !agent.isInstalled)
    }

    /// A plugin turned off is turned on by the same button, and can be uninstalled.
    @Test func turnsOnADisabledPlugin() async {
        let agent = AgentSettingsViewModel(registrar: FakeRegistrar(.disabled), defaults: .isolated)
        await agent.read()
        #expect(agent.headline == .init(
            glyph: .off, title: "Scry's plugin is installed but turned off",
            detail: "git-scry 0.3.0 · Claude Code does not load it until you turn it on."
        ))
        #expect(agent.installTitle == "Turn On Scry's Plugin")
        #expect(agent.canInstall && agent.canUninstall)
        #expect(agent.command == nil)
        #expect(!agent.pluginOn)
        #expect(agent.explainDetail == AgentSettingsViewModel.unavailable)
    }

    /// The server an earlier Scry added is named, so Claude Code shows Scry once: installing
    /// removes it, and with the plugin already installed its own button does.
    @Test func removesTheEarlierServer() async {
        let registrar = FakeRegistrar(.notInstalled)
        registrar.oldServer = true
        let agent = AgentSettingsViewModel(registrar: registrar, defaults: .isolated)
        await agent.read()
        #expect(agent.oldServerWarning?.title == "Claude Code shows Scry twice")
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
    /// it, and an installed plugin says it will not find Scry; without Claude Code that is
    /// not said.
    @Test func saysWhenThePluginWouldNotFindScry() async {
        let registrar = FakeRegistrar(.notInstalled)
        registrar.pluginScry = nil
        let agent = AgentSettingsViewModel(registrar: registrar, defaults: .isolated)
        await agent.read()
        #expect(agent.missingScry?.title == "The plugin will not find Scry")
        registrar.state = .installed
        await agent.read()
        #expect(agent.headline.glyph == .warning)
        #expect(agent.headline.title == "Scry's plugin is installed, but will not find Scry")

        registrar.state = .noClaude
        await agent.read()
        #expect(agent.missingScry == nil)
    }

    /// Without Claude Code there is nothing to install: Check Again runs the detection once
    /// it is, and the commands say to run them once it is installed.
    @Test func withoutClaudeCodeCheckAgain() async {
        let registrar = FakeRegistrar(.noClaude)
        let agent = AgentSettingsViewModel(registrar: registrar, defaults: .isolated)
        await agent.read()
        #expect(!agent.canInstall && !agent.canUninstall && !agent.canRemoveOldServer)
        #expect(agent.headline.title == "Claude Code is not installed")
        #expect(agent.canCheckAgain)
        #expect(agent.command != nil)
        #expect(agent.commandIntro.hasPrefix("Once Claude Code is installed"))

        registrar.state = .notInstalled
        agent.refresh()
        for _ in 0..<100 where agent.setup?.state != .notInstalled {
            try? await Task.sleep(for: .milliseconds(10))
        }
        #expect(agent.setup?.state == .notInstalled)
        #expect(!agent.canCheckAgain)
    }

    /// When `claude` fails, what it answered is shown whole under the action's failure, with
    /// the commands to run instead; Try Again runs it again, and a success clears it.
    @Test func showsWhatClaudeAnswered() async {
        let registrar = FakeRegistrar(.notInstalled)
        registrar.failure = .Failed(message: "claude plugin install failed:\nError: failed to add marketplace\nfatal: could not resolve host")
        let agent = AgentSettingsViewModel(registrar: registrar, defaults: .isolated)
        await agent.read()
        await agent.install()
        #expect(agent.headline == .init(glyph: .failed, title: "Claude Code could not install the plugin", detail: "claude answered:"))
        #expect(agent.failure?.output.contains("fatal: could not resolve host") == true)
        #expect(!agent.isInstalled && agent.canInstall)
        #expect(agent.command != nil)

        registrar.failure = nil
        await agent.tryAgain()
        #expect(agent.failure == nil)
        #expect(agent.isInstalled)

        registrar.failure = .Failed(message: "claude plugin uninstall failed:\nbusy")
        await agent.uninstall()
        #expect(agent.headline.title == "Claude Code could not uninstall the plugin")
        #expect(agent.command == nil)
    }

    /// A plugin installed without Scry (from Claude Code) leaves Claude Code asking before
    /// each tool Explain's agent runs: the settings say so and allow them in one click;
    /// once allowed, or with no plugin, nothing is said.
    @Test func allowsThePluginsTools() async {
        let registrar = FakeRegistrar(.installed)
        let agent = AgentSettingsViewModel(registrar: registrar, defaults: .isolated)
        await agent.read()
        #expect(agent.toolsWarning?.text.hasPrefix("Claude Code asks before Scry's agent") == true)
        #expect(agent.canAllowTools)
        await agent.allowTools()
        #expect(agent.toolsWarning == nil && !agent.canAllowTools)

        registrar.state = .notInstalled
        registrar.toolsAllowed = false
        await agent.read()
        #expect(agent.toolsWarning == nil)
        await agent.install()
        #expect(agent.toolsWarning == nil)
    }

    /// Watching now: the repository open in Scry, whether a session watches it and since
    /// when; none open, it says so; until the plugin is on, the row is not available.
    @Test func showsWhoWatchesTheOpenRepository() async {
        let registrar = FakeRegistrar(.installed)
        registrar.toolsAllowed = true
        var open: String? = nil
        let agent = AgentSettingsViewModel(registrar: registrar, defaults: .isolated, repository: { open })
        await agent.read()
        await agent.refreshWatch()
        #expect(agent.watching == .init(name: nil, watching: false, detail: "No repository is open in Scry."))
        #expect(agent.watchingNote.hasPrefix("Open Claude Code in the repository's folder"))

        open = "/r/offline-sync"
        registrar.watched[open!] = RepositoryWatch(name: "offline-sync", sessions: [])
        await agent.refreshWatch()
        #expect(agent.watching == .init(name: "offline-sync", watching: false, detail: "no session"))

        let since = Date(timeIntervalSince1970: 1_800_000_000)
        registrar.watched[open!] = RepositoryWatch(name: "offline-sync", sessions: [1_800_000_000, 1_800_000_100])
        await agent.refreshWatch()
        let time = since.formatted(date: .omitted, time: .shortened)
        #expect(agent.watching == .init(name: "offline-sync", watching: true, detail: "2 sessions, since \(time)"))
        #expect(agent.watchingNote == "A session watches the repository it was opened in.")

        registrar.state = .notInstalled
        await agent.read()
        #expect(agent.watchingNote == AgentSettingsViewModel.unavailable)
    }
}
