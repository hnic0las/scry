import SwiftUI

/// The stacked layout: ours and theirs side by side above, the result full
/// width below, with a bar between them to resize (SPEC: sides above,
/// result below). Both areas list the same rows at the same heights, the
/// core's aligned rows with their spacers, so scrolling them together is
/// sharing one offset.
struct StackedPanes: View {
    let viewModel: MergeEditorViewModel
    let scroll: SideScroll
    let size: CGSize

    /// The top area's share of the height.
    @State private var topFraction: CGFloat = 0.44
    @State private var dragStart: CGFloat?
    @State private var top = ScrollPosition(idType: Int.self)
    @State private var bottom = ScrollPosition(idType: Int.self)
    /// The area the user scrolls: only it moves the other, so the other's
    /// clamping never cuts the first one's momentum or bounce.
    @State private var driver: Area?
    /// The edit block's height, matched by a blank in the top area.
    @State private var editHeight: CGFloat = 0
    /// The row under the pointer, in either area: the twin line, tinted in
    /// both so the eye can jump between them.
    @State private var hovered: Int?

    private enum Area {
        case top, bottom
    }

    static let barHeight: CGFloat = 9

    var body: some View {
        // The overview ruler, beside the result, takes its width off both
        // areas so the sides and the result stay the same width.
        let width = size.width - OverviewRuler.width
        let sides = PaneWidths.stackedSides(total: width)
        let result = PaneWidths.stackedResult(total: width)
        let available = max(size.height - Self.barHeight, 0)
        VStack(spacing: 0) {
            VStack(spacing: 0) {
                PaneHeaders(
                    ours: viewModel.oursHeader, result: viewModel.resultHeader, theirs: viewModel.theirsHeader,
                    widths: sides, part: .sides
                )
                Rectangle().fill(Theme.Surfaces.hairline).frame(height: 1)
                area(.top, widths: sides)
                SideScrollBars(scroll: scroll, widths: sides, part: .sides)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .frame(height: available * topFraction)
            resizeBar(available: available)
            VStack(spacing: 0) {
                PaneHeaders(
                    ours: viewModel.oursHeader, result: viewModel.resultHeader, theirs: viewModel.theirsHeader,
                    widths: result, part: .result
                )
                .frame(maxWidth: .infinity, alignment: .leading)
                Rectangle().fill(Theme.Surfaces.hairline).frame(height: 1)
                HStack(spacing: 0) {
                    VStack(spacing: 0) {
                        area(.bottom, widths: result)
                        SideScrollBars(scroll: scroll, widths: result, part: .result)
                    }
                    OverviewRuler(ruler: viewModel.ruler) { viewModel.focus($0) }
                }
            }
            .frame(maxHeight: .infinity)
        }
        .onChange(of: viewModel.currentConflict) { _, conflict in
            guard let row = viewModel.scrollTarget(for: conflict) else { return }
            driver = nil
            withAnimation {
                top.scrollTo(id: row, anchor: .top)
                bottom.scrollTo(id: row, anchor: .top)
            }
        }
    }

    private func area(_ area: Area, widths: PaneWidths) -> some View {
        ScrollView(.vertical) {
            LazyVStack(alignment: .leading, spacing: 0) {
                let rows = viewModel.rows
                ForEach(rows.indices, id: \.self) { offset in
                    let row = rows[offset]
                    RowView(
                        row: row,
                        widths: widths,
                        scroll: scroll,
                        part: area == .top ? .sides : .result,
                        twin: hovered == offset,
                        take: { viewModel.take($0, in: $1) },
                        takeBoth: { viewModel.takeBoth(in: $0) },
                        editByHand: { viewModel.editByHand(in: $0) },
                        focus: { viewModel.focus($0) }
                    )
                    .id(offset)
                    .onHover { inside in
                        if inside {
                            hovered = offset
                        } else if hovered == offset {
                            hovered = nil
                        }
                    }
                    if case .lens(let lens) = row, lens.editing, let editBlock = viewModel.editBlock {
                        if area == .bottom {
                            EditBlockView(viewModel: editBlock, widths: widths)
                                .onGeometryChange(for: CGFloat.self) { $0.size.height } action: { editHeight = $0 }
                        } else {
                            Theme.Surfaces.sidebar.frame(height: editHeight)
                        }
                    }
                }
            }
            .scrollTargetLayout()
        }
        .scrollPosition(area == .top ? $top : $bottom)
        .onScrollPhaseChange { _, phase in
            // The user's own scrolling; a programmatic scroll animates
            // both areas at once and needs no driver.
            if phase == .interacting || phase == .tracking || phase == .decelerating {
                driver = area
            } else if phase == .idle, driver == area {
                driver = nil
            }
        }
        .onScrollGeometryChange(for: CGFloat.self) { geometry in
            geometry.contentOffset.y + geometry.contentInsets.top
        } action: { _, y in
            follow(from: area, y: y)
        }
        .background {
            HorizontalScrollMonitor { delta in
                scroll.scroll(by: delta)
            }
        }
    }

    /// Moves the other area to the same offset.
    private func follow(from area: Area, y: CGFloat) {
        guard driver == area || driver == nil else { return }
        let y = max(y, 0)
        switch area {
        case .top: bottom.scrollTo(y: y)
        case .bottom: top.scrollTo(y: y)
        }
    }

    /// A 9 pt bar with a grab handle; drag to resize.
    private func resizeBar(available: CGFloat) -> some View {
        ZStack {
            Theme.Surfaces.chrome
            Capsule()
                .fill(Theme.Surfaces.textTertiary)
                .frame(width: 36, height: 3)
        }
        .frame(height: Self.barHeight)
        .overlay(alignment: .top) { Rectangle().fill(Theme.Surfaces.hairline).frame(height: 1) }
        .overlay(alignment: .bottom) { Rectangle().fill(Theme.Surfaces.hairline).frame(height: 1) }
        .contentShape(Rectangle())
        .onHover { inside in
            if inside { NSCursor.resizeUpDown.push() } else { NSCursor.pop() }
        }
        .gesture(
            DragGesture(minimumDistance: 1)
                .onChanged { drag in
                    let start = dragStart ?? topFraction
                    dragStart = start
                    guard available > 0 else { return }
                    topFraction = min(max(start + drag.translation.height / available, 0.2), 0.8)
                }
                .onEnded { _ in dragStart = nil }
        )
        .help("Drag to resize. Both areas scroll together.")
        .accessibilityLabel("Resize the sides and the result")
    }
}
