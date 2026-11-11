import SwiftUI

/// Scry › Settings: the merge tool.
struct SettingsView: View {
    @State private var viewModel: SettingsViewModel

    init(viewModel: SettingsViewModel = SettingsViewModel(installer: AppMergetoolInstaller())) {
        _viewModel = State(initialValue: viewModel)
    }

    var body: some View {
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
                VStack(alignment: .leading, spacing: 6) {
                    Text("Sets merge.tool, mergetool.scry.cmd and mergetool.scry.trustExitCode in your global git configuration. git runs:")
                    Text(viewModel.command)
                        .font(Metrics.codeSmall)
                        .textSelection(.enabled)
                }
                .font(Metrics.caption)
                .foregroundStyle(Theme.Surfaces.textSecondary)
            }
        }
        .formStyle(.grouped)
        .frame(width: 520)
        .fixedSize(horizontal: false, vertical: true)
        .onAppear { viewModel.refresh() }
    }
}

/// Settings states for previews, without touching git's configuration.
private struct PreviewInstaller: MergetoolInstaller {
    var state: MergetoolSetup
    var isAvailable = true
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
