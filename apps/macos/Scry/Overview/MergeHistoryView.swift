import SwiftUI

/// A merge as a graph: the merge commit to come at the top, joining two
/// lanes, the branch checked out on the left and the branch merged in on
/// the right, down to the merge base where they split.
struct MergeHistoryView: View {
    let ours: String
    let theirs: String
    let caption: String
    let rows: [OverviewViewModel.HistoryRow]

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            VStack(alignment: .leading, spacing: 6) {
                Text("Merge history")
                    .font(Metrics.captionBold)
                    .foregroundStyle(Theme.Surfaces.textSecondary)
                HStack(spacing: 6) {
                    BranchPill(name: ours, role: "yours", color: Theme.Sides.ours)
                    Image(systemName: "arrow.left")
                        .font(.system(size: 9, weight: .bold))
                        .foregroundStyle(Theme.Surfaces.textTertiary)
                    BranchPill(name: theirs, role: "merged in", color: Theme.Sides.theirs)
                }
                Text(caption)
                    .font(Metrics.caption)
                    .foregroundStyle(Theme.Surfaces.textTertiary)
            }
            .padding(.horizontal, 12)
            .padding(.top, 10)
            .padding(.bottom, 6)
            ScrollView {
                VStack(alignment: .leading, spacing: 0) {
                    ForEach(rows) { row in
                        HistoryRowView(row: row)
                    }
                }
                .padding(.horizontal, 8)
                .padding(.bottom, 10)
            }
        }
        .background(Theme.Surfaces.sidebar)
    }
}

/// A branch's name in its lane's colour, with its role.
struct BranchPill: View {
    let name: String
    let role: String
    let color: Color

    var body: some View {
        Text(name)
            .font(Metrics.codeSmall.weight(.semibold))
            .lineLimit(1)
            .truncationMode(.middle)
            .padding(.horizontal, 6)
            .padding(.vertical, 2)
            .background(Capsule().fill(color.opacity(0.16)))
            .foregroundStyle(color)
            .help("\(name), \(role)")
    }
}

/// Where the lanes run in the gutter.
private enum Lanes {
    static let gutter: CGFloat = 44
    static let ours: CGFloat = 12
    static let theirs: CGFloat = 30
    static let rowHeight: CGFloat = 40
}

/// One row of the merge graph.
struct HistoryRowView: View {
    let row: OverviewViewModel.HistoryRow

    var body: some View {
        HStack(spacing: 8) {
            gutter
            VStack(alignment: .leading, spacing: 1) {
                Text(row.title)
                    .font(Metrics.callout.weight(row.kind == .mergeCommit ? .semibold : .regular))
                    .italic(isMore)
                    .foregroundStyle(isMore ? Theme.Surfaces.textSecondary : Theme.Surfaces.text)
                    .lineLimit(1)
                    .truncationMode(.tail)
                Text(row.detail)
                    .font(Metrics.caption)
                    .foregroundStyle(Theme.Surfaces.textTertiary)
                    .lineLimit(1)
            }
            Spacer(minLength: 0)
        }
        .frame(height: Lanes.rowHeight)
        .padding(.trailing, 6)
        .background(
            RoundedRectangle(cornerRadius: 6)
                .fill(row.kind == .mergeCommit ? Theme.Interaction.accent.opacity(0.08) : Color.clear)
        )
        .contentShape(Rectangle())
        .help(row.help)
        .accessibilityElement(children: .combine)
    }

    private var isMore: Bool {
        if case .more = row.kind { return true }
        return false
    }

    private var gutter: some View {
        let height = Lanes.rowHeight
        let middle = height / 2
        return ZStack {
            switch row.kind {
            case .mergeCommit:
                line(Lanes.ours, from: middle, to: height, Theme.Sides.ours)
                Path { path in
                    path.move(to: CGPoint(x: Lanes.ours, y: middle))
                    path.addQuadCurve(
                        to: CGPoint(x: Lanes.theirs, y: height),
                        control: CGPoint(x: Lanes.theirs, y: middle)
                    )
                }
                .stroke(Theme.Sides.theirs, lineWidth: 2)
                // Not made yet: a hollow, dashed node.
                Circle()
                    .fill(Theme.Surfaces.sidebar)
                    .overlay(Circle().strokeBorder(Theme.Interaction.accent, style: StrokeStyle(lineWidth: 2, dash: [3, 2])))
                    .frame(width: 16, height: 16)
                    .position(x: Lanes.ours, y: middle)
            case .commit(let side):
                line(Lanes.ours, from: 0, to: height, Theme.Sides.ours)
                line(Lanes.theirs, from: 0, to: height, Theme.Sides.theirs)
                Circle()
                    .fill(color(side))
                    .overlay(Circle().strokeBorder(Theme.Surfaces.sidebar, lineWidth: 2))
                    .frame(width: 12, height: 12)
                    .position(x: x(side), y: middle)
            case .more(let side):
                line(Lanes.ours, from: 0, to: height, Theme.Sides.ours, dashed: side == .ours)
                line(Lanes.theirs, from: 0, to: height, Theme.Sides.theirs, dashed: side == .theirs)
            case .base:
                line(Lanes.ours, from: 0, to: middle, Theme.Sides.ours)
                Path { path in
                    path.move(to: CGPoint(x: Lanes.theirs, y: 0))
                    path.addQuadCurve(
                        to: CGPoint(x: Lanes.ours, y: middle),
                        control: CGPoint(x: Lanes.theirs, y: middle)
                    )
                }
                .stroke(Theme.Sides.theirs, lineWidth: 2)
                line(Lanes.ours, from: middle, to: height, Theme.Surfaces.textTertiary, dashed: true)
                Circle()
                    .fill(Theme.Surfaces.textSecondary)
                    .frame(width: 12, height: 12)
                    .position(x: Lanes.ours, y: middle)
            }
        }
        .frame(width: Lanes.gutter, height: height)
    }

    private func line(_ x: CGFloat, from top: CGFloat, to bottom: CGFloat, _ color: Color, dashed: Bool = false) -> some View {
        Path { path in
            path.move(to: CGPoint(x: x, y: top))
            path.addLine(to: CGPoint(x: x, y: bottom))
        }
        .stroke(color, style: StrokeStyle(lineWidth: 2, dash: dashed ? [2, 3] : []))
    }

    private func x(_ side: OverviewViewModel.HistoryRow.Side) -> CGFloat {
        side == .ours ? Lanes.ours : Lanes.theirs
    }

    private func color(_ side: OverviewViewModel.HistoryRow.Side) -> Color {
        side == .ours ? Theme.Sides.ours : Theme.Sides.theirs
    }
}
