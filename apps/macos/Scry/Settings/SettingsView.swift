import SwiftUI

/// Scry › Settings. The CLI tab: the command-line tool inside the app,
/// and making it git's merge tool.
struct SettingsView: View {
    @State private var viewModel: SettingsViewModel

    init(viewModel: SettingsViewModel = SettingsViewModel(installer: AppMergetoolInstaller())) {
        _viewModel = State(initialValue: viewModel)
    }

    var body: some View {
        TabView {
            Tab("CLI", systemImage: "terminal") {
                cli
            }
        }
        .frame(width: 520)
        .onAppear { viewModel.refresh() }
    }

    private var cli: some View {
        Form {
            Section {
                Label {
                    Text(viewModel.status)
                } icon: {
                    Image(systemName: viewModel.isInstalled ? "checkmark.circle.fill" : "arrow.triangle.merge")
                        .foregroundStyle(viewModel.isInstalled ? Theme.States.resolved : Theme.Surfaces.textSecondary)
                }
                if !viewModel.isInstalled {
                    Button(viewModel.buttonTitle) { viewModel.install() }
                        .disabled(!viewModel.canInstall)
                }
                if let error = viewModel.error {
                    Text(error)
                        .font(Metrics.callout)
                        .foregroundStyle(Theme.Severity.error)
                }
            } header: {
                Text("Merge Tool")
            } footer: {
                Text("Sets merge.tool, mergetool.scry.cmd and mergetool.scry.trustExitCode in your global git configuration.")
                    .font(Metrics.caption)
                    .foregroundStyle(Theme.Surfaces.textSecondary)
            }
            Section("git-scry") {
                CodeLine(text: viewModel.cliPath)
            }
            Section {
                CodeLine(text: viewModel.command)
            } header: {
                Text("Merge tool command")
            } footer: {
                Text("What git runs for each conflicted file once Scry is its merge tool.")
                    .font(Metrics.caption)
                    .foregroundStyle(Theme.Surfaces.textSecondary)
            }
        }
        .formStyle(.grouped)
        .fixedSize(horizontal: false, vertical: true)
    }
}

/// A path or a command, in code type, to select and copy.
struct CodeLine: View {
    let text: String

    var body: some View {
        Text(text)
            .font(Metrics.codeSmall)
            .textSelection(.enabled)
            .frame(maxWidth: .infinity, alignment: .leading)
    }
}

/// Settings states for previews, without touching git's configuration.
private struct PreviewInstaller: MergetoolInstaller {
    var state: MergetoolSetup
    var isAvailable = true
    var cli = "/Applications/Scry.app/Contents/Helpers/git-scry"
    var command: String {
        "\"/Applications/Scry.app/Contents/Helpers/git-scry\" resolve --app=\"/Applications/Scry.app\" \"$BASE\" \"$LOCAL\" \"$REMOTE\" \"$MERGED\""
    }

    func setup() throws -> MergetoolSetup { state }
    func install() throws {}
}

#Preview("Settings, no merge tool") {
    SettingsView(viewModel: SettingsViewModel(installer: PreviewInstaller(state: .notSet)))
}

#Preview("Settings, another tool") {
    SettingsView(viewModel: SettingsViewModel(installer: PreviewInstaller(state: .other(tool: "opendiff"))))
}

#Preview("Settings, installed") {
    SettingsView(viewModel: SettingsViewModel(installer: PreviewInstaller(state: .scry(current: true))))
        .preferredColorScheme(.dark)
}

#Preview("Settings, app moved") {
    SettingsView(viewModel: SettingsViewModel(installer: PreviewInstaller(state: .scry(current: false))))
}
