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

        if let explain = viewModel.explain {
            ExplainBlock(
                explain: explain,
                expanded: viewModel.isExpanded,
                error: viewModel.explainError,
                ask: { viewModel.askExplain() },
                cancel: { viewModel.cancelExplain() },
                toggleExpanded: { viewModel.toggleExpanded() }
            )
        }

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

/// Explain: what the user's Claude Code said about the conflict, asked
/// from here and answered in the background of their session (SPEC:
/// "Explain (answered by the user's Claude Code)"). Read-only: nothing in
/// it changes the result.
struct ExplainBlock: View {
    let explain: InspectorViewModel.Explain
    let expanded: Bool
    let error: String?
    let ask: () -> Void
    let cancel: () -> Void
    let toggleExpanded: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            switch explain {
            case .notAsked(let watching):
                notAsked(watching: watching)
            case .waiting(let seconds, let previous):
                box(strip: .dashed) {
                    HStack {
                        ProgressView().controlSize(.small)
                        Text("Asked Claude Code…").font(Metrics.headline)
                        Spacer()
                        Text("\(seconds) s")
                            .font(Metrics.callout)
                            .monospacedDigit()
                            .foregroundStyle(Theme.Surfaces.textSecondary)
                    }
                    detail("A helper answers in the background of your session. Usually 5 to 20 seconds.")
                    Button("Cancel", action: cancel)
                        .keyboardShortcut(.cancelAction)
                }
                if let previous { answer(previous, quiet: true) }
            case .noSession(let previous):
                box(strip: nil) {
                    HStack(spacing: 6) {
                        PresenceDot(watching: false)
                        Text("Waiting for Claude Code").font(Metrics.headline)
                    }
                    detail("Your ask is queued. Open Claude Code in this repository and it is answered right away.")
                    HStack {
                        Button("Cancel", action: cancel)
                        SettingsLink {
                            Text("Plugin not set up? Settings › Agent…")
                        }
                        .buttonStyle(.link)
                    }
                }
                if let previous { answer(previous, quiet: true) }
            case .noAnswer(let previous):
                box(strip: nil) {
                    Label {
                        Text("No answer from Claude Code").font(Metrics.headline)
                    } icon: {
                        Image(systemName: "exclamationmark.triangle").foregroundStyle(Theme.Severity.warning)
                    }
                    detail("Nothing came back after a minute. Look at your Claude Code session: it may be busy or waiting for you.")
                    HStack {
                        askAgain
                        Button("Cancel", action: cancel)
                    }
                }
                if let previous { answer(previous, quiet: true) }
            case .answered(let explanation, _):
                answer(explanation, quiet: false)
            }
            if let error {
                Text(error)
                    .font(Metrics.callout)
                    .foregroundStyle(Theme.Severity.error)
            }
        }
        .accessibilityElement(children: .contain)
        .accessibilityLabel("Explanation")
    }

    private func notAsked(watching: Bool) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack {
                Button(action: ask) {
                    Label("Explain", systemImage: "sparkle")
                }
                .help(Shortcut.explain.help())
                Spacer()
                Presence(watching: watching)
            }
            detail("Claude Code explains what each side meant and the risk of each candidate. It never changes the result.")
        }
    }

    private func answer(_ explanation: Explanation, quiet: Bool) -> some View {
        box(strip: .solid) {
            Label {
                Text("Explained by \(explanation.by)").font(Metrics.captionBold)
            } icon: {
                Image(systemName: "sparkle").foregroundStyle(Theme.Provenance.agent)
            }
            Text(explanation.text)
                .font(Metrics.callout)
                .lineSpacing(5)
                .foregroundStyle(quiet ? Theme.Surfaces.textSecondary : Theme.Surfaces.text)
                .lineLimit(expanded || !Self.isLong(explanation.text) ? nil : 8)
                .fixedSize(horizontal: false, vertical: true)
                .textSelection(.enabled)
            HStack {
                Text(Self.relative(explanation.at))
                    .font(Metrics.caption)
                    .foregroundStyle(Theme.Surfaces.textSecondary)
                    .help(Self.exact(explanation.at))
                Spacer()
                if Self.isLong(explanation.text) {
                    Button(expanded ? "Show Less" : "Show More", action: toggleExpanded)
                        .buttonStyle(.link)
                }
                if !quiet { askAgain }
            }
        }
    }

    private var askAgain: some View {
        Button("Ask Again", action: ask)
            .help(Shortcut.explain.help())
    }

    private func detail(_ text: String) -> some View {
        Text(text)
            .font(Metrics.callout)
            .foregroundStyle(Theme.Surfaces.textSecondary)
            .fixedSize(horizontal: false, vertical: true)
    }

    private enum Strip { case solid, dashed }

    /// A box with the agent's provenance strip on its left edge: dashed
    /// while asked, solid once answered, none while nobody watches.
    private func box(strip: Strip?, @ViewBuilder content: () -> some View) -> some View {
        VStack(alignment: .leading, spacing: 6, content: content)
            .padding(10)
            .padding(.leading, strip == nil ? 0 : 3)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(
                RoundedRectangle(cornerRadius: Metrics.cardCornerRadius)
                    .fill(Theme.Surfaces.pane)
                    .stroke(Theme.Surfaces.hairline)
            )
            .overlay(alignment: .leading) {
                if let strip {
                    VerticalLine()
                        .stroke(
                            Theme.Provenance.agent,
                            style: StrokeStyle(lineWidth: 3, lineCap: .round, dash: strip == .dashed ? [4, 4] : [])
                        )
                        .frame(width: 3)
                        .padding(.vertical, 8)
                        .padding(.leading, 5)
                }
            }
    }

    /// More than eight lines in the inspector, about.
    static func isLong(_ text: String) -> Bool { text.count > 420 }

    static func relative(_ milliseconds: UInt64) -> String {
        let date = Date(timeIntervalSince1970: Double(milliseconds) / 1000)
        return RelativeDateTimeFormatter().localizedString(for: date, relativeTo: Date())
    }

    static func exact(_ milliseconds: UInt64) -> String {
        Date(timeIntervalSince1970: Double(milliseconds) / 1000)
            .formatted(date: .abbreviated, time: .standard)
    }
}

/// A line down the middle of its frame, for the provenance strip.
private struct VerticalLine: Shape {
    func path(in rect: CGRect) -> Path {
        Path { path in
            path.move(to: CGPoint(x: rect.midX, y: rect.minY))
            path.addLine(to: CGPoint(x: rect.midX, y: rect.maxY))
        }
    }
}

/// "Claude Code is here", or "is not here": whether a session watches the
/// repository, in the Explain block and the editor's status bar. The
/// words change, not only the dot; clicking opens Settings › Agent.
struct Presence: View {
    let watching: Bool

    var body: some View {
        SettingsLink {
            HStack(spacing: 5) {
                PresenceDot(watching: watching)
                Text(watching ? "Claude Code is here" : "Claude Code is not here")
                    .font(Metrics.caption)
                    .foregroundStyle(Theme.Surfaces.textSecondary)
            }
        }
        .buttonStyle(.plain)
        .help(watching
            ? "A Claude Code session is watching this repository"
            : "No Claude Code session is watching this repository. An ask waits until one opens.")
    }
}

/// Whether a Claude Code session watches the repository: filled in the
/// agent's colour, or hollow. Never a state colour (SPEC).
struct PresenceDot: View {
    let watching: Bool

    var body: some View {
        Circle()
            .fill(watching ? Theme.Provenance.agent : .clear)
            .stroke(watching ? Theme.Provenance.agent : Theme.Surfaces.textTertiary, lineWidth: 1.5)
            .frame(width: 7, height: 7)
            .accessibilityHidden(true)
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

private let previewExplanation = Explanation(
    text: "Ours (main) raised the request timeout from 30 s to 60 s for slow cellular links. Theirs (rename) renamed it to requestTimeout and kept 30 s; line 17 already reads requestTimeout, so ours alone would not build. Keep theirs, then set it to 60 by hand.",
    by: "claude-code 2.1.331",
    at: UInt64(Date().addingTimeInterval(-120).timeIntervalSince1970 * 1000)
)

private struct ExplainPreview: View {
    let states: [InspectorViewModel.Explain]

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            ForEach(Array(states.enumerated()), id: \.offset) { _, state in
                ExplainBlock(explain: state, expanded: false, error: nil, ask: {}, cancel: {}, toggleExpanded: {})
            }
        }
        .padding(12)
        .frame(width: 320)
        .background(Theme.Surfaces.sidebar)
    }
}

#Preview("Explain states", traits: .fixedLayout(width: 320, height: 1000)) {
    ExplainPreview(states: [
        .notAsked(watching: true),
        .waiting(seconds: 7, previous: nil),
        .noSession(previous: nil),
        .noAnswer(previous: nil),
        .answered(previewExplanation, watching: true),
    ])
}

#Preview("Explain states, dark", traits: .fixedLayout(width: 320, height: 1000)) {
    ExplainPreview(states: [
        .notAsked(watching: false),
        .waiting(seconds: 12, previous: previewExplanation),
        .answered(previewExplanation, watching: true),
    ])
    .preferredColorScheme(.dark)
}
