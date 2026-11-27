import SwiftUI

/// What a row needs to let its line be selected and to show what is
/// selected and explained.
struct LineGutter {
    let model: ChangesViewModel
    /// The row's place in the layout's rows.
    let row: Int
}

/// The line numbers of a row select its line: a click, a shift-click to
/// extend, a drag for a range.
struct LineSelectGesture: ViewModifier {
    let lines: LineGutter?
    /// The row has a line of the side that can be selected.
    let selectable: Bool

    func body(content: Content) -> some View {
        if let lines, selectable, lines.model.canSelectLines {
            content
                .contentShape(Rectangle())
                .gesture(
                    DragGesture(minimumDistance: 0)
                        .onChanged { value in
                            lines.model.pressLine(
                                row: lines.row,
                                rowDelta: Int((value.location.y / Metrics.rowHeight).rounded(.down)),
                                extend: NSEvent.modifierFlags.contains(.shift)
                            )
                        }
                        .onEnded { _ in lines.model.releaseLine() }
                )
                .accessibilityAddTraits(.isButton)
                .accessibilityHint("Selects the line, to explain it")
        } else {
            content
        }
    }
}

/// Over a row, or over one side of it: the wash and the edges of the
/// selection, the strip of an explanation, and on the first selected line
/// the bar that asks.
struct LineMarks: ViewModifier {
    let lines: LineGutter?
    /// The row's line on the side that can be selected.
    let number: UInt32?

    func body(content: Content) -> some View {
        if let lines, let number, lines.model.canSelectLines {
            let line = Int(number)
            let selection = lines.model.selection
            let first = selection?.lowerBound == line
            content
                .overlay {
                    if selection?.contains(line) == true {
                        // Faint, so added and removed stay readable.
                        Theme.Interaction.accent.opacity(0.10).allowsHitTesting(false)
                    }
                }
                .overlay(alignment: .top) {
                    if first { Rectangle().fill(Theme.Interaction.focus).frame(height: 1) }
                }
                .overlay(alignment: .bottom) {
                    if selection?.upperBound == line { Rectangle().fill(Theme.Interaction.focus).frame(height: 1) }
                }
                .overlay(alignment: .leading) {
                    if let strip = lines.model.strip(line: line) {
                        ExplainStripMark(kind: strip.kind) { lines.model.open(strip.id) }
                    }
                }
                .overlay(alignment: .trailing) {
                    if first, let selection {
                        SelectionBar(count: selection.count) { Task { await lines.model.explainSelection() } }
                            .padding(.trailing, 10)
                    }
                }
        } else {
            content
        }
    }
}

/// The 3 pt strip at the left of an explained line, in the agent's
/// colour: dashed while waiting, solid once answered, grey and dashed
/// once stale. A click shows the explanation.
struct ExplainStripMark: View {
    let kind: ChangesViewModel.ExplainStrip
    let open: () -> Void

    var body: some View {
        Rectangle()
            .fill(kind == .stale ? Theme.Sides.base : Theme.Provenance.agent)
            .frame(width: 3)
            // A gap in each row draws the dashes.
            .padding(.vertical, kind == .answered ? 0 : 4)
            .frame(width: 7, alignment: .leading)
            .contentShape(Rectangle())
            .onTapGesture(perform: open)
            .help(kind == .waiting ? "Asked Claude Code about these lines"
                : kind == .answered ? "Explained: click to read" : "Explained before these lines changed")
            .accessibilityLabel(kind == .waiting ? "Explanation asked" : kind == .answered ? "Explained" : "Explanation, stale")
    }
}

/// "9 lines · Explain ⌃⌘E", at the right of the first selected line.
struct SelectionBar: View {
    let count: Int
    let explain: () -> Void

    var body: some View {
        HStack(spacing: 8) {
            Text(count == 1 ? "1 line" : "\(count) lines")
                .foregroundStyle(Theme.Surfaces.textSecondary)
                .monospacedDigit()
            Button(action: explain) {
                HStack(spacing: 4) {
                    Image(systemName: "sparkle").foregroundStyle(Theme.Provenance.agent)
                    Text("Explain").fontWeight(.medium)
                    Text(Shortcut.explainLines.keys).foregroundStyle(Theme.Surfaces.textSecondary)
                }
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .help(Shortcut.explainLines.help("Ask Claude Code what these lines do and why they changed"))
        }
        .font(Metrics.caption)
        .padding(.horizontal, 8)
        .frame(height: Metrics.rowHeight - 2)
        .background(
            RoundedRectangle(cornerRadius: 5)
                .fill(Theme.Surfaces.chrome)
                .stroke(Theme.Surfaces.controlBorder)
        )
        .fixedSize()
    }
}

/// The inspector of the Changes window: the selection and its Explain
/// button, then the explanations of the file, the one in hand whole and
/// the others folded.
struct LineExplainInspector: View {
    let viewModel: ChangesViewModel

    var body: some View {
        ScrollViewReader { proxy in
            ScrollView {
                VStack(alignment: .leading, spacing: 10) {
                    if let selection = viewModel.selection, viewModel.currentExplanation == nil {
                        title("Selection")
                        Text(ChangesViewModel.linesLabel(selection))
                            .font(Metrics.headline)
                            .monospacedDigit()
                        ExplainBlock(
                            explain: .notAsked(watching: viewModel.explanations?.watching ?? false),
                            expanded: false, error: viewModel.explainError,
                            ask: { Task { await viewModel.explainSelection() } },
                            cancel: {}, toggleExpanded: {}, about: .lines
                        )
                    }
                    let explained = viewModel.explainedLines
                    if !explained.isEmpty {
                        title("Explanations in this file · \(explained.count)")
                            .padding(.top, viewModel.selection != nil && viewModel.currentExplanation == nil ? 6 : 0)
                        ForEach(explained) { one in
                            explanation(one).id(one.id)
                        }
                    }
                }
                .padding(12)
                .frame(maxWidth: .infinity, alignment: .leading)
            }
            .onChange(of: viewModel.openExplanation) { _, id in
                if let id { withAnimation { proxy.scrollTo(id, anchor: .top) } }
            }
        }
        .background(Theme.Surfaces.sidebar, ignoresSafeAreaEdges: [])
    }

    private func title(_ text: String) -> some View {
        Text(text)
            .font(Metrics.captionBold)
            .foregroundStyle(Theme.Surfaces.textSecondary)
            .monospacedDigit()
    }

    @ViewBuilder
    private func explanation(_ one: ChangesViewModel.ExplainedLines) -> some View {
        VStack(alignment: .leading, spacing: 5) {
            Button(one.label) { viewModel.show(one.item) }
                .buttonStyle(.link)
                .font(Metrics.callout.weight(.medium))
                .monospacedDigit()
                .help("Go to these lines and select them")
                .contextMenu {
                    Button("Remove This Explanation") { Task { await viewModel.forgetExplanation(one.item) } }
                }
            if viewModel.isOpen(one) {
                ExplainBlock(
                    explain: one.state,
                    expanded: viewModel.expandedExplanations.contains(one.id),
                    error: viewModel.openExplanation == one.id ? viewModel.explainError : nil,
                    ask: { Task { await viewModel.askAgain(one.item) } },
                    cancel: { Task { await viewModel.cancelExplain(one.item) } },
                    toggleExpanded: { viewModel.toggleExpanded(one.id) },
                    about: .lines
                )
            } else {
                FoldedExplanation(explained: one) { viewModel.open(one.id) }
            }
        }
    }
}

/// An explanation folded to two lines, with who and when: a click opens
/// it.
struct FoldedExplanation: View {
    let explained: ChangesViewModel.ExplainedLines
    let open: () -> Void

    var body: some View {
        Button(action: open) {
            VStack(alignment: .leading, spacing: 4) {
                Text(explained.item.answer?.text ?? "Asked Claude Code…")
                    .font(Metrics.callout)
                    .foregroundStyle(Theme.Surfaces.text)
                    .lineLimit(2)
                    .multilineTextAlignment(.leading)
                Text(detail)
                    .font(Metrics.caption)
                    .foregroundStyle(Theme.Surfaces.textSecondary)
            }
            .padding(10)
            .padding(.leading, 3)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(
                RoundedRectangle(cornerRadius: Metrics.cardCornerRadius)
                    .fill(Theme.Surfaces.pane)
                    .stroke(Theme.Surfaces.hairline)
            )
            .overlay(alignment: .leading) {
                RoundedRectangle(cornerRadius: 1.5)
                    .fill(explained.item.stale ? Theme.Sides.base : Theme.Provenance.agent)
                    .frame(width: 3)
                    .padding(.vertical, 8)
                    .padding(.leading, 5)
            }
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .help("Show this explanation")
    }

    private var detail: String {
        if explained.item.askedAt != nil { return "Waiting for the answer" }
        guard let answer = explained.item.answer else { return "" }
        let stale = explained.item.stale ? " · the lines changed since" : ""
        return "\(answer.by) · \(ExplainBlock.relative(answer.at))\(stale)"
    }
}
