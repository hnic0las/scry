import SwiftUI

/// One row of the editor: a lens row above a conflict, or a code row with a
/// cell per pane. The core decides the rows; this only draws them.
struct RowView: View {
    let row: MergeEditorViewModel.RowState
    let widths: PaneWidths
    /// The panes' shared sideways scroll, read by the cells only.
    let scroll: SideScroll
    /// Focuses a conflict.
    let focus: (Int) -> Void

    var body: some View {
        switch row {
        case .lens(let lens):
            LensRow(state: lens, widths: widths) { focus(lens.conflict) }
        case .code(let code):
            HStack(spacing: 0) {
                CellView(cell: code.ours, pane: .ours, invalid: false, scroll: scroll, codeWidth: widths.code(widths.side))
                    .frame(width: widths.side)
                Connector(color: Theme.Sides.ours, active: code.conflict != nil)
                    .frame(width: widths.connector)
                CellView(
                    cell: code.result, pane: .result, invalid: code.resultInvalid,
                    scroll: scroll, codeWidth: widths.code(widths.result),
                    provenance: code.resultProvenance.color
                )
                    .frame(width: widths.result)
                Connector(color: Theme.Sides.theirs, active: code.conflict != nil)
                    .frame(width: widths.connector)
                CellView(cell: code.theirs, pane: .theirs, invalid: false, scroll: scroll, codeWidth: widths.code(widths.side))
                    .frame(width: widths.side)
            }
            .frame(height: Metrics.rowHeight)
            .contentShape(Rectangle())
            .onTapGesture {
                if let conflict = code.conflict { focus(conflict) }
            }
        }
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
    let scroll: SideScroll
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
                .offset(x: -scroll.clamped)
                .frame(width: codeWidth, alignment: .leading)
                .mask { clipped ? AnyView(fadeMask) : AnyView(Color.black) }
                .overlay(alignment: .trailing) {
                    if clipped { clipIndicator }
                }
        }
    }

    /// Whether the line goes on past the pane's right edge.
    private var clipped: Bool {
        CGFloat(cell.columns) * Metrics.codeCharacterWidth - scroll.clamped > codeWidth
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
    let state: MergeEditorViewModel.LensRowState
    let widths: PaneWidths
    let focus: () -> Void

    var body: some View {
        HStack(spacing: 0) {
            LensCommitView(commit: state.oursCommit)
                .frame(width: widths.side)
            Color.clear.frame(width: widths.connector)
            HStack(spacing: 6) {
                Button(state.title, action: focus)
                    .buttonStyle(.plain)
                    .font(Metrics.captionBold)
                    .padding(.horizontal, 6)
                    .frame(height: Metrics.badgeHeight)
                    .background(
                        RoundedRectangle(cornerRadius: 4)
                            .fill(state.focused ? Theme.Interaction.focus.opacity(0.2) : Theme.Surfaces.hairline)
                    )
                ForEach(Array(state.badges.enumerated()), id: \.offset) { _, badge in
                    Badge(badge)
                }
                Spacer(minLength: 0)
            }
            .padding(.horizontal, 10)
            .frame(width: widths.result)
            Color.clear.frame(width: widths.connector)
            LensCommitView(commit: state.theirsCommit)
                .frame(width: widths.side)
        }
        .frame(height: Metrics.lensHeight)
        .background(Theme.Surfaces.sidebar)
        .overlay(alignment: .top) { Rectangle().fill(Theme.Surfaces.hairline).frame(height: 1) }
    }

}

struct Badge: View {
    let text: String
    let symbol: String
    let color: Color

    init(text: String, symbol: String, color: Color) {
        self.text = text
        self.symbol = symbol
        self.color = color
    }

    /// A lens row's badge, in its role's symbol and colour.
    init(_ badge: LensBadge) {
        text = badge.text
        symbol = badge.role.symbol
        color = badge.role.color
    }

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

/// The core's theme roles as symbols and colours (portability rule 5).
extension ThemeRole {
    var symbol: String {
        switch self {
        case .provenance(let provenance): provenance.symbol
        case .state(let state): state.symbol
        case .severity(let severity): severity.symbol
        }
    }

    var color: Color {
        switch self {
        case .provenance(let provenance): provenance.color
        case .state(let state): state.color
        case .severity(let severity): severity.color
        }
    }
}

extension ConflictState {
    var symbol: String {
        switch self {
        case .unresolved: "exclamationmark.triangle"
        case .needsReview: "eye"
        case .resolved: "checkmark"
        case .staged: "tray.full"
        }
    }

    var color: Color {
        switch self {
        case .unresolved: Theme.States.unresolved
        case .needsReview: Theme.States.needsReview
        case .resolved: Theme.States.resolved
        case .staged: Theme.States.staged
        }
    }
}

extension Severity {
    var symbol: String {
        switch self {
        case .error: "xmark"
        case .warning: "exclamationmark.triangle"
        case .info: "info.circle"
        }
    }

    var color: Color {
        switch self {
        case .error: Theme.Severity.error
        case .warning: Theme.Severity.warning
        case .info: Theme.Severity.info
        }
    }
}
