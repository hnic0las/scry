import AppKit
import SwiftUI

/// Settings › Agent: Scry's plugin in Claude Code, installed for every
/// project with one button or with the commands to copy; who watches the
/// repository open in Scry; Explain.
struct AgentSettingsView: View {
    @Bindable var viewModel: AgentSettingsViewModel

    var body: some View {
        Form {
            Section("Claude Code") {
                claudeCode
            }
            Section {
                watching
            } header: {
                Text("Watching now")
            } footer: {
                note(viewModel.watchingNote)
            }
            .foregroundStyle(viewModel.pluginOn ? Theme.Surfaces.text : Theme.Surfaces.textTertiary)
            Section {
                Toggle("Explain and suggest with Claude Code", isOn: $viewModel.explainEnabled)
                    .disabled(!viewModel.pluginOn)
            } header: {
                Text("Explain")
            } footer: {
                note(viewModel.explainDetail)
            }
            Section {
                Label {
                    Text("The plugin lets a Claude Code session opened in a repository see Scry's conflicts, resolve them for your review, and answer Explain. Scry never starts Claude Code and sends nothing itself: what Claude reads goes through your own Claude Code session, with your account and its settings.")
                        .font(Metrics.caption)
                        .foregroundStyle(Theme.Surfaces.textSecondary)
                } icon: {
                    Image(systemName: "info.circle").foregroundStyle(Theme.Surfaces.textSecondary)
                }
            }
        }
        .formStyle(.grouped)
        .fixedSize(horizontal: false, vertical: true)
        .onAppear { viewModel.refresh() }
        // Claude Code installed or changed in a terminal meanwhile.
        .onReceive(NotificationCenter.default.publisher(for: NSApplication.didBecomeActiveNotification)) { _ in
            viewModel.refresh()
        }
        .task {
            while !Task.isCancelled {
                await viewModel.refreshWatch()
                try? await Task.sleep(for: .seconds(2))
            }
        }
        .confirmationDialog(
            "Uninstall Scry's plugin from Claude Code?",
            isPresented: $viewModel.confirmingUninstall
        ) {
            Button("Uninstall", role: .destructive) {
                Task { await viewModel.uninstall() }
            }
        } message: {
            Text("Claude Code sessions no longer see Scry's conflicts or answer Explain once restarted. Scry's marketplace stays in Claude Code.")
        }
    }

    @ViewBuilder
    private var claudeCode: some View {
        let headline = viewModel.headline
        VStack(alignment: .leading, spacing: 6) {
            Label {
                Text(headline.title).font(Metrics.headline)
            } icon: {
                glyph(headline.glyph)
            }
            if let detail = headline.detail {
                note(detail)
            }
        }
        if let failure = viewModel.failure, !viewModel.isWorking {
            OutputBox(text: failure.output, strip: Theme.Severity.error)
            HStack {
                Button("Try Again") { Task { await viewModel.tryAgain() } }
                    .buttonStyle(.borderedProminent)
                Button("Copy Error") { copy(failure.output) }
            }
        }
        if let callout = viewModel.missingScry {
            CalloutView(callout: callout)
        }
        if let callout = viewModel.oldServerWarning {
            CalloutView(callout: callout) {
                if viewModel.setup?.state != .notInstalled {
                    Button("Remove the Earlier Server") { Task { await viewModel.removeOldServer() } }
                        .disabled(!viewModel.canRemoveOldServer)
                }
            }
        }
        if let callout = viewModel.toolsWarning {
            CalloutView(callout: callout) {
                Button("Allow Scry's Tools") { Task { await viewModel.allowTools() } }
                    .disabled(!viewModel.canAllowTools)
            }
        }
        if viewModel.canCheckAgain || viewModel.canInstall || viewModel.canUninstall
            || (viewModel.isWorking && viewModel.setup?.state != .installed) {
            HStack {
                if viewModel.setup?.state == .noClaude {
                    Button("Check Again") { viewModel.refresh() }
                        .disabled(!viewModel.canCheckAgain)
                }
                if viewModel.setup?.state == .notInstalled || viewModel.setup?.state == .disabled {
                    Button(viewModel.installTitle) { Task { await viewModel.install() } }
                        .buttonStyle(.borderedProminent)
                        .disabled(!viewModel.canInstall)
                }
                if viewModel.setup?.state == .installed || viewModel.setup?.state == .disabled {
                    Button("Uninstall from Claude Code") { viewModel.confirmingUninstall = true }
                        .disabled(!viewModel.canUninstall)
                }
            }
        }
        if let command = viewModel.command {
            VStack(alignment: .leading, spacing: 6) {
                note(viewModel.commandIntro)
                HStack(alignment: .top) {
                    OutputBox(text: command, strip: nil)
                    Button {
                        copy(command)
                    } label: {
                        Label("Copy", systemImage: "doc.on.doc")
                    }
                    .help("Copy the commands, to run them in a terminal yourself")
                }
            }
        }
    }

    @ViewBuilder
    private var watching: some View {
        let watching = viewModel.watching
        HStack(spacing: 8) {
            PresenceDot(watching: viewModel.pluginOn && watching.watching)
            if !viewModel.pluginOn {
                Text("Not available yet")
            } else if let name = watching.name {
                Text(name).font(Metrics.headline)
                Text(watching.detail)
                    .font(Metrics.callout)
                    .foregroundStyle(Theme.Surfaces.textSecondary)
            } else {
                Text(watching.detail)
            }
        }
    }

    @ViewBuilder
    private func glyph(_ glyph: AgentSettingsViewModel.Glyph) -> some View {
        switch glyph {
        case .looking, .working:
            ProgressView().controlSize(.small)
        case .info:
            Image(systemName: "info.circle").foregroundStyle(Theme.Surfaces.textSecondary)
        case .failed:
            Image(systemName: "xmark.circle").foregroundStyle(Theme.Severity.error)
        case .off:
            Image(systemName: "pause.circle").foregroundStyle(Theme.Surfaces.textSecondary)
        case .installed:
            Image(systemName: "checkmark.circle").foregroundStyle(Theme.States.resolved)
        case .warning:
            Image(systemName: "exclamationmark.triangle").foregroundStyle(Theme.Severity.warning)
        }
    }

    private func note(_ text: String) -> some View {
        Text(text)
            .font(Metrics.caption)
            .foregroundStyle(Theme.Surfaces.textSecondary)
            .fixedSize(horizontal: false, vertical: true)
    }

    private func copy(_ text: String) {
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(text, forType: .string)
    }
}

/// Text from a terminal, in mono, as received: the commands to run, or
/// what `claude` answered, with the error strip on its left edge.
private struct OutputBox: View {
    let text: String
    let strip: Color?

    var body: some View {
        Text(text)
            .font(Metrics.codeSmall)
            .textSelection(.enabled)
            .fixedSize(horizontal: false, vertical: true)
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(8)
            .padding(.leading, strip == nil ? 0 : 3)
            .background(
                RoundedRectangle(cornerRadius: Metrics.cardCornerRadius)
                    .fill(Theme.Surfaces.pane)
                    .stroke(Theme.Surfaces.hairline)
            )
            .overlay(alignment: .leading) {
                if let strip {
                    UnevenRoundedRectangle(topLeadingRadius: Metrics.cardCornerRadius, bottomLeadingRadius: Metrics.cardCornerRadius)
                        .fill(strip)
                        .frame(width: 3)
                }
            }
    }
}

/// A warning in a faint tint of the warning colour: a title, what to do,
/// and the button that does it.
private struct CalloutView<Actions: View>: View {
    let callout: AgentSettingsViewModel.Callout
    @ViewBuilder var actions: Actions

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            Label {
                Text(callout.title).font(Metrics.headline)
            } icon: {
                Image(systemName: "exclamationmark.triangle").foregroundStyle(Theme.Severity.warning)
            }
            Text(callout.text)
                .font(Metrics.callout)
                .fixedSize(horizontal: false, vertical: true)
            actions
        }
        .padding(10)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(
            RoundedRectangle(cornerRadius: Metrics.cardCornerRadius)
                .fill(Theme.Severity.warning.opacity(0.08))
        )
    }
}

extension CalloutView where Actions == EmptyView {
    init(callout: AgentSettingsViewModel.Callout) {
        self.init(callout: callout) { EmptyView() }
    }
}

/// Claude Code's configuration in memory, for previews.
private struct PreviewRegistrar: AgentRegistrar {
    static let command = "claude plugin marketplace add hnic0las/scry\nclaude plugin install git-scry@scry"
    var current: AgentSetup
    func setup() async -> AgentSetup { current }
    func install() async throws {}
    func uninstall() async throws {}
    func removeOldServer() async throws {}
    func allowTools() async throws {}
    func watch(path: String) async -> RepositoryWatch? {
        RepositoryWatch(name: "offline-sync", sessions: [UInt64(Date().addingTimeInterval(-3600).timeIntervalSince1970)])
    }
}

private struct AgentPreview: View {
    @State private var viewModel: AgentSettingsViewModel

    init(_ state: AgentSetupState, oldServer: Bool = false, finds: Bool = true, allowed: Bool = true) {
        let setup = AgentSetup(
            state: state,
            command: PreviewRegistrar.command,
            oldServer: oldServer,
            pluginScry: finds ? "/Applications/Scry.app/Contents/Helpers/git-scry" : nil,
            toolsAllowed: allowed,
            pluginVersion: state == .installed || state == .disabled ? "0.3.0" : nil
        )
        _viewModel = State(initialValue: AgentSettingsViewModel(
            registrar: PreviewRegistrar(current: setup),
            setup: setup,
            defaults: .preview,
            repository: { "/Users/me/offline-sync" }
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
