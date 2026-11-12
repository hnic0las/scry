import SwiftUI

/// Scry › Settings. The CLI tab: the command-line tool inside the app,
/// and making it git's merge tool.
struct SettingsView: View {
    @State private var viewModel: SettingsViewModel
    @State private var tab: SettingsTab

    enum SettingsTab: Hashable {
        case cli, appearance
    }

    init(
        viewModel: SettingsViewModel = SettingsViewModel(installer: AppMergetoolInstaller()),
        tab: SettingsTab = .cli
    ) {
        _viewModel = State(initialValue: viewModel)
        _tab = State(initialValue: tab)
    }

    var body: some View {
        TabView(selection: $tab) {
            Tab("CLI", systemImage: "terminal", value: .cli) {
                cli
            }
            Tab("Appearance", systemImage: "rectangle.split.3x1", value: .appearance) {
                appearance
            }
        }
        .frame(width: 520)
        .onAppear { viewModel.refresh() }
    }

    private var appearance: some View {
        Form {
            Section {
                Picker("Merge editor layout", selection: $viewModel.editorLayout) {
                    Text("Three columns").tag(MergeEditorViewModel.EditorLayout.columns)
                    Text("Stacked: sides above the result").tag(MergeEditorViewModel.EditorLayout.stacked)
                }
                .pickerStyle(.radioGroup)
            } footer: {
                Text("Also in the editor's toolbar and View ▸ Layout (\(Shortcut.switchLayout.keys)). Windows narrower than 1,100 pt are always stacked.")
                    .font(Metrics.caption)
                    .foregroundStyle(Theme.Surfaces.textSecondary)
            }
        }
        .formStyle(.grouped)
        .fixedSize(horizontal: false, vertical: true)
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
    SettingsView(viewModel: SettingsViewModel(installer: PreviewInstaller(state: .notSet), defaults: .preview))
}

#Preview("Settings, another tool") {
    SettingsView(viewModel: SettingsViewModel(installer: PreviewInstaller(state: .other(tool: "opendiff")), defaults: .preview))
}

#Preview("Settings, installed") {
    SettingsView(viewModel: SettingsViewModel(installer: PreviewInstaller(state: .scry(current: true)), defaults: .preview))
        .preferredColorScheme(.dark)
}

#Preview("Settings, app moved") {
    SettingsView(viewModel: SettingsViewModel(installer: PreviewInstaller(state: .scry(current: false)), defaults: .preview))
}

#Preview("Settings, appearance") {
    SettingsView(
        viewModel: SettingsViewModel(installer: PreviewInstaller(state: .notSet), defaults: .preview),
        tab: .appearance
    )
}
