import SwiftUI

/// S3: the focused conflict, its classification and its candidates.
struct InspectorView: View {
    let viewModel: InspectorViewModel
    /// The candidate list has the keyboard: ↑ / ↓ move the choice.
    @FocusState private var candidatesFocused: Bool

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

        if let review = viewModel.review {
            ReviewCard(
                review: review,
                canMarkReviewed: viewModel.canMarkReviewed,
                canReopen: viewModel.canReopen,
                markReviewed: { viewModel.markReviewed() },
                reopen: { viewModel.reopen() }
            )
        }

        Text("Candidates")
            .font(Metrics.captionBold)
            .foregroundStyle(Theme.Surfaces.textSecondary)

        VStack(alignment: .leading, spacing: 12) {
            ForEach(viewModel.cards) { card in
                CandidateCard(
                    candidate: card.candidate,
                    selected: card.selected,
                    focused: candidatesFocused,
                    language: viewModel.language
                ) {
                    viewModel.choose(card.candidate.id)
                    candidatesFocused = true
                }
                .disabled(!viewModel.canChoose)
            }
        }
        // A radio group: one stop for Tab, then ↑ / ↓ inside it.
        .focusable(viewModel.canChoose)
        .focused($candidatesFocused)
        .focusEffectDisabled()
        // Plain arrows only: ⌥⌘↓ past the last conflict (its command then
        // disabled) must not fall through here and change the choice.
        .onKeyPress(keys: [.upArrow, .downArrow], phases: [.down, .repeat]) { press in
            guard press.modifiers.isDisjoint(with: Self.chordModifiers) else { return .ignored }
            viewModel.chooseAdjacent(press.key == .upArrow ? -1 : 1)
            return .handled
        }
        .accessibilityElement(children: .contain)
        .accessibilityLabel("Candidates")
        .accessibilityHint("Up and down arrows choose another candidate")

        if let edited = viewModel.editedCard {
            CandidateCard(candidate: edited.candidate, selected: true, focused: false, language: viewModel.language) {
                viewModel.editByHand()
            }
        }
        Button { viewModel.editByHand() } label: {
            Label("Edit by Hand", systemImage: "pencil")
        }
        .help("Write this conflict's result yourself (↩)")
        .disabled(!viewModel.canEditByHand)
    }

    /// Arrow keys typed with one of these belong to a shortcut, not to the candidates.
    /// `.numericPad` and `.function`, which AppKit sets on every arrow key, are left out.
    private static let chordModifiers: EventModifiers = [.command, .option, .control, .shift]

    private var card: some View {
        RoundedRectangle(cornerRadius: Metrics.cardCornerRadius)
            .fill(Theme.Surfaces.pane)
            .stroke(Theme.Surfaces.hairline)
    }
}

/// A conflict resolved outside Scry: who resolved it, why, and the
/// review's actions. Dashed until reviewed, as the result's strip.
struct ReviewCard: View {
    let review: InspectorViewModel.Review
    let canMarkReviewed: Bool
    let canReopen: Bool
    let markReviewed: () -> Void
    let reopen: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack(alignment: .firstTextBaseline) {
                Label(review.by, systemImage: review.provenance.symbol)
                    .font(Metrics.headline)
                    .foregroundStyle(review.provenance.color)
                Spacer(minLength: 4)
                Badge(
                    text: review.status,
                    symbol: review.reviewed ? "checkmark" : "eye",
                    color: review.reviewed ? Theme.States.resolved : Theme.States.needsReview
                )
                // Whole: the agent's name wraps instead.
                .fixedSize()
            }
            Text(review.reason)
                .font(Metrics.callout)
                .italic(!review.hasReason)
                .foregroundStyle(review.hasReason ? Theme.Surfaces.text : Theme.Surfaces.textSecondary)
                .textSelection(.enabled)
                .fixedSize(horizontal: false, vertical: true)
            if let dropped = review.dropped {
                Text(dropped)
                    .font(Metrics.caption)
                    .foregroundStyle(Theme.Surfaces.textSecondary)
                    .textSelection(.enabled)
                    .fixedSize(horizontal: false, vertical: true)
            }
            HStack {
                Button(action: markReviewed) {
                    Label("Mark Reviewed", systemImage: "checkmark")
                }
                .help("Keep this resolution: the file stages without asking once every conflict is reviewed")
                .disabled(!canMarkReviewed)
                Button(action: reopen) {
                    Label("Reopen", systemImage: "arrow.uturn.backward")
                }
                .help("Put the conflict's markers back, to resolve it here")
                .disabled(!canReopen)
            }
        }
        .padding(10)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(
            RoundedRectangle(cornerRadius: Metrics.cardCornerRadius)
                .fill(Theme.Surfaces.pane)
                .stroke(
                    review.provenance.color,
                    style: StrokeStyle(lineWidth: 1, dash: review.reviewed ? [] : [4, 3])
                )
        )
    }
}

struct CandidateCard: View {
    let candidate: CandidateInfo
    let selected: Bool
    /// The list has the keyboard: the selected card shows the focus ring.
    let focused: Bool
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
            .overlay {
                if selected && focused {
                    RoundedRectangle(cornerRadius: Metrics.cardCornerRadius + 3)
                        .stroke(accent.opacity(0.35), lineWidth: 3)
                        .padding(-3)
                }
            }
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityLabel("\(candidate.title), \(candidate.subtitle)")
        .accessibilityAddTraits(selected ? .isSelected : [])
        .accessibilityValue(selected ? "chosen" : "")
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

#Preview("Inspector, review · agent's own text", traits: .fixedLayout(width: 320, height: 900)) {
    if let editor = ReviewSample.editor({ $0.focus(1) }) {
        InspectorView(viewModel: editor.inspector)
    }
}

#Preview("Inspector, review · resolved outside Scry", traits: .fixedLayout(width: 320, height: 900)) {
    if let editor = ReviewSample.editor() {
        InspectorView(viewModel: editor.inspector)
    }
}

#Preview("Inspector, review · reviewed, dark", traits: .fixedLayout(width: 320, height: 900)) {
    if let editor = ReviewSample.editor({ editor in
        editor.focus(1)
        editor.inspector.markReviewed()
    }) {
        InspectorView(viewModel: editor.inspector)
            .preferredColorScheme(.dark)
    }
}
