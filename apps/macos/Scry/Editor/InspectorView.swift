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

        if let suggest = viewModel.suggest {
            switch suggest {
            case .proposed:
                EmptyView()
            default:
                SuggestBlock(
                    suggest: suggest,
                    ask: { viewModel.askSuggest() },
                    cancel: { viewModel.cancelSuggest() }
                )
            }
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

        if case .proposed(let proposal, let accepted, let stale) = viewModel.suggest {
            ProposalCard(
                proposal: proposal,
                title: viewModel.proposalTitle(proposal),
                accepted: accepted,
                stale: stale,
                language: viewModel.language,
                enabled: viewModel.canAnswerProposal,
                accept: { viewModel.acceptProposal() },
                edit: { viewModel.editProposal() },
                reject: { viewModel.rejectProposal() },
                askAgain: { viewModel.askSuggest() }
            )
        }

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
    /// What is explained: its words differ.
    var about = About.conflict

    enum About {
        case conflict
        /// Lines selected in the Changes window, where esc clears the
        /// selection rather than take the ask back.
        case lines
    }

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
                    if about == .conflict {
                        Button("Cancel", action: cancel)
                            .keyboardShortcut(.cancelAction)
                    } else {
                        Button("Cancel", action: cancel)
                    }
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
            case .stale(let explanation):
                stale(explanation)
            case .failed(let explanation):
                box(strip: nil) {
                    Label {
                        Text("Claude Code reported a problem").font(Metrics.headline)
                    } icon: {
                        Image(systemName: "exclamationmark.triangle").foregroundStyle(Theme.Severity.warning)
                    }
                    detail("Look at your Claude Code session, then ask again.")
                    Text(explanation.text)
                        .font(Metrics.code)
                        .foregroundStyle(Theme.Surfaces.textSecondary)
                        .fixedSize(horizontal: false, vertical: true)
                        .textSelection(.enabled)
                    Text("\(explanation.by) · \(Self.relative(explanation.at))")
                        .font(Metrics.caption)
                        .foregroundStyle(Theme.Surfaces.textSecondary)
                        .help(Self.exact(explanation.at))
                    HStack {
                        Spacer()
                        askAgain
                    }
                }
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
            detail(about == .conflict
                ? "Claude Code explains what each side meant and the risk of each candidate. It never changes the result."
                : "Claude Code explains what the selected lines do and why they changed. It never changes a file.")
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

    /// An answer about text the user rewrote since: kept, quiet, with a
    /// grey dashed strip.
    private func stale(_ explanation: Explanation) -> some View {
        box(strip: .stale) {
            Label {
                Text("About an earlier version").font(Metrics.captionBold)
            } icon: {
                Image(systemName: "clock").foregroundStyle(Theme.Surfaces.textSecondary)
            }
            detail(about == .conflict
                ? "The conflict changed after this was written."
                : "These lines changed after this was written.")
            Text(explanation.text)
                .font(Metrics.callout)
                .lineSpacing(5)
                .foregroundStyle(Theme.Surfaces.textSecondary)
                .lineLimit(expanded || !Self.isLong(explanation.text) ? nil : 8)
                .fixedSize(horizontal: false, vertical: true)
                .textSelection(.enabled)
            // Who and when on a line of its own: beside the buttons, the
            // agent's name wrapped on four lines.
            Text("Explained by \(explanation.by) · \(Self.relative(explanation.at))")
                .font(Metrics.caption)
                .foregroundStyle(Theme.Surfaces.textSecondary)
                .help(Self.exact(explanation.at))
            HStack {
                Spacer()
                if Self.isLong(explanation.text) {
                    Button(expanded ? "Show Less" : "Show More", action: toggleExpanded)
                        .buttonStyle(.link)
                }
                askAgain
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

    private enum Strip { case solid, dashed, stale }

    /// A box with the agent's provenance strip on its left edge: dashed
    /// while asked, solid once answered, grey dashed once stale, none
    /// while nobody watches.
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
                            strip == .stale ? Theme.Sides.base : Theme.Provenance.agent,
                            style: StrokeStyle(lineWidth: 3, lineCap: .round, dash: strip == .solid ? [] : [4, 4])
                        )
                        .frame(width: 3)
                        .padding(.vertical, 8)
                        .padding(.leading, 5)
                }
            }
    }

    /// More than eight lines in the inspector, about.
    static func isLong(_ text: String) -> Bool { text.count > 420 }

    /// "2 minutes ago"; an answer stamped a moment after the app's clock
    /// reads as now, not "in 0 seconds".
    static func relative(_ milliseconds: UInt64) -> String {
        let now = Date()
        let date = min(Date(timeIntervalSince1970: Double(milliseconds) / 1000), now)
        return RelativeDateTimeFormatter().localizedString(for: date, relativeTo: now)
    }

    static func exact(_ milliseconds: UInt64) -> String {
        Date(timeIntervalSince1970: Double(milliseconds) / 1000)
            .formatted(date: .abbreviated, time: .standard)
    }
}

/// Suggest: asking the user's Claude Code for a resolution, until it
/// proposes one (then `ProposalCard`, among the candidates). Nothing here
/// changes the result.
struct SuggestBlock: View {
    let suggest: InspectorViewModel.Suggest
    let ask: () -> Void
    let cancel: () -> Void

    var body: some View {
        Group {
            switch suggest {
            case .notAsked, .proposed:
                VStack(alignment: .leading, spacing: 6) {
                    Button(action: ask) {
                        Label("Suggest a Resolution…", systemImage: "sparkles")
                    }
                    .help(Shortcut.suggest.help())
                    detail("Claude Code proposes a resolution and says why. Nothing changes until you accept it.")
                }
            case .waiting(let seconds):
                AgentBox(strip: .dashed) {
                    HStack {
                        ProgressView().controlSize(.small)
                        Text("Asked Claude Code for a resolution…").font(Metrics.headline)
                        Spacer()
                        Text("\(seconds) s")
                            .font(Metrics.callout)
                            .monospacedDigit()
                            .foregroundStyle(Theme.Surfaces.textSecondary)
                    }
                    detail("A helper proposes one in the background of your session. Usually 10 to 30 seconds.")
                    Button("Cancel", action: cancel)
                }
            case .noSession:
                AgentBox(strip: nil) {
                    HStack(spacing: 6) {
                        PresenceDot(watching: false)
                        Text("Waiting for Claude Code").font(Metrics.headline)
                    }
                    detail("Your ask is queued. Open Claude Code in this repository and it proposes one right away.")
                    HStack {
                        Button("Cancel", action: cancel)
                        SettingsLink {
                            Text("Plugin not set up? Settings › Agent…")
                        }
                        .buttonStyle(.link)
                    }
                }
            case .noAnswer:
                AgentBox(strip: nil) {
                    Label {
                        Text("No proposal from Claude Code").font(Metrics.headline)
                    } icon: {
                        Image(systemName: "exclamationmark.triangle").foregroundStyle(Theme.Severity.warning)
                    }
                    detail("Nothing came back after a minute. Look at your Claude Code session: it may be busy or waiting for you.")
                    HStack {
                        Button("Ask Again", action: ask).help(Shortcut.suggest.help())
                        Button("Cancel", action: cancel)
                    }
                }
            case .failed(let failure):
                AgentBox(strip: nil) {
                    Label {
                        Text("Claude Code could not propose one").font(Metrics.headline)
                    } icon: {
                        Image(systemName: "exclamationmark.triangle").foregroundStyle(Theme.Severity.warning)
                    }
                    Text(failure.reason)
                        .font(Metrics.code)
                        .foregroundStyle(Theme.Surfaces.textSecondary)
                        .fixedSize(horizontal: false, vertical: true)
                        .textSelection(.enabled)
                    Text("\(failure.by) · \(ExplainBlock.relative(failure.at))")
                        .font(Metrics.caption)
                        .foregroundStyle(Theme.Surfaces.textSecondary)
                        .help(ExplainBlock.exact(failure.at))
                    HStack {
                        Spacer()
                        Button("Ask Again", action: ask).help(Shortcut.suggest.help())
                    }
                }
            }
        }
        .accessibilityElement(children: .contain)
        .accessibilityLabel("Suggest a resolution")
    }

    private func detail(_ text: String) -> some View {
        Text(text)
            .font(Metrics.callout)
            .foregroundStyle(Theme.Surfaces.textSecondary)
            .fixedSize(horizontal: false, vertical: true)
    }
}

/// The agent's proposal, among the candidates (S3's agent candidate):
/// dashed in the agent's colour until accepted, then solid and selected;
/// its reason, what it leaves out, whether the file parses with it, and
/// Accept, Edit, Reject. About text rewritten since, a grey strip says so.
struct ProposalCard: View {
    let proposal: AgentProposal
    /// The candidate it picks ("Theirs"), or "Own text".
    let title: String
    let accepted: Bool
    let stale: Bool
    let language: String?
    let enabled: Bool
    let accept: () -> Void
    let edit: () -> Void
    let reject: () -> Void
    let askAgain: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack(alignment: .top, spacing: 8) {
                Image(systemName: accepted ? "largecircle.fill.circle" : "circle")
                    .foregroundStyle(accepted ? Theme.Provenance.agent : Theme.Surfaces.textTertiary)
                VStack(alignment: .leading, spacing: 2) {
                    Text("Suggested by \(proposal.by)").font(Metrics.headline)
                    Text(title)
                        .font(Metrics.caption)
                        .foregroundStyle(Theme.Surfaces.textSecondary)
                }
                Spacer(minLength: 4)
                Badge(text: Provenance.agent.name, symbol: Provenance.agent.symbol, color: Theme.Provenance.agent)
            }
            if stale {
                Label("About an earlier version: you changed the conflict since.", systemImage: "clock")
                    .font(Metrics.caption)
                    .foregroundStyle(Theme.Surfaces.textSecondary)
            }
            preview
            Text(proposal.reason)
                .font(Metrics.callout)
                .foregroundStyle(stale ? Theme.Surfaces.textSecondary : Theme.Surfaces.text)
                .fixedSize(horizontal: false, vertical: true)
                .textSelection(.enabled)
            if let dropped = proposal.dropped {
                Text("Left out: \(dropped)")
                    .font(Metrics.caption)
                    .foregroundStyle(Theme.Surfaces.textSecondary)
                    .fixedSize(horizontal: false, vertical: true)
                    .textSelection(.enabled)
            }
            verification
            HStack {
                if accepted {
                    Label("Accepted · ⌘Z to undo", systemImage: "checkmark")
                        .font(Metrics.caption)
                        .foregroundStyle(Theme.States.resolved)
                } else {
                    Button("Accept", action: accept)
                        .help("Resolve this conflict with Claude Code's proposal")
                }
                Button("Edit", action: edit)
                    .help("Start from this proposal and write the result yourself")
                Button("Reject", action: reject)
                    .help("Forget this proposal")
                Spacer(minLength: 4)
                Button("Ask Again", action: askAgain)
                    .buttonStyle(.link)
                    .help(Shortcut.suggest.help())
            }
            .disabled(!enabled)
            Text(ExplainBlock.relative(proposal.at))
                .font(Metrics.caption)
                .foregroundStyle(Theme.Surfaces.textSecondary)
                .help(ExplainBlock.exact(proposal.at))
        }
        .padding(10)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(
            RoundedRectangle(cornerRadius: Metrics.cardCornerRadius)
                .fill(Theme.Surfaces.pane)
                .stroke(
                    stale ? Theme.Sides.base : Theme.Provenance.agent,
                    style: StrokeStyle(lineWidth: accepted ? 2 : 1, dash: accepted ? [] : [4, 3])
                )
        )
        .accessibilityElement(children: .contain)
        .accessibilityLabel("Suggested by \(proposal.by), \(title)")
        .accessibilityValue(accepted ? "accepted" : "")
    }

    private var preview: some View {
        let lines = proposal.text.split(separator: "\n", omittingEmptySubsequences: false)
            .dropLast(proposal.text.hasSuffix("\n") ? 1 : 0)
        return VStack(alignment: .leading, spacing: 0) {
            if lines.isEmpty {
                Text("(nothing)")
                    .italic()
                    .foregroundStyle(Theme.Surfaces.textTertiary)
            }
            ForEach(Array(lines.prefix(6).enumerated()), id: \.offset) { _, line in
                Text(String(line)).lineLimit(1).truncationMode(.tail)
            }
            if lines.count > 6 {
                Text("…").foregroundStyle(Theme.Surfaces.textTertiary)
            }
        }
        .font(Metrics.codeSmall)
        .foregroundStyle(Theme.Surfaces.text)
        .padding(8)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(RoundedRectangle(cornerRadius: 5).fill(Theme.Surfaces.sidebar))
        .textSelection(.enabled)
    }

    @ViewBuilder
    private var verification: some View {
        switch proposal.parse {
        case .parses:
            Label("Parses as \(language ?? "code")", systemImage: "checkmark")
                .font(Metrics.caption)
                .foregroundStyle(Theme.States.resolved)
        case .doesNotParse(let line):
            Label("Does not parse near line \(line) · still acceptable, then edit by hand", systemImage: "xmark")
                .font(Metrics.caption)
                .foregroundStyle(Theme.Severity.error)
        case .notChecked:
            EmptyView()
        }
    }
}

/// A box with the agent's provenance strip on its left edge: dashed while
/// asked, none while nobody watches.
private struct AgentBox<Content: View>: View {
    enum Strip { case dashed }
    let strip: Strip?
    @ViewBuilder let content: Content

    var body: some View {
        VStack(alignment: .leading, spacing: 6) { content }
            .padding(10)
            .padding(.leading, strip == nil ? 0 : 3)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(
                RoundedRectangle(cornerRadius: Metrics.cardCornerRadius)
                    .fill(Theme.Surfaces.pane)
                    .stroke(Theme.Surfaces.hairline)
            )
            .overlay(alignment: .leading) {
                if strip != nil {
                    VerticalLine()
                        .stroke(
                            Theme.Provenance.agent,
                            style: StrokeStyle(lineWidth: 3, lineCap: .round, dash: [4, 4])
                        )
                        .frame(width: 3)
                        .padding(.vertical, 8)
                        .padding(.leading, 5)
                }
            }
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
    by: "Claude Code",
    at: UInt64(Date().addingTimeInterval(-120).timeIntervalSince1970 * 1000),
    failed: false
)

private let previewFailure = Explanation(
    text: "conflict found no conflict c7a9 in Sources/Network.swift: the file has 2 conflicts.",
    by: "Claude Code",
    at: UInt64(Date().addingTimeInterval(-30).timeIntervalSince1970 * 1000),
    failed: true
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

#Preview("Explain stale and failed", traits: .fixedLayout(width: 320, height: 600)) {
    ExplainPreview(states: [
        .stale(previewExplanation),
        .failed(previewFailure),
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

private let previewProposal = AgentProposal(
    reason: "Keeps the rename's requestTimeout, which line 17 already uses, with main's 60 s for slow cellular links.",
    by: "Claude Code",
    at: UInt64(Date().addingTimeInterval(-40).timeIntervalSince1970 * 1000),
    failed: false,
    pick: nil,
    text: "    /// How long a request may wait before it fails.\n    static let requestTimeout: TimeInterval = 60\n",
    dropped: "nothing: only the old name timeout goes.",
    parse: .parses
)

#Preview("Suggest states", traits: .fixedLayout(width: 320, height: 1100)) {
    VStack(alignment: .leading, spacing: 16) {
        SuggestBlock(suggest: .notAsked, ask: {}, cancel: {})
        SuggestBlock(suggest: .waiting(seconds: 9), ask: {}, cancel: {})
        SuggestBlock(suggest: .noAnswer, ask: {}, cancel: {})
        ProposalCard(
            proposal: previewProposal, title: "Own text", accepted: false, stale: false, language: "Swift",
            enabled: true, accept: {}, edit: {}, reject: {}, askAgain: {}
        )
        ProposalCard(
            proposal: previewProposal, title: "Own text", accepted: true, stale: false, language: "Swift",
            enabled: true, accept: {}, edit: {}, reject: {}, askAgain: {}
        )
    }
    .padding(12)
    .frame(width: 320)
    .background(Theme.Surfaces.sidebar)
}
