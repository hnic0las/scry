import AppKit
import SwiftUI

/// Settings › Agent: Scry's plugin in Claude Code, installed for every
/// project with one button, or with the commands to copy.
struct AgentSettingsView: View {
    @Bindable var viewModel: AgentSettingsViewModel

    var body: some View {
        Form {
            Section {
                Label {
                    Text(viewModel.status)
                } icon: {
                    Image(systemName: viewModel.isInstalled ? "checkmark.circle.fill" : "sparkles")
                        .foregroundStyle(viewModel.isInstalled ? Theme.States.resolved : Theme.Surfaces.textSecondary)
                }
                if let command = viewModel.command {
                    HStack(alignment: .firstTextBaseline) {
                        CodeLine(text: command)
                        Button("Copy") { copy(command) }
                            .help("Copy the commands, to run them in a terminal yourself")
                    }
                }
                if let missing = viewModel.missingScry {
                    warning(missing)
                }
                if let old = viewModel.oldServerWarning {
                    warning(old)
                }
                if let tools = viewModel.toolsWarning {
                    warning(tools)
                }
                HStack {
                    if viewModel.canInstall {
                        Button(viewModel.installTitle) {
                            Task { await viewModel.install() }
                        }
                    }
                    if viewModel.canRemoveOldServer && !viewModel.canInstall {
                        Button("Remove the Earlier Server") {
                            Task { await viewModel.removeOldServer() }
                        }
                    }
                    if viewModel.canAllowTools {
                        Button("Allow Scry's Tools") {
                            Task { await viewModel.allowTools() }
                        }
                    }
                    if viewModel.canUninstall {
                        Button("Uninstall from Claude Code") {
                            Task { await viewModel.uninstall() }
                        }
                    }
                    if viewModel.isWorking {
                        ProgressView().controlSize(.small)
                    }
                }
                if let error = viewModel.error {
                    Text(error)
                        .font(Metrics.callout)
                        .foregroundStyle(Theme.Severity.error)
                }
            } header: {
                Text("Claude Code")
            } footer: {
                Text("Installs Scry's git-scry plugin in Claude Code once, for all your projects: the server that reads conflicts and writes the resolutions you ask for, /git-scry:resolve, and the agent that answers Explain in the background of your session. Allows its read tools and Explain in Claude Code's settings; resolving still asks. Restart Claude Code sessions to load it. Nothing is staged, continued or committed by it.")
                    .font(Metrics.caption)
                    .foregroundStyle(Theme.Surfaces.textSecondary)
            }
            Section {
                Toggle("Explain conflicts with Claude Code", isOn: $viewModel.explainEnabled)
            } header: {
                Text("Explain")
            } footer: {
                Text(viewModel.explainDetail)
                    .font(Metrics.caption)
                    .foregroundStyle(Theme.Surfaces.textSecondary)
            }
        }
        .formStyle(.grouped)
        .fixedSize(horizontal: false, vertical: true)
        .onAppear { viewModel.refresh() }
    }

    private func warning(_ text: String) -> some View {
        Label {
            Text(text)
                .font(Metrics.callout)
        } icon: {
            Image(systemName: "exclamationmark.triangle.fill")
                .foregroundStyle(Theme.Severity.warning)
        }
    }

    private func copy(_ text: String) {
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(text, forType: .string)
    }
}

/// Claude Code's configuration in memory, for previews.
private struct PreviewRegistrar: AgentRegistrar {
    static let command = "claude plugin marketplace add hnic0las/scry && claude plugin install git-scry@scry"
    var current: AgentSetup
    func setup() async -> AgentSetup { current }
    func install() async throws {}
    func uninstall() async throws {}
    func removeOldServer() async throws {}
    func allowTools() async throws {}
}

private struct AgentPreview: View {
    @State private var viewModel: AgentSettingsViewModel

    init(_ state: AgentSetupState, oldServer: Bool = false, finds: Bool = true, allowed: Bool = true) {
        let setup = AgentSetup(
            state: state,
            command: PreviewRegistrar.command,
            oldServer: oldServer,
            pluginScry: finds ? "/Applications/Scry.app/Contents/Helpers/git-scry" : nil,
            toolsAllowed: allowed
        )
        _viewModel = State(initialValue: AgentSettingsViewModel(
            registrar: PreviewRegistrar(current: setup),
            setup: setup,
            defaults: .preview
        ))
    }

    var body: some View {
        AgentSettingsView(viewModel: viewModel)
            .frame(width: 520)
    }
}

#Preview("Agent, not installed") {
    AgentPreview(.notInstalled)
}

#Preview("Agent, installed") {
    AgentPreview(.installed)
        .preferredColorScheme(.dark)
}

#Preview("Agent, earlier server left") {
    AgentPreview(.installed, oldServer: true)
}

#Preview("Agent, tools not allowed") {
    AgentPreview(.installed, allowed: false)
}

#Preview("Agent, no Scry for the plugin") {
    AgentPreview(.notInstalled, finds: false)
}

#Preview("Agent, no Claude Code") {
    AgentPreview(.noClaude)
}
