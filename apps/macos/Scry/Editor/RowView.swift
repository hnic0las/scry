import SwiftUI

/// One row of the editor: a lens row above a conflict, or a code row with a
/// cell per pane. The core decides the rows; this only draws them.
struct RowView: View {
    let row: Row
    let widths: PaneWidths
    let model: EditorModel
    /// The panes' shared sideways scroll.
    let scrollX: CGFloat

    var body: some View {
        switch row {
        case .lens(let conflict):
            LensRow(conflict: Int(conflict), widths: widths, model: model)
        case .code(let ours, let result, let theirs, let conflict):
            let index = conflict.map { Int($0) }
            HStack(spacing: 0) {
                CellView(cell: ours, pane: .ours, invalid: false, scrollX: scrollX, codeWidth: widths.code(widths.side))
                    .frame(width: widths.side)
                Connector(color: Theme.Sides.ours, active: index != nil)
                    .frame(width: widths.connector)
                CellView(
                    cell: result, pane: .result, invalid: isInvalid(index),
                    scrollX: scrollX, codeWidth: widths.code(widths.result),
                    provenance: index.flatMap { model.chosenCandidate(of: $0)?.provenance.color } ?? Theme.Provenance.git
                )
                    .frame(width: widths.result)
                Connector(color: Theme.Sides.theirs, active: index != nil)
                    .frame(width: widths.connector)
                CellView(cell: theirs, pane: .theirs, invalid: false, scrollX: scrollX, codeWidth: widths.code(widths.side))
                    .frame(width: widths.side)
            }
            .frame(height: Metrics.rowHeight)
            .contentShape(Rectangle())
            .onTapGesture {
                if let index { model.current = index }
            }
        }
    }

    private func isInvalid(_ conflict: Int?) -> Bool {
        guard let conflict, let chosen = model.chosenCandidate(of: conflict) else { return false }
        if case .doesNotParse = chosen.parse { return true }
        return false
    }
}

enum Pane {
    case ours, result, theirs
}

struct CellView: View {
    let cell: Cell
    let pane: Pane
    /// The chosen candidate does not parse.
    let invalid: Bool
    let scrollX: CGFloat
    /// The width the code is shown in, gutters excluded.
    let codeWidth: CGFloat
    /// The colour of the chosen resolution's provenance, for the result.
    var provenance: Color = Theme.Provenance.git

    var body: some View {
        HStack(spacing: 0) {
            Text(cell.number.map(String.init) ?? "")
                .font(Metrics.lineNumber)
                .foregroundStyle(Theme.Surfaces.textTertiary)
                .frame(width: Metrics.lineNumberWidth - Metrics.lineNumberPadding, alignment: .trailing)
                .padding(.trailing, Metrics.lineNumberPadding)
            Text(mark)
                .font(Metrics.lineNumber)
                .foregroundStyle(markColor)
                .frame(width: Metrics.markWidth, alignment: .leading)
            content
                .frame(width: codeWidth, alignment: .leading)
                .clipped()
        }
        .frame(maxHeight: .infinity)
        .background(background)
        .overlay(alignment: .leading) { strip }
        .clipped()
    }

    @ViewBuilder
    private var content: some View {
        switch cell.kind {
        case .unresolved:
            Text("Unresolved · take a side or pick a candidate")
                .font(Metrics.callout)
                .foregroundStyle(Theme.Severity.error)
        case .removed:
            Text("Block removed · take none")
                .font(Metrics.callout)
                .italic()
                .foregroundStyle(Theme.Surfaces.textSecondary)
        default:
            // No wrapping: rows stay one line high so the panes stay aligned.
            Text(cell.highlighted(wordTint: wordTint))
                .font(Metrics.code)
                .lineLimit(1)
                .fixedSize()
                .offset(x: -scrollX)
                .frame(width: codeWidth, alignment: .leading)
                .mask { clipped ? AnyView(fadeMask) : AnyView(Color.black) }
                .overlay(alignment: .trailing) {
                    if clipped { clipIndicator }
                }
        }
    }

    /// Whether the line goes on past the pane's right edge.
    private var clipped: Bool {
        CGFloat(cell.text.count) * Metrics.codeCharacterWidth - scrollX > codeWidth
    }

    /// An opaque tab on the pane's edge, with the cell's own background and a
    /// rule on its left, so it reads as part of the interface, not as a `>`
    /// in the code.
    private var clipIndicator: some View {
        ZStack {
            Theme.Surfaces.pane
            background
            Image(systemName: "chevron.right")
                .font(.system(size: 8, weight: .bold))
                .foregroundStyle(Theme.Surfaces.textSecondary)
                .frame(width: Metrics.clipIndicatorWidth - 4, height: Metrics.rowHeight - 6)
                .background(RoundedRectangle(cornerRadius: 3).fill(Theme.Surfaces.textTertiary.opacity(0.3)))
        }
        .frame(width: Metrics.clipIndicatorWidth)
        .overlay(alignment: .leading) {
            Rectangle().fill(Theme.Surfaces.hairline).frame(width: 1)
        }
    }

    /// Opaque, then fading out over the last points, so a cut line never
    /// looks complete.
    private var fadeMask: some View {
        HStack(spacing: 0) {
            Color.black
            LinearGradient(colors: [.black, .clear], startPoint: .leading, endPoint: .trailing)
                .frame(width: Metrics.clipFadeWidth)
        }
    }

    private var mark: String {
        switch cell.kind {
        case .conflict:
            switch cell.change {
            case .added: return "+"
            case .edited: return "~"
            case .unchanged: return ""
            }
        case .unresolved: return "!"
        case .chosen: return invalid ? "!" : ""
        default: return ""
        }
    }

    /// Changed words are tinted with their side's colour.
    private var wordTint: Color? {
        switch (cell.kind, pane) {
        case (.conflict, .ours): return Theme.Sides.ours.opacity(Metrics.changedWordOpacity)
        case (.conflict, .theirs): return Theme.Sides.theirs.opacity(Metrics.changedWordOpacity)
        default: return nil
        }
    }

    private var markColor: Color {
        switch (cell.kind, pane) {
        case (.conflict, .ours): return Theme.Sides.ours
        case (.conflict, .theirs): return Theme.Sides.theirs
        default: return Theme.Severity.error
        }
    }

    @ViewBuilder
    private var background: some View {
        switch cell.kind {
        case .common:
            Theme.Surfaces.pane
        case .conflict:
            pane == .ours ? Theme.Sides.oursTint : Theme.Sides.theirsTint
        case .spacer:
            Hatch()
        case .unresolved:
            Theme.Severity.error.opacity(Metrics.blockTintOpacity)
        case .chosen:
            (invalid ? Theme.Severity.error : provenance).opacity(Metrics.blockTintOpacity)
        case .removed:
            Theme.Provenance.human.opacity(Metrics.blockTintOpacity)
        }
    }

    /// The result's left strip: solid for a choice, dotted red when
    /// unresolved or invalid.
    @ViewBuilder
    private var strip: some View {
        switch cell.kind {
        case .unresolved:
            DottedStrip(color: Theme.Severity.error)
        case .chosen where invalid:
            DottedStrip(color: Theme.Severity.error)
        case .chosen:
            Rectangle().fill(provenance).frame(width: Metrics.provenanceStrip)
        case .removed:
            Rectangle().fill(Theme.Provenance.human).frame(width: Metrics.provenanceStrip)
        default:
            EmptyView()
        }
    }
}

/// Diagonal lines on rows where a pane has no line (135°, 5 pt apart).
struct Hatch: View {
    var body: some View {
        Canvas { context, size in
            var path = Path()
            var x: CGFloat = -size.height
            while x < size.width {
                path.move(to: CGPoint(x: x, y: size.height))
                path.addLine(to: CGPoint(x: x + size.height, y: 0))
                x += 5
            }
            context.stroke(path, with: .color(Theme.Surfaces.hairline), lineWidth: 1)
        }
        .background(Theme.Surfaces.pane)
    }
}

struct DottedStrip: View {
    let color: Color

    var body: some View {
        Rectangle()
            .stroke(color, style: StrokeStyle(lineWidth: Metrics.provenanceStrip, dash: [2, 2]))
            .frame(width: Metrics.provenanceStrip)
    }
}

/// The column between two panes, tinted along a conflict.
struct Connector: View {
    let color: Color
    let active: Bool

    var body: some View {
        Rectangle().fill(active ? color.opacity(0.12) : Theme.Surfaces.pane)
    }
}

/// Above each conflict: its number and the state of its resolution.
struct LensRow: View {
    let conflict: Int
    let widths: PaneWidths
    let model: EditorModel

    var body: some View {
        HStack(spacing: 0) {
            LensCommitView(commit: info?.oursCommit)
                .frame(width: widths.side)
            Color.clear.frame(width: widths.connector)
            HStack(spacing: 6) {
                Button("Conflict \(conflict + 1)") { model.current = conflict }
                    .buttonStyle(.plain)
                    .font(Metrics.captionBold)
                    .padding(.horizontal, 6)
                    .frame(height: Metrics.badgeHeight)
                    .background(
                        RoundedRectangle(cornerRadius: 4)
                            .fill(focused ? Theme.Interaction.focus.opacity(0.2) : Theme.Surfaces.hairline)
                    )
                badges
                Spacer(minLength: 0)
            }
            .padding(.horizontal, 10)
            .frame(width: widths.result)
            Color.clear.frame(width: widths.connector)
            LensCommitView(commit: info?.theirsCommit)
                .frame(width: widths.side)
        }
        .frame(height: Metrics.lensHeight)
        .background(Theme.Surfaces.sidebar)
        .overlay(alignment: .top) { Rectangle().fill(Theme.Surfaces.hairline).frame(height: 1) }
    }

    private var focused: Bool { model.current == conflict }

    private var info: ConflictInfo? {
        model.conflicts.indices.contains(conflict) ? model.conflicts[conflict] : nil
    }

    @ViewBuilder
    private var badges: some View {
        if let chosen = model.chosenCandidate(of: conflict) {
            let provenance = chosen.provenance
            Badge(text: chosen.title, symbol: provenance.symbol, color: provenance.color)
            if case .doesNotParse = chosen.parse {
                Badge(text: "Does not parse", symbol: "xmark", color: Theme.Severity.error)
            }
        } else {
            Badge(text: "Unresolved", symbol: "exclamationmark.triangle", color: Theme.States.unresolved)
        }
    }
}

struct Badge: View {
    let text: String
    let symbol: String
    let color: Color

    var body: some View {
        Label(text, systemImage: symbol)
            .font(Metrics.captionBold)
            .foregroundStyle(color)
            .padding(.horizontal, 6)
            .frame(height: Metrics.badgeHeight)
            .background(RoundedRectangle(cornerRadius: 4).fill(color.opacity(0.13)))
            .lineLimit(1)
    }
}

/// A side's part of the lens row: the commit that last touched the
/// conflict's lines on that side, as `sha message · author · age`.
struct LensCommitView: View {
    let commit: LensCommit?

    var body: some View {
        HStack(spacing: 6) {
            if let commit {
                Text(commit.shortId)
                    .font(Metrics.lineNumber)
                    .foregroundStyle(Theme.Surfaces.text)
                // The message gives way first; author and age always show.
                Text(commit.subject)
                    .lineLimit(1)
                    .truncationMode(.tail)
                    .layoutPriority(0)
                Text("· \(commit.author) · \(commit.age)")
                    .lineLimit(1)
                    .fixedSize()
                    .layoutPriority(1)
            }
            Spacer(minLength: 0)
        }
        .font(Metrics.caption)
        .foregroundStyle(Theme.Surfaces.textSecondary)
        .padding(.leading, 10)
        .padding(.trailing, 8)
        .help(commit.map { "\($0.shortId) \($0.subject)\n\($0.author) · \($0.age)" } ?? "")
    }
}

#Preview("Lens commit", traits: .fixedLayout(width: 360, height: 90)) {
    VStack(spacing: 0) {
        LensCommitView(commit: LensCommit(
            shortId: "9f8e7d6", subject: "Add a 30 s timeout to uploads (#412)",
            author: "Alex Martin", age: "2 days ago"
        ))
        .frame(height: Metrics.lensHeight)
        LensCommitView(commit: LensCommit(
            shortId: "a1b2c3d", subject: "Retry failed uploads with exponential backoff and jitter",
            author: "You", age: "today"
        ))
        .frame(height: Metrics.lensHeight)
        LensCommitView(commit: nil)
            .frame(height: Metrics.lensHeight)
    }
    .background(Theme.Surfaces.sidebar)
}
