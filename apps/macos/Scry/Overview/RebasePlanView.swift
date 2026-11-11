import SwiftUI

/// The rebase as a graph: the branch being rebased at the top, each line
/// of its todo list on that branch, newest first, and at the bottom the
/// commit it is replayed onto, where the branch leaves the target's line.
struct RebasePlanView: View {
    let caption: String
    let branch: String?
    let rows: [OverviewViewModel.PlanRow]
    let base: OverviewViewModel.PlanBase

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            VStack(alignment: .leading, spacing: 2) {
                Text("Rebase plan")
                    .font(Metrics.captionBold)
                    .foregroundStyle(Theme.Surfaces.textSecondary)
                Text(caption)
                    .font(Metrics.caption)
                    .foregroundStyle(Theme.Surfaces.textTertiary)
            }
            .padding(.horizontal, 12)
            .padding(.top, 10)
            .padding(.bottom, 6)
            ScrollView {
                VStack(alignment: .leading, spacing: 0) {
                    if let branch {
                        BranchTipRow(name: branch)
                    }
                    ForEach(Array(rows.enumerated()), id: \.element.id) { index, row in
                        PlanStepRow(row: row, startsLine: index == 0 && branch == nil)
                    }
                    PlanBaseRow(base: base)
                }
                .padding(.horizontal, 8)
                .padding(.bottom, 10)
            }
        }
        .background(Theme.Surfaces.sidebar)
    }
}

/// Where the lines run in the graph's gutter.
private enum Lane {
    static let gutter: CGFloat = 44
    /// The rebased branch.
    static let branch: CGFloat = 28
    /// The target branch, which the rebased one leaves at the bottom.
    static let target: CGFloat = 10
    static let rowHeight: CGFloat = 40
}

/// The branch's name, where its line starts.
struct BranchTipRow: View {
    let name: String

    var body: some View {
        HStack(spacing: 6) {
            Path { path in
                path.move(to: CGPoint(x: Lane.branch, y: Lane.rowHeight / 2))
                path.addLine(to: CGPoint(x: Lane.branch, y: Lane.rowHeight))
            }
            .stroke(Theme.Surfaces.hairline, style: StrokeStyle(lineWidth: 2, dash: [3, 3]))
            .overlay {
                Image(systemName: "arrow.triangle.branch")
                    .font(.system(size: 10, weight: .semibold))
                    .foregroundStyle(Theme.Interaction.accent)
                    .position(x: Lane.branch, y: Lane.rowHeight / 2)
            }
            .frame(width: Lane.gutter, height: Lane.rowHeight)
            Text(name)
                .font(Metrics.code.weight(.semibold))
                .padding(.horizontal, 6)
                .padding(.vertical, 2)
                .background(Capsule().fill(Theme.Interaction.accent.opacity(0.14)))
                .foregroundStyle(Theme.Interaction.accent)
            Spacer(minLength: 0)
        }
        .accessibilityElement(children: .combine)
        .accessibilityLabel("Branch \(name), being rebased")
    }
}

/// One line of the todo list: the graph's node, the action and the commit.
struct PlanStepRow: View {
    let row: OverviewViewModel.PlanRow
    /// The first row draws no line above its node.
    let startsLine: Bool

    var body: some View {
        HStack(alignment: .center, spacing: 8) {
            gutter
            ActionPill(action: row.action)
            VStack(alignment: .leading, spacing: 1) {
                Text(row.subject)
                    .font(Metrics.callout.weight(row.state == .current ? .semibold : .regular))
                    .strikethrough(row.dropped)
                    .foregroundStyle(row.dropped || row.state == .pending ? Theme.Surfaces.textSecondary : Theme.Surfaces.text)
                    .lineLimit(1)
                    .truncationMode(.tail)
                HStack(spacing: 4) {
                    if let shortId = row.shortId {
                        Text(shortId).font(Metrics.codeSmall)
                    }
                    if let note = row.note {
                        Text("↳ \(note)")
                    }
                    if row.state == .current {
                        Text("Stopped here")
                            .fontWeight(.semibold)
                            .foregroundStyle(Theme.Interaction.accent)
                    }
                }
                .font(Metrics.caption)
                .foregroundStyle(Theme.Surfaces.textTertiary)
            }
            Spacer(minLength: 0)
        }
        .frame(height: Lane.rowHeight)
        .padding(.trailing, 6)
        .background(
            RoundedRectangle(cornerRadius: 6)
                .fill(row.state == .current ? Theme.Interaction.accent.opacity(0.10) : Color.clear)
        )
        .contentShape(Rectangle())
        .help("\(row.number). \(row.action) · \(row.detail)")
        .accessibilityElement(children: .combine)
        .accessibilityLabel("Step \(row.number), \(row.action), \(row.subject). \(row.detail)")
    }

    private var gutter: some View {
        let middle = Lane.rowHeight / 2
        let x = row.joined ? Lane.branch + 12 : Lane.branch
        return ZStack {
            Path { path in
                path.move(to: CGPoint(x: Lane.branch, y: startsLine ? middle : 0))
                path.addLine(to: CGPoint(x: Lane.branch, y: Lane.rowHeight))
            }
            .stroke(lineColor, style: StrokeStyle(lineWidth: 2, dash: row.state == .pending ? [3, 3] : []))
            if row.joined {
                // A fixup hangs off the line, towards the commit it joins
                // below.
                Path { path in
                    path.move(to: CGPoint(x: x, y: middle))
                    path.addQuadCurve(
                        to: CGPoint(x: Lane.branch, y: Lane.rowHeight),
                        control: CGPoint(x: x, y: Lane.rowHeight)
                    )
                }
                .stroke(lineColor, style: StrokeStyle(lineWidth: 1.5, dash: row.state == .pending ? [2, 2] : []))
            }
            PlanNode(state: row.state, dropped: row.dropped, joined: row.joined, action: row.action)
                .position(x: x, y: middle)
        }
        .frame(width: Lane.gutter, height: Lane.rowHeight)
    }

    private var lineColor: Color {
        switch row.state {
        case .done: Theme.States.resolved
        case .current: Theme.Interaction.accent
        case .pending: Theme.Surfaces.hairline
        }
    }
}

/// A step's node: green and ticked once done, accent where git stopped,
/// hollow to come; a dropped commit is crossed out, a fixup is small.
struct PlanNode: View {
    let state: RebaseStepState
    let dropped: Bool
    let joined: Bool
    let action: String

    var body: some View {
        let size: CGFloat = joined ? 9 : state == .current ? 16 : 14
        ZStack {
            if state == .current {
                Circle()
                    .stroke(Theme.Interaction.accent.opacity(0.3), lineWidth: 4)
                    .frame(width: size + 6, height: size + 6)
            }
            Circle()
                .fill(fill)
            Circle()
                .strokeBorder(stroke, style: StrokeStyle(lineWidth: 1.5, dash: dropped ? [2, 2] : []))
            if !joined, let symbol {
                Image(systemName: symbol)
                    .font(.system(size: 7, weight: .heavy))
                    .foregroundStyle(state == .pending || dropped ? Theme.Surfaces.textSecondary : Color.white)
            }
        }
        .frame(width: size, height: size)
    }

    private var fill: Color {
        if dropped { return Theme.Surfaces.sidebar }
        switch state {
        case .done: return Theme.States.resolved
        case .current: return Theme.Interaction.accent
        case .pending: return Theme.Surfaces.sidebar
        }
    }

    private var stroke: Color {
        if dropped { return Theme.Surfaces.textTertiary }
        switch state {
        case .done: return Theme.States.resolved
        case .current: return Theme.Interaction.accent
        case .pending: return Theme.Surfaces.textTertiary
        }
    }

    private var symbol: String? {
        if dropped { return "xmark" }
        switch state {
        case .done: return "checkmark"
        case .current: return action == "edit" || action == "break" ? "pause.fill" : "exclamationmark"
        case .pending: return action == "exec" ? "chevron.right" : nil
        }
    }
}

/// The action, coloured by what it does to the commit.
struct ActionPill: View {
    let action: String

    var body: some View {
        Text(action)
            .font(.system(size: 10, weight: .semibold, design: .monospaced))
            .frame(minWidth: 44)
            .padding(.vertical, 2)
            .background(RoundedRectangle(cornerRadius: 4).fill(color.opacity(0.14)))
            .foregroundStyle(color)
    }

    private var color: Color {
        switch action {
        case "pick": Theme.Surfaces.textSecondary
        case "reword": Theme.Provenance.analyzer
        case "edit", "break": Theme.Severity.warning
        case "fixup", "squash": Theme.Provenance.structural
        case "drop": Theme.Severity.error
        default: Theme.Provenance.rule
        }
    }
}

/// The commit replayed onto, on the target branch's line, where the
/// rebased branch leaves it.
struct PlanBaseRow: View {
    let base: OverviewViewModel.PlanBase

    var body: some View {
        HStack(spacing: 8) {
            ZStack {
                // The rebased branch comes down and leaves the target here.
                Path { path in
                    path.move(to: CGPoint(x: Lane.branch, y: 0))
                    path.addQuadCurve(
                        to: CGPoint(x: Lane.target, y: Lane.rowHeight / 2),
                        control: CGPoint(x: Lane.branch, y: Lane.rowHeight / 2)
                    )
                }
                .stroke(Theme.States.resolved, lineWidth: 2)
                // The target's history goes on below.
                Path { path in
                    path.move(to: CGPoint(x: Lane.target, y: Lane.rowHeight / 2))
                    path.addLine(to: CGPoint(x: Lane.target, y: Lane.rowHeight))
                }
                .stroke(Theme.Surfaces.textTertiary, style: StrokeStyle(lineWidth: 2, dash: [1, 3]))
                Circle()
                    .fill(Theme.Surfaces.textSecondary)
                    .frame(width: 12, height: 12)
                    .position(x: Lane.target, y: Lane.rowHeight / 2)
            }
            .frame(width: Lane.gutter, height: Lane.rowHeight)
            Text(base.name)
                .font(Metrics.code.weight(.semibold))
                .padding(.horizontal, 6)
                .padding(.vertical, 2)
                .background(Capsule().fill(Theme.Surfaces.textSecondary.opacity(0.14)))
            VStack(alignment: .leading, spacing: 1) {
                Text(base.subject)
                    .font(Metrics.callout)
                    .lineLimit(1)
                    .truncationMode(.tail)
                Text("\(base.shortId) · replayed onto")
                    .font(Metrics.caption)
                    .foregroundStyle(Theme.Surfaces.textTertiary)
            }
            Spacer(minLength: 0)
        }
        .padding(.trailing, 6)
        .help("The commit the branch is replayed onto.")
        .accessibilityElement(children: .combine)
    }
}

/// What git ran on its own since it last stopped.
struct AppliedNote: View {
    let text: String

    var body: some View {
        Label(text, systemImage: "forward.fill")
            .font(Metrics.callout)
            .foregroundStyle(Theme.Severity.info)
            .padding(.horizontal, 10)
            .padding(.vertical, 6)
            .background(RoundedRectangle(cornerRadius: 6).fill(Theme.Severity.info.opacity(0.10)))
    }
}
