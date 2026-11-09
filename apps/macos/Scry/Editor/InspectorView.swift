import SwiftUI

/// S3: the focused conflict, its classification and its candidates.
struct InspectorView: View {
    let model: EditorModel

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 12) {
                if let conflict = model.currentConflict {
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
            Text("Conflict \(conflict.number) of \(model.conflicts.count)")
                .font(Metrics.headline)
            Text("line \(conflict.startLine)")
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

        ForEach(conflict.candidates, id: \.id) { candidate in
            CandidateCard(
                candidate: candidate,
                selected: conflict.chosen == candidate.id,
                language: model.status.language
            ) {
                model.choose(candidate.id, conflict: Int(conflict.number) - 1)
            }
        }
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

    private var invalid: Bool {
        if case .doesNotParse = candidate.parse { return true }
        return false
    }

    private var accent: Color {
        invalid ? Theme.Severity.error : Theme.Interaction.focus
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

/// Where a candidate comes from, as the badges show it.
struct CandidateProvenance {
    let name: String
    let symbol: String
    let color: Color
}

extension CandidateInfo {
    /// The "closed" variants are made by a rule; the others are git's sides
    /// as they are.
    var provenance: CandidateProvenance {
        if id.hasSuffix("-closed") {
            return CandidateProvenance(name: "rule", symbol: "ruler", color: Theme.Provenance.rule)
        }
        return CandidateProvenance(name: "git", symbol: "arrow.triangle.branch", color: Theme.Provenance.git)
    }
}
