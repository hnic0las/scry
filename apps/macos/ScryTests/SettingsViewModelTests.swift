import Testing
@testable import Scry

/// Git's merge tool in memory: these tests must never write the user's
/// git configuration.
private final class FakeInstaller: MergetoolInstaller {
    var state: MergetoolSetup
    var isAvailable = true
    var failure: ScryError?
    private(set) var installs = 0

    init(_ state: MergetoolSetup) {
        self.state = state
    }

    var command: String { "\"/A/git-scry\" resolve --app=\"/A/Scry.app\" \"$BASE\" \"$LOCAL\" \"$REMOTE\" \"$MERGED\"" }

    func setup() throws -> MergetoolSetup { state }

    func install() throws {
        if let failure { throw failure }
        installs += 1
        state = .scry(current: true)
    }
}

@MainActor
struct SettingsViewModelTests {
    /// With another tool set, the settings say which one Scry replaces,
    /// and the button makes Scry the merge tool, after which it is no
    /// longer offered.
    @Test func replacesAnotherTool() {
        let installer = FakeInstaller(.other(tool: "opendiff"))
        let settings = SettingsViewModel(installer: installer)
        settings.refresh()

        #expect(settings.status.contains("opendiff"))
        #expect(settings.canInstall)
        #expect(settings.buttonTitle == "Use Scry as git's Merge Tool")

        settings.install()

        #expect(installer.installs == 1)
        #expect(settings.isInstalled)
        #expect(!settings.canInstall)
    }

    /// A Scry command for an app moved since is offered as an update.
    @Test func offersToUpdateAMovedApp() {
        let settings = SettingsViewModel(installer: FakeInstaller(.scry(current: false)))
        settings.refresh()

        #expect(!settings.isInstalled)
        #expect(settings.buttonTitle == "Update Merge Tool")
    }

    /// A build without its git-scry inside cannot install a command that
    /// would point at nothing.
    @Test func needsTheBundledCommandLineTool() {
        let installer = FakeInstaller(.notSet)
        installer.isAvailable = false
        let settings = SettingsViewModel(installer: installer)
        settings.refresh()

        settings.install()

        #expect(!settings.canInstall)
        #expect(installer.installs == 0)
        #expect(settings.status.contains("no git-scry"))
    }

    /// When git refuses the change, the settings say why and stay as they
    /// were.
    @Test func showsWhyInstallingFailed() {
        let installer = FakeInstaller(.notSet)
        installer.failure = .Failed(message: "could not lock config file")
        let settings = SettingsViewModel(installer: installer)
        settings.refresh()

        settings.install()

        #expect(settings.error?.contains("could not lock") == true)
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
}
