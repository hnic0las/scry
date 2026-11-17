import SwiftUI

/// Scry › Settings. The CLI tab: the `git scry` command, linked to the
/// command-line tool inside the app, and making it git's merge tool. The
/// Agent tab: Scry's MCP server in Claude Code.
struct SettingsView: View {
    @State private var viewModel: SettingsViewModel
    @State private var agent: AgentSettingsViewModel
    @State private var tab: SettingsTab

    enum SettingsTab: Hashable {
        case cli, agent, appearance
    }

    init(
        viewModel: SettingsViewModel = SettingsViewModel(installer: AppMergetoolInstaller()),
        agent: AgentSettingsViewModel = AgentSettingsViewModel(registrar: AppAgentRegistrar()),
        tab: SettingsTab = .cli
    ) {
        _viewModel = State(initialValue: viewModel)
        _agent = State(initialValue: agent)
        _tab = State(initialValue: tab)
    }

    var body: some View {
        TabView(selection: $tab) {
            Tab("CLI", systemImage: "terminal", value: .cli) {
                cli
            }
            Tab("Agent", systemImage: "sparkles", value: .agent) {
                AgentSettingsView(viewModel: agent)
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

    /// The `git scry` command: whether the terminal finds the app's
    /// git-scry, and the button that links it.
    private var command: some View {
        Section {
            Label {
                Text(viewModel.commandStatus)
            } icon: {
                Image(systemName: viewModel.isCommandInstalled ? "checkmark.circle.fill" : "terminal")
                    .foregroundStyle(viewModel.isCommandInstalled ? Theme.States.resolved : Theme.Surfaces.textSecondary)
            }
            ForEach(viewModel.commandsToRun, id: \.self) { line in
                CodeLine(text: line)
            }
            if viewModel.canInstallCommand {
                VStack(alignment: .leading, spacing: 4) {
                    Button(viewModel.commandButtonTitle) {
                        Task { await viewModel.installCommand() }
                    }
                    if let detail = viewModel.commandDetail {
                        Text(detail)
                            .font(Metrics.caption)
                            .foregroundStyle(Theme.Surfaces.textSecondary)
                    }
                }
            }
            if let error = viewModel.commandError {
                Text(error)
                    .font(Metrics.callout)
                    .foregroundStyle(Theme.Severity.error)
            }
            CodeLine(text: viewModel.cliPath)
        } header: {
            Text("git scry Command")
        } footer: {
            Text("git runs git scry by finding git-scry on your terminal's PATH. The link points at the git-scry inside the app, so it follows the app's updates.")
                .font(Metrics.caption)
                .foregroundStyle(Theme.Surfaces.textSecondary)
        }
    }

    private var cli: some View {
        Form {
            command
            Section {
                Label {
                    Text(viewModel.status)
                } icon: {
                    Image(systemName: viewModel.isInstalled ? "checkmark.circle.fill" : "arrow.triangle.merge")
                        .foregroundStyle(viewModel.isInstalled ? Theme.States.resolved : Theme.Surfaces.textSecondary)
                }
                HStack {
                    if !viewModel.isInstalled {
                        Button(viewModel.buttonTitle) { viewModel.install() }
                            .disabled(!viewModel.canInstall)
                    }
                    if viewModel.canUninstall {
                        Button("Stop Using Scry") { viewModel.uninstall() }
                    }
                }
                if let error = viewModel.error {
                    Text(error)
                        .font(Metrics.callout)
                        .foregroundStyle(Theme.Severity.error)
                }
            } header: {
                Text("Merge Tool")
            } footer: {
                Text("Sets merge.tool, mergetool.scry.cmd and mergetool.scry.trustExitCode in your global git configuration, and keeps the merge tool it replaces to put it back.")
                    .font(Metrics.caption)
                    .foregroundStyle(Theme.Surfaces.textSecondary)
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
        "\"/Applications/Scry.app/Contents/Helpers/git-scry\" resolve --app=\"/Applications/Scry.app\" -- \"$BASE\" \"$LOCAL\" \"$REMOTE\" \"$MERGED\""
    }

    func setup() throws -> MergetoolSetup { state }
    func install() throws {}
    func previousTool() throws -> String? { "opendiff" }
    func uninstall() throws {}
    var commandState: CliCommandState = .notInstalled(link: "/Users/me/.local/bin/git-scry")
    func cliCommandState() async -> CliCommandState { commandState }
    func installCommand(link: String) throws {}
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

#Preview("Settings, git scry installed") {
    SettingsView(viewModel: SettingsViewModel(
        installer: PreviewInstaller(
            state: .scry(current: true),
            commandState: .installed(path: "/opt/homebrew/bin/git-scry")
        ),
        defaults: .preview
    ))
}

#Preview("Settings, another git-scry first") {
    SettingsView(viewModel: SettingsViewModel(
        installer: PreviewInstaller(
            state: .notSet,
            commandState: .blocked(other: "/Users/me/.cargo/bin/git-scry", remove: "cargo uninstall git-scry")
        ),
        defaults: .preview
    ))
}

#Preview("Settings, no folder on the PATH") {
    SettingsView(viewModel: SettingsViewModel(
        installer: PreviewInstaller(state: .notSet, commandState: .noFolder),
        defaults: .preview
    ))
}

#Preview("Settings, appearance") {
    SettingsView(
        viewModel: SettingsViewModel(installer: PreviewInstaller(state: .notSet), defaults: .preview),
        tab: .appearance
    )
}
