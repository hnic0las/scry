import AppKit
import SwiftUI

/// Settings › Agent: Scry's MCP server in Claude Code, added for every
/// project with one button, or with the command to copy.
struct AgentSettingsView: View {
    let viewModel: AgentSettingsViewModel

    var body: some View {
        Form {
            Section {
                Label {
                    Text(viewModel.status)
                } icon: {
                    Image(systemName: viewModel.isRegistered ? "checkmark.circle.fill" : "sparkles")
                        .foregroundStyle(viewModel.isRegistered ? Theme.States.resolved : Theme.Surfaces.textSecondary)
                }
                if let command = viewModel.command {
                    HStack(alignment: .firstTextBaseline) {
                        CodeLine(text: command)
                        Button("Copy") { copy(command) }
                            .help("Copy the command, to run it in a terminal yourself")
                    }
                }
                HStack {
                    if viewModel.canRegister {
                        Button(viewModel.registerTitle) {
                            Task { await viewModel.register() }
                        }
                    }
                    if viewModel.canUnregister {
                        Button("Remove from Claude Code") {
                            Task { await viewModel.unregister() }
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
                Text("Adds Scry's MCP server to Claude Code once, for all your projects (claude mcp add --scope user). Then ask Claude Code \u{201c}what conflicts are left?\u{201d} in a repository with conflicts. The server only reads: nothing is staged, continued or committed by it.")
                    .font(Metrics.caption)
                    .foregroundStyle(Theme.Surfaces.textSecondary)
            }
        }
        .formStyle(.grouped)
        .fixedSize(horizontal: false, vertical: true)
        .onAppear { viewModel.refresh() }
    }

    private func copy(_ text: String) {
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(text, forType: .string)
    }
}

/// Claude Code's configuration in memory, for previews.
private struct PreviewRegistrar: AgentRegistrar {
    static let command = "claude mcp add --scope user scry -- /opt/homebrew/bin/git-scry mcp"
    var state: AgentSetupState
    func setup() async -> AgentSetup {
        AgentSetup(state: state, command: Self.command)
    }
    func register() async throws {}
    func unregister() async throws {}
}

private struct AgentPreview: View {
    @State private var viewModel: AgentSettingsViewModel

    init(_ state: AgentSetupState) {
        let registrar = PreviewRegistrar(state: state)
        _viewModel = State(initialValue: AgentSettingsViewModel(
            registrar: registrar,
            setup: AgentSetup(state: state, command: PreviewRegistrar.command)
        ))
    }

    var body: some View {
        AgentSettingsView(viewModel: viewModel)
            .frame(width: 520)
    }
}

#Preview("Agent, not added") {
    AgentPreview(.notRegistered)
}

#Preview("Agent, added") {
    AgentPreview(.registered)
        .preferredColorScheme(.dark)
}

#Preview("Agent, another Scry") {
    AgentPreview(.other(command: "/Users/me/workspace/scry/target/debug/git-scry mcp"))
}

#Preview("Agent, no Claude Code") {
    AgentPreview(.noClaude)
}
