import Foundation
import Testing
@testable import Scry

/// Git's merge tool in memory: these tests must never write the user's
/// git configuration.
private final class FakeInstaller: MergetoolInstaller {
    var state: MergetoolSetup
    var isAvailable = true
    let cli = "/A/git-scry"
    var failure: ScryError?
    private(set) var installs = 0

    init(_ state: MergetoolSetup) {
        self.state = state
    }

    var command: String { "\"/A/git-scry\" resolve --app=\"/A/Scry.app\" \"$BASE\" \"$LOCAL\" \"$REMOTE\" \"$MERGED\"" }

    func setup() throws -> MergetoolSetup { state }

    var previous: String?

    func install() throws {
        if let failure { throw failure }
        installs += 1
        if case .other(let tool) = state { previous = tool }
        state = .scry(current: true)
    }

    func previousTool() throws -> String? { previous }

    func uninstall() throws {
        state = previous.map { .other(tool: $0) } ?? .notSet
        previous = nil
    }

    /// What `git scry` runs; installing makes it this app's.
    var commandState: CliCommandState = .notInstalled(link: "/H/.local/bin/git-scry")
    var commandFailure: ScryError?
    private(set) var links: [String] = []

    func cliCommandState() async -> CliCommandState { commandState }

    func installCommand(link: String) throws {
        if let commandFailure { throw commandFailure }
        links.append(link)
        commandState = .installed(path: link)
    }
}

@MainActor
struct SettingsViewModelTests {
    /// With another tool set, the settings say which one Scry replaces,
    /// and the button makes Scry the merge tool, after which it is no
    /// longer offered.
    @Test func replacesAnotherTool() {
        let installer = FakeInstaller(.other(tool: "opendiff"))
        let settings = SettingsViewModel(installer: installer, defaults: .isolated)
        settings.refresh()

        #expect(settings.status.contains("opendiff"))
        #expect(settings.canInstall)
        #expect(settings.buttonTitle == "Use Scry as git's Merge Tool")

        settings.install()

        #expect(installer.installs == 1)
        #expect(settings.isInstalled)
        #expect(!settings.canInstall)
    }

    /// Trying Scry can be undone: once installed, the settings name the tool
    /// it replaced, and Stop Using Scry puts it back.
    @Test func stopsUsingScry() {
        let installer = FakeInstaller(.other(tool: "opendiff"))
        let settings = SettingsViewModel(installer: installer, defaults: .isolated)
        settings.refresh()
        #expect(!settings.canUninstall)

        settings.install()
        #expect(settings.canUninstall)
        #expect(settings.status.hasSuffix("Stop Using Scry puts opendiff back."))

        settings.uninstall()
        #expect(settings.setup == .other(tool: "opendiff"))
        #expect(!settings.canUninstall)
        #expect(settings.canInstall)
    }

    /// The settings show the app's own git-scry and the exact command git
    /// runs, so they can be copied and checked.
    @Test func showsTheCommandLineTool() {
        let settings = SettingsViewModel(installer: FakeInstaller(.notSet), defaults: .isolated)

        #expect(settings.cliPath == "/A/git-scry")
        #expect(settings.command.hasPrefix("\"/A/git-scry\" resolve --app="))
    }

    /// A Scry command for an app moved since is offered as an update.
    @Test func offersToUpdateAMovedApp() {
        let settings = SettingsViewModel(installer: FakeInstaller(.scry(current: false)), defaults: .isolated)
        settings.refresh()

        #expect(!settings.isInstalled)
        #expect(settings.buttonTitle == "Update Merge Tool")
    }

    /// A build without its git-scry inside cannot install a command that
    /// would point at nothing.
    @Test func needsTheBundledCommandLineTool() {
        let installer = FakeInstaller(.notSet)
        installer.isAvailable = false
        let settings = SettingsViewModel(installer: installer, defaults: .isolated)
        settings.refresh()

        settings.install()

        #expect(!settings.canInstall)
        #expect(installer.installs == 0)
        #expect(settings.status.contains("no git-scry"))
    }

    /// When git refuses the change, the settings say why, in the core's
    /// sentence rather than `Failed(message: …)`, and stay as they were.
    @Test func showsWhyInstallingFailed() {
        let installer = FakeInstaller(.notSet)
        installer.failure = .Failed(message: "could not lock config file")
        let settings = SettingsViewModel(installer: installer, defaults: .isolated)
        settings.refresh()

        settings.install()

        #expect(settings.error == "could not lock config file")
        #expect(!settings.isInstalled)
    }

    /// The app's own installer points git at the git-scry inside the app,
    /// which the build puts there.
    @Test func theAppCarriesItsCommandLineTool() {
        let installer = AppMergetoolInstaller()

        #expect(installer.cli.hasSuffix("Scry.app/Contents/Helpers/git-scry"))
        #expect(installer.isAvailable)
        #expect(installer.command.contains("resolve --app="))
    }

    /// A user of the release has no `git scry` until the app links it: the
    /// settings say so, and the button links it where the core found room,
    /// after which `git scry` runs this app.
    @Test func installsTheGitScryCommand() async {
        let installer = FakeInstaller(.notSet)
        let settings = SettingsViewModel(installer: installer, defaults: .isolated)
        #expect(settings.commandStatus == "Looking for git-scry on your PATH…")
        #expect(!settings.canInstallCommand)

        await settings.refreshCommand()
        #expect(settings.commandStatus == "git scry is not installed: git finds no git-scry on your PATH.")
        #expect(settings.canInstallCommand)
        #expect(settings.commandButtonTitle == "Install git scry")
        #expect(settings.commandDetail == "Links /H/.local/bin/git-scry to the app's git-scry.")

        await settings.installCommand()
        #expect(installer.links == ["/H/.local/bin/git-scry"])
        #expect(settings.isCommandInstalled)
        #expect(!settings.canInstallCommand)
        #expect(settings.commandStatus == "git scry runs this app's git-scry, linked at /H/.local/bin/git-scry.")
    }

    /// Another copy of Scry, or one moved since, is replaced by this app's;
    /// a link made earlier on the PATH says so.
    @Test func replacesAnotherGitScry() async {
        let installer = FakeInstaller(.notSet)
        installer.commandState = .other(other: "/H/.cargo/bin/git-scry", link: "/H/.local/bin/git-scry")
        let settings = SettingsViewModel(installer: installer, defaults: .isolated)
        await settings.refreshCommand()

        #expect(settings.commandStatus == "git scry runs /H/.cargo/bin/git-scry, not this app's.")
        #expect(settings.commandButtonTitle == "Use This App's git-scry")
        #expect(settings.commandDetail == "Links /H/.local/bin/git-scry to the app's git-scry; it comes first on your PATH.")

        await settings.installCommand()
        #expect(installer.links == ["/H/.local/bin/git-scry"])
    }

    /// What the app cannot fix itself, it gives as commands to copy: another
    /// git-scry to remove, or a folder to put on the PATH.
    @Test func givesCommandsWhenItCannotLink() async {
        let installer = FakeInstaller(.notSet)
        installer.commandState = .blocked(other: "/H/.cargo/bin/git-scry", remove: "cargo uninstall git-scry")
        let settings = SettingsViewModel(installer: installer, defaults: .isolated)
        await settings.refreshCommand()
        #expect(!settings.canInstallCommand)
        #expect(settings.commandStatus == "git scry runs /H/.cargo/bin/git-scry, which Scry does not replace. Remove it in a terminal, then come back:")
        #expect(settings.commandsToRun == ["cargo uninstall git-scry"])

        installer.commandState = .noFolder
        await settings.refreshCommand()
        #expect(!settings.canInstallCommand)
        #expect(settings.commandsToRun == [
            "mkdir -p ~/.local/bin && ln -sf '/A/git-scry' ~/.local/bin/git-scry",
            "echo 'export PATH=\"$HOME/.local/bin:$PATH\"' >> ~/.zshrc",
        ])
    }

    /// A link that cannot be made says why, and leaves the state as it was.
    @Test func showsWhyLinkingFailed() async {
        let installer = FakeInstaller(.notSet)
        installer.commandFailure = .Refused(message: "Move Scry to the Applications folder.")
        let settings = SettingsViewModel(installer: installer, defaults: .isolated)
        await settings.refreshCommand()

        await settings.installCommand()
        #expect(settings.commandError == "Move Scry to the Applications folder.")
        #expect(!settings.isCommandInstalled)
    }

    /// Settings ▸ Appearance shows and changes the same layout choice as
    /// the editor's toolbar, so the two never disagree.
    @Test func sharesTheEditorLayoutChoice() throws {
        let defaults = UserDefaults.isolated
        let settings = SettingsViewModel(installer: FakeInstaller(.notSet), defaults: defaults)
        let editor = try ConflictFile().editor(defaults: defaults)
        #expect(settings.editorLayout == .columns)

        settings.editorLayout = .stacked
        editor.reloadLayout()
        #expect(editor.layout == .stacked)

        editor.switchLayout()
        settings.refresh()
        #expect(settings.editorLayout == .columns)
    }
}
