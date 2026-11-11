import SwiftUI

/// S3: the focused conflict, its classification and its candidates.
struct InspectorView: View {
    let viewModel: InspectorViewModel

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 12) {
                if let conflict = viewModel.conflict {
                    content(conflict)
                } else {
                    Text("No conflict in this file.")
                        .foregroundStyle(Theme.Surfaces.textSecondary)
                }
            }
            .padding(12)
        }
        // Stops under the toolbar: by default a background extends into the
        // title bar area and the column would look like it starts at the top
        // of the window.
        .background(Theme.Surfaces.sidebar, ignoresSafeAreaEdges: [])
    }

    @ViewBuilder
    private func content(_ conflict: ConflictInfo) -> some View {
        HStack(alignment: .firstTextBaseline) {
            Text(viewModel.title)
                .font(Metrics.headline)
            Text(viewModel.line)
                .font(Metrics.callout)
                .foregroundStyle(Theme.Surfaces.textSecondary)
            Spacer()
            Text(conflict.id)
                .font(Metrics.codeSmall)
                .foregroundStyle(Theme.Surfaces.textTertiary)
                .textSelection(.enabled)
        }

        Label {
            Text(conflict.classification).font(Metrics.headline)
        } icon: {
            Image(systemName: "ruler").foregroundStyle(Theme.Provenance.rule)
        }
        .padding(10)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(card)

        Text("Candidates")
            .font(Metrics.captionBold)
            .foregroundStyle(Theme.Surfaces.textSecondary)

        ForEach(viewModel.cards) { card in
            CandidateCard(candidate: card.candidate, selected: card.selected, language: viewModel.language) {
                viewModel.choose(card.candidate.id)
            }
            .disabled(!viewModel.canChoose)
        }

        if let edited = viewModel.editedCard {
            CandidateCard(candidate: edited.candidate, selected: true, language: viewModel.language) {
                viewModel.editByHand()
            }
        }
        Button { viewModel.editByHand() } label: {
            Label("Edit by Hand", systemImage: "pencil")
        }
        .help("Write this conflict's result yourself (↩)")
        .disabled(!viewModel.canEditByHand)
    }

    private var card: some View {
        RoundedRectangle(cornerRadius: Metrics.cardCornerRadius)
            .fill(Theme.Surfaces.pane)
            .stroke(Theme.Surfaces.hairline)
    }
}

struct CandidateCard: View {
    let candidate: CandidateInfo
    let selected: Bool
    let language: String?
    let choose: () -> Void

    var body: some View {
        Button(action: choose) {
            VStack(alignment: .leading, spacing: 8) {
                HStack(alignment: .top, spacing: 8) {
                    Image(systemName: selected ? "largecircle.fill.circle" : "circle")
                        .foregroundStyle(selected ? accent : Theme.Surfaces.textTertiary)
                    VStack(alignment: .leading, spacing: 2) {
                        Text(candidate.title).font(Metrics.headline)
                        Text(candidate.subtitle)
                            .font(Metrics.caption)
                            .foregroundStyle(Theme.Surfaces.textSecondary)
                            .lineLimit(2)
                    }
                    Spacer(minLength: 4)
                    provenance
                }
                preview
                verification
            }
            .padding(10)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(
                RoundedRectangle(cornerRadius: Metrics.cardCornerRadius)
                    .fill(Theme.Surfaces.pane)
                    .stroke(selected ? accent : Theme.Surfaces.hairline, lineWidth: selected ? 2 : 1)
            )
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityLabel("\(candidate.title), \(candidate.subtitle)")
        .accessibilityAddTraits(selected ? .isSelected : [])
    }

    private var accent: Color {
        candidate.doesNotParse ? Theme.Severity.error : Theme.Interaction.focus
    }

    @ViewBuilder
    private var provenance: some View {
        if candidate.suggested {
            Badge(text: "suggested", symbol: "ruler", color: Theme.Provenance.rule)
        } else {
            let provenance = candidate.provenance
            Badge(text: provenance.name, symbol: provenance.symbol, color: provenance.color)
        }
    }

    private var preview: some View {
        VStack(alignment: .leading, spacing: 0) {
            if candidate.preview.isEmpty {
                Text("(nothing)")
                    .italic()
                    .foregroundStyle(Theme.Surfaces.textTertiary)
            }
            ForEach(Array(candidate.preview.enumerated()), id: \.offset) { _, line in
                Text(line).lineLimit(1).truncationMode(.tail)
            }
            if candidate.truncated {
                Text("…").foregroundStyle(Theme.Surfaces.textTertiary)
            }
        }
        .font(Metrics.codeSmall)
        .foregroundStyle(Theme.Surfaces.text)
        .padding(8)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(RoundedRectangle(cornerRadius: 5).fill(Theme.Surfaces.sidebar))
    }

    @ViewBuilder
    private var verification: some View {
        switch candidate.parse {
        case .parses:
            Label("Parses as \(language ?? "code")", systemImage: "checkmark")
                .font(Metrics.caption)
                .foregroundStyle(Theme.States.resolved)
        case .doesNotParse(let line):
            Label("Does not parse near line \(line) · still selectable, then edit by hand", systemImage: "xmark")
                .font(Metrics.caption)
                .foregroundStyle(Theme.Severity.error)
        case .notChecked:
            EmptyView()
        }
    }
}

/// A provenance as the badges and the result's strip show it.
extension Provenance {
    var name: String {
        switch self {
        case .git: "git"
        case .rule: "rule"
        case .structural: "structure"
        case .analyzer: "analyzer"
        case .agent: "agent"
        case .human: "you"
        }
    }

    var symbol: String {
        switch self {
        case .git: "arrow.triangle.branch"
        case .rule: "ruler"
        case .structural: "square.stack.3d.up"
        case .analyzer: "magnifyingglass"
        case .agent: "sparkles"
        case .human: "person.fill"
        }
    }

    var color: Color {
        switch self {
        case .git: Theme.Provenance.git
        case .rule: Theme.Provenance.rule
        case .structural: Theme.Provenance.structural
        case .analyzer: Theme.Provenance.analyzer
        case .agent: Theme.Provenance.agent
        case .human: Theme.Provenance.human
        }
    }
}

#Preview("Inspector, unresolved", traits: .fixedLayout(width: 320, height: 760)) {
    if let editor = PreviewSample.editor() {
        InspectorView(viewModel: editor.inspector)
    }
}

#Preview("Inspector, resolved by a side", traits: .fixedLayout(width: 320, height: 760)) {
    if let editor = PreviewSample.editor({ $0.take("theirs") }) {
        InspectorView(viewModel: editor.inspector)
    }
}

#Preview("Inspector, edited by hand", traits: .fixedLayout(width: 320, height: 1150)) {
    if let editor = PreviewSample.editor({ editor in
        editor.startEditing(1)
        editor.editBlock?.draft = "    queue.cancelAll()\n    queue.suspend()\n"
        editor.editBlock?.apply()
    }) {
        InspectorView(viewModel: editor.inspector)
    }
}

#Preview("Inspector, dark", traits: .fixedLayout(width: 320, height: 760)) {
    if let editor = PreviewSample.editor({ $0.take("ours") }) {
        InspectorView(viewModel: editor.inspector)
            .preferredColorScheme(.dark)
    }
}
