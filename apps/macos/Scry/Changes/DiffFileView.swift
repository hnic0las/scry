import SwiftUI

/// The widths of a diff's columns for one window width and layout.
struct DiffMetrics: Equatable {
    let layout: ChangesViewModel.Layout
    /// The whole file area, change map excluded.
    let width: CGFloat
    /// One line-number column.
    let number: CGFloat
    /// The blame gutter, left of the new side's numbers; 0 when hidden.
    let blame: CGFloat
    static let sign: CGFloat = 16
    static let divider: CGFloat = 1
    /// The blame gutter with the age and the author's name.
    static let blameFull: CGFloat = 128
    /// With the age and the author's initials: in split, and where the
    /// file area is too narrow for names.
    static let blameCompact: CGFloat = 80
    /// The file area's width from which unified has room for names: a
    /// 1,280 pt window with its sidebar.
    static let blameFullFrom: CGFloat = 900

    /// `digits` is the longest line number of either side.
    init(layout: ChangesViewModel.Layout, width: CGFloat, digits: Int, blame: Bool = false) {
        self.layout = layout
        self.width = max(width, 0)
        self.blame = !blame ? 0 : layout == .unified && width >= Self.blameFullFrom ? Self.blameFull : Self.blameCompact
        number = CGFloat(digits) * Self.digitWidth + 2 * Metrics.lineNumberPadding
    }

    /// The width of one digit of the line-number font.
    static let digitWidth: CGFloat = {
        let font = NSFont.monospacedSystemFont(ofSize: 11, weight: .regular)
        return ("0" as NSString).size(withAttributes: [.font: font]).width
    }()

    /// One side of the split view.
    var pane: CGFloat { max((width - Self.divider) / 2, 0) }

    /// What comes before the code: two numbers and the sign in unified,
    /// one number and the sign per side in split.
    var gutter: CGFloat {
        layout == .unified ? 2 * number + Self.sign : number + Self.sign
    }

    /// The width the code is shown in: the new side's, the narrower one
    /// in split when blame is shown.
    var code: CGFloat {
        max((layout == .unified ? width : pane) - gutter - blame, 0)
    }

    /// The blame gutter has no room for names.
    var compactBlame: Bool { blame < Self.blameFull }

    /// How far the longest line, of `columns` columns, can scroll sideways.
    func maximumScroll(columns: Int) -> CGFloat {
        // A little past the end, so the last character clears the edge.
        max(CGFloat(columns + 2) * Metrics.codeCharacterWidth - code, 0)
    }
}

/// A text file whole: the compare bar, the rows in the window's layout,
/// and the change map.
struct DiffFileView: View {
    let viewModel: ChangesViewModel
    let diff: TextDiffInfo
    let scroll: SideScroll
    /// Moves the rows to a place given in points, after the file changed
    /// on disk.
    @State private var position = ScrollPosition()

    var body: some View {
        GeometryReader { geometry in
            let metrics = DiffMetrics(
                layout: viewModel.layout,
                width: geometry.size.width - ChangeMapView.width,
                digits: viewModel.numberDigits,
                blame: viewModel.blame != nil
            )
            VStack(spacing: 0) {
                CompareBar(
                    oldLabel: viewModel.header.oldLabel, newLabel: viewModel.header.newLabel, metrics: metrics
                )
                Rectangle().fill(Theme.Surfaces.hairline).frame(height: 1)
                if let notice = viewModel.notice {
                    DiffNotice(text: notice, turnOn: diff.large ? { Task { await viewModel.turnOnMarks() } } : nil)
                    Rectangle().fill(Theme.Surfaces.hairline).frame(height: 1)
                }
                HStack(spacing: 0) {
                    VStack(spacing: 0) {
                        rows(metrics: metrics)
                        DiffScrollBars(scroll: scroll, metrics: metrics)
                    }
                    ChangeMapView(marks: viewModel.changeMap, rows: viewModel.rowCount) { viewModel.go(to: $0) }
                }
            }
            .onChange(of: metrics.maximumScroll(columns: Int(diff.longestLine)), initial: true) { _, maximum in
                scroll.maximum = maximum
            }
        }
    }

    /// The blame and what a click on one of its lines does.
    private var blameGutter: BlameGutter? {
        viewModel.blame.map { blame in BlameGutter(reading: blame) { viewModel.showBlamed($0) } }
    }

    private func rows(metrics: DiffMetrics) -> some View {
        ScrollViewReader { proxy in
            ScrollView(.vertical) {
                LazyVStack(alignment: .leading, spacing: 0) {
                    let current = viewModel.currentRows
                    if viewModel.layout == .unified {
                        ForEach(diff.unified.indices, id: \.self) { index in
                            UnifiedRowView(
                                line: diff.unified[index], diff: diff, metrics: metrics, scroll: scroll,
                                current: current.contains(index), blame: blameGutter,
                                lines: LineGutter(model: viewModel, row: index)
                            )
                            .id(index)
                        }
                    } else {
                        ForEach(diff.rows.indices, id: \.self) { index in
                            SplitRowView(
                                row: diff.rows[index], diff: diff, metrics: metrics, scroll: scroll,
                                current: current.contains(index), blame: blameGutter,
                                lines: LineGutter(model: viewModel, row: index)
                            )
                            .id(index)
                        }
                    }
                }
            }
            .scrollPosition($position)
            .onScrollGeometryChange(for: CGFloat.self) { $0.contentOffset.y } action: { _, offset in
                viewModel.topRow = offset / Metrics.rowHeight
            }
            .onChange(of: viewModel.placeRequest) {
                // The file changed on disk: the line at the top stays there.
                position.scrollTo(y: viewModel.placeTarget * Metrics.rowHeight)
            }
            // The other layout has other rows: its list is built anew.
            .id(viewModel.layout)
            .background(Theme.Surfaces.pane)
            .background {
                HorizontalScrollMonitor { delta in scroll.scroll(by: delta) }
            }
            .onChange(of: viewModel.scrollRequest, initial: true) {
                // After this layout pass, once the rows exist.
                Task { @MainActor in
                    guard let target = viewModel.scrollTarget else { return }
                    // A few lines of what comes before stay in view.
                    proxy.scrollTo(target, anchor: UnitPoint(x: 0, y: 0.25))
                }
            }
        }
    }
}

/// What the two sides are: the one place to look to know what is compared.
struct CompareBar: View {
    let oldLabel: String
    let newLabel: String
    let metrics: DiffMetrics

    static let height: CGFloat = 26

    var body: some View {
        HStack(spacing: 0) {
            if metrics.layout == .unified {
                HStack(spacing: 6) {
                    name(oldLabel)
                    Image(systemName: "arrow.right")
                        .font(.system(size: 9, weight: .semibold))
                        .foregroundStyle(Theme.Surfaces.textSecondary)
                    name(newLabel)
                }
                .padding(.horizontal, 12)
                Spacer(minLength: 0)
            } else {
                side("Old", oldLabel).frame(width: metrics.pane, alignment: .leading)
                Rectangle().fill(Theme.Surfaces.hairline).frame(width: DiffMetrics.divider)
                side("New", newLabel).frame(width: metrics.pane, alignment: .leading)
                Spacer(minLength: 0)
            }
        }
        .frame(height: Self.height)
        .background(Theme.Surfaces.chrome)
        .accessibilityElement(children: .combine)
        .accessibilityLabel("Comparing \(oldLabel) with \(newLabel)")
    }

    private func name(_ label: String) -> some View {
        Text(label)
            .font(.system(size: 11, weight: .semibold, design: .monospaced))
            .lineLimit(1)
            .truncationMode(.middle)
    }

    private func side(_ role: String, _ label: String) -> some View {
        HStack(spacing: 6) {
            Text(role)
                .font(Metrics.caption)
                .foregroundStyle(Theme.Surfaces.textSecondary)
            name(label)
        }
        .padding(.horizontal, 12)
    }
}

/// One line about the file, under the compare bar.
struct DiffNotice: View {
    let text: String
    /// "Turn On for This File", on a large file.
    var turnOn: (() -> Void)?

    var body: some View {
        HStack(spacing: 10) {
            Text(text)
                .font(Metrics.caption)
                .foregroundStyle(Theme.Surfaces.textSecondary)
                .lineLimit(2)
            if let turnOn {
                Button("Turn On for This File", action: turnOn)
                    .controlSize(.small)
            }
            Spacer(minLength: 0)
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 5)
        .background(Theme.Surfaces.chrome)
    }
}

/// A line's code, coloured once: SwiftUI evaluates it again only when the
/// line or its marks change, not on every sideways-scroll tick.
struct DiffCode: View, Equatable {
    let line: DiffLine
    let words: [ByteRange]
    let wordTint: Color?

    var body: some View {
        Text(CodeText.highlighted(text: line.text, spans: line.spans, words: words, wordTint: wordTint))
            .font(Metrics.code)
            .lineLimit(1)
            .fixedSize()
    }
}

/// A line's code in its column, scrolled sideways with the others and cut
/// at the column's edge.
struct DiffCodeCell: View {
    let line: DiffLine
    let words: [ByteRange]
    let wordTint: Color?
    let scroll: SideScroll
    let width: CGFloat

    var body: some View {
        DiffCode(line: line, words: words, wordTint: wordTint)
            .equatable()
            .offset(x: -scroll.clamped)
            .frame(width: width, alignment: .leading)
            .mask { clipped ? AnyView(fadeMask) : AnyView(Color.black) }
            .clipped()
    }

    /// Whether the line goes on past the column's right edge.
    private var clipped: Bool {
        CGFloat(line.columns) * Metrics.codeCharacterWidth - scroll.clamped > width
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
}

/// A line number, right aligned in its column.
struct DiffNumber: View {
    let number: UInt32?
    let width: CGFloat
    /// The row's line is selected: bold, on the selection's tint.
    var selected = false

    var body: some View {
        Text(number.map(String.init) ?? "")
            .font(selected ? Metrics.lineNumber.weight(.bold) : Metrics.lineNumber)
            .foregroundStyle(selected ? Theme.Surfaces.text : Theme.Surfaces.textTertiary)
            .frame(width: width - Metrics.lineNumberPadding, alignment: .trailing)
            .padding(.trailing, Metrics.lineNumberPadding)
    }
}

/// "+" or "−" in its colour: added and removed never rely on colour alone.
struct DiffSign: View {
    enum Kind {
        case none, added, removed
    }

    let kind: Kind

    var body: some View {
        Text(kind == .added ? "+" : kind == .removed ? "−" : "")
            .font(.system(size: 11, weight: .bold, design: .monospaced))
            .foregroundStyle(kind == .added ? Theme.Diff.addedMark : Theme.Diff.removedMark)
            .frame(width: DiffMetrics.sign, alignment: .leading)
    }
}

/// The bar at the left of the rows of the block "next change" is on.
private struct CurrentBlockBar: View {
    let current: Bool

    var body: some View {
        Rectangle()
            .fill(current ? Theme.Interaction.focus : Color.clear)
            .frame(width: 2)
    }
}

/// One line of the unified view: old number, new number, sign, code.
struct UnifiedRowView: View {
    let line: UnifiedDiffLine
    let diff: TextDiffInfo
    let metrics: DiffMetrics
    let scroll: SideScroll
    let current: Bool
    var blame: BlameGutter?
    var lines: LineGutter?

    /// The row's line on the side whose lines can be selected.
    private var selectable: UInt32? {
        lines?.model.selectsOldSide == true ? line.old : line.new
    }

    private var isSelected: Bool {
        guard let selectable, let selection = lines?.model.selection else { return false }
        return selection.contains(Int(selectable))
    }

    var body: some View {
        HStack(spacing: 0) {
            if let blame {
                BlameCellView(gutter: blame, line: line.new, width: metrics.blame, compact: metrics.compactBlame)
            }
            HStack(spacing: 0) {
                DiffNumber(number: line.old, width: metrics.number, selected: isSelected)
                DiffNumber(number: line.new, width: metrics.number, selected: isSelected)
            }
            .frame(maxHeight: .infinity)
            .background(isSelected ? Theme.Interaction.selection : Color.clear)
            .modifier(LineSelectGesture(lines: lines, selectable: selectable != nil))
            DiffSign(kind: sign)
            if let text {
                DiffCodeCell(line: text, words: line.words, wordTint: wordTint, scroll: scroll, width: metrics.code)
            }
            Spacer(minLength: 0)
        }
        .frame(height: Metrics.rowHeight)
        .background(background)
        .overlay(alignment: .leading) { CurrentBlockBar(current: current) }
        .modifier(LineMarks(lines: lines, number: selectable))
        .accessibilityElement(children: .combine)
        .accessibilityLabel(label)
        .accessibilityAddTraits(isSelected ? .isSelected : [])
    }

    /// The line's text: on the old side when removed, on the new otherwise.
    private var text: DiffLine? {
        switch line.kind {
        case .removed: line.old.flatMap { diff.oldLines[safe: Int($0) - 1] }
        case .added, .unchanged: line.new.flatMap { diff.newLines[safe: Int($0) - 1] }
        }
    }

    private var sign: DiffSign.Kind {
        switch line.kind {
        case .added: .added
        case .removed: .removed
        case .unchanged: .none
        }
    }

    private var wordTint: Color? {
        switch line.kind {
        case .added: Theme.Diff.addedWord
        case .removed: Theme.Diff.removedWord
        case .unchanged: nil
        }
    }

    private var background: Color {
        switch line.kind {
        case .added: Theme.Diff.added
        case .removed: Theme.Diff.removed
        case .unchanged: Theme.Surfaces.pane
        }
    }

    private var label: String {
        let content = text?.text ?? ""
        switch line.kind {
        case .added: return "Added, line \(line.new ?? 0): \(content)"
        case .removed: return "Removed, line \(line.old ?? 0): \(content)"
        case .unchanged: return "Line \(line.new ?? 0): \(content)"
        }
    }
}

/// One row of the split view: the old line on the left, the new one on the
/// right; a side with no line here is hatched, as in the merge editor.
struct SplitRowView: View {
    let row: DiffRow
    let diff: TextDiffInfo
    let metrics: DiffMetrics
    let scroll: SideScroll
    let current: Bool
    var blame: BlameGutter?
    var lines: LineGutter?

    var body: some View {
        HStack(spacing: 0) {
            side(
                number: row.old, line: row.old.flatMap { diff.oldLines[safe: Int($0) - 1] },
                words: row.oldWords, changed: row.kind == .removed || row.kind == .changed, added: false
            )
            Rectangle().fill(Theme.Surfaces.hairline).frame(width: DiffMetrics.divider)
            if let blame {
                BlameCellView(gutter: blame, line: row.new, width: metrics.blame, compact: metrics.compactBlame)
            }
            side(
                number: row.new, line: row.new.flatMap { diff.newLines[safe: Int($0) - 1] },
                words: row.newWords, changed: row.kind == .added || row.kind == .changed, added: true
            )
            Spacer(minLength: 0)
        }
        .frame(height: Metrics.rowHeight)
        .overlay(alignment: .leading) { CurrentBlockBar(current: current) }
    }

    @ViewBuilder
    private func side(number: UInt32?, line: DiffLine?, words: [ByteRange], changed: Bool, added: Bool) -> some View {
        if let line {
            // Lines are selected on the new side, or on the old one of a
            // deleted file.
            let selects = lines.map { $0.model.selectsOldSide != added } ?? false
            let isSelected = selects && number.map { lines?.model.selection?.contains(Int($0)) == true } == true
            HStack(spacing: 0) {
                DiffNumber(number: number, width: metrics.number, selected: isSelected)
                    .frame(maxHeight: .infinity)
                    .background(isSelected ? Theme.Interaction.selection : Color.clear)
                    .modifier(LineSelectGesture(lines: lines, selectable: selects))
                DiffSign(kind: !changed ? .none : added ? .added : .removed)
                DiffCodeCell(
                    line: line, words: words,
                    wordTint: !changed ? nil : added ? Theme.Diff.addedWord : Theme.Diff.removedWord,
                    scroll: scroll, width: metrics.code + (added ? 0 : metrics.blame)
                )
            }
            .frame(width: metrics.pane - (added ? metrics.blame : 0), alignment: .leading)
            .frame(maxHeight: .infinity)
            .background(!changed ? Theme.Surfaces.pane : added ? Theme.Diff.added : Theme.Diff.removed)
            .modifier(LineMarks(lines: selects ? lines : nil, number: number))
            .accessibilityElement(children: .combine)
            .accessibilityLabel(
                "\(!changed ? "Line" : added ? "Added, line" : "Removed, line") \(number ?? 0): \(line.text)"
            )
        } else {
            Hatch()
                .frame(width: metrics.pane - (added ? metrics.blame : 0))
                .accessibilityHidden(true)
        }
    }
}

/// The scroll bar under the code, one per side in split: every row moves
/// by the same offset. Hidden when every line fits.
struct DiffScrollBars: View {
    let scroll: SideScroll
    let metrics: DiffMetrics

    var body: some View {
        if scroll.canScroll {
            HStack(spacing: 0) {
                bar(new: metrics.layout == .unified)
                if metrics.layout == .split {
                    Rectangle().fill(Theme.Surfaces.hairline).frame(width: DiffMetrics.divider)
                    bar(new: true)
                }
                Spacer(minLength: 0)
            }
            .frame(height: SideScrollBar.height)
            .background(Theme.Surfaces.pane)
            .overlay(alignment: .top) { Rectangle().fill(Theme.Surfaces.hairline).frame(height: 1) }
        }
    }

    /// The bar under one side; the new side has the blame gutter before
    /// its numbers.
    private func bar(new: Bool) -> some View {
        HStack(spacing: 0) {
            Color.clear.frame(width: metrics.gutter + (new ? metrics.blame : 0))
            SideScrollBar(scroll: scroll, visible: metrics.code)
        }
        .frame(width: metrics.layout == .unified ? metrics.width : metrics.pane)
    }
}

/// The strip beside the rows: one mark per changed block, its removed part
/// in red and its added part in green, in proportion to the file. A click
/// goes there. A file shorter than the strip keeps its rows' places.
struct ChangeMapView: View {
    let marks: [ChangesViewModel.MapMark]
    /// The rows the file area draws.
    let rows: Int
    let go: (Int) -> Void

    static let width: CGFloat = 12

    var body: some View {
        GeometryReader { geometry in
            let content = min(CGFloat(rows) * Metrics.rowHeight, geometry.size.height)
            ForEach(marks) { mark in
                let height = max(mark.length * content, 3)
                let removed = height * mark.removedShare
                Button { go(mark.id) } label: {
                    VStack(spacing: 0) {
                        Rectangle().fill(Theme.Diff.removedMark).frame(height: removed)
                        Rectangle().fill(Theme.Diff.addedMark).frame(height: height - removed)
                    }
                    .frame(width: mark.current ? 8 : 6)
                    .clipShape(RoundedRectangle(cornerRadius: 1))
                    .frame(width: Self.width, height: height)
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .help("Change \(mark.id + 1)")
                .accessibilityLabel("Change \(mark.id + 1)")
                .position(x: Self.width / 2, y: mark.start * content + height / 2)
            }
        }
        .frame(width: Self.width)
        .background(Theme.Surfaces.pane)
        .overlay(alignment: .leading) { Rectangle().fill(Theme.Surfaces.hairline).frame(width: 1) }
        .accessibilityElement(children: .contain)
        .accessibilityLabel("Changes in the file")
    }
}

extension Array {
    /// The element at `index`, or `nil` outside the array: a row naming a
    /// line the core did not send is drawn empty rather than crash.
    subscript(safe index: Int) -> Element? {
        indices.contains(index) ? self[index] : nil
    }
}

/// The blame of the file on screen, and what a click on a line does.
struct BlameGutter {
    let reading: BlameReading
    let show: (BlameCommitInfo) -> Void
}

/// The blame gutter beside one row: the age and the author on the first
/// line of each group of lines one commit wrote, a grey bar while git has
/// not said, nothing beside a row with no new line.
struct BlameCellView: View {
    let gutter: BlameGutter
    /// The row's line on the new side.
    let line: UInt32?
    let width: CGFloat
    /// No room for names: initials.
    let compact: Bool

    static let ageWidth: CGFloat = 30

    var body: some View {
        let cell = line.map { gutter.reading.cell(line: $0) }
        ZStack(alignment: .leading) {
            Theme.Surfaces.sidebar
            if let line, let cell {
                content(cell, line: line)
            }
        }
        .frame(width: width)
        .frame(maxHeight: .infinity)
        .overlay(alignment: .trailing) { Rectangle().fill(Theme.Surfaces.hairline).frame(width: 1) }
        .overlay(alignment: .top) {
            if isFirst(cell) { Rectangle().fill(Theme.Surfaces.hairline).frame(height: 1) }
        }
    }

    private func isFirst(_ cell: BlameCell?) -> Bool {
        switch cell {
        case .notCommitted(let first), .commit(_, let first): first
        case .unknown, nil: false
        }
    }

    @ViewBuilder
    private func content(_ cell: BlameCell, line: UInt32) -> some View {
        switch cell {
        case .unknown:
            // Bars of a few lengths, the same for a line each time.
            Capsule()
                .fill(Theme.Surfaces.hairline)
                .frame(width: (width - 24) * [0.62, 0.45, 0.78, 0.55, 0.7][Int(line) % 5], height: 6)
                .padding(.leading, 8)
                .accessibilityHidden(true)
        case .notCommitted(let first):
            if first {
                Text(compact ? "—" : "Not committed")
                    .font(Metrics.caption.italic())
                    .foregroundStyle(Theme.Surfaces.textTertiary)
                    .lineLimit(1)
                    .padding(.leading, compact ? 8 : 8 + Self.ageWidth)
                    .accessibilityLabel("Not committed")
            }
        case .commit(let commit, let first):
            let start = gutter.reading.groupStart(line: line)
            ZStack(alignment: .leading) {
                if gutter.reading.hovered == start { Theme.Interaction.selection }
                if first {
                    HStack(spacing: 0) {
                        Text(blameAge(time: commit.time, now: Int64(Date().timeIntervalSince1970)))
                            .foregroundStyle(Theme.Surfaces.textTertiary)
                            .frame(width: Self.ageWidth, alignment: .leading)
                        Text(compact ? BlameReading.initials(commit) : BlameReading.author(commit))
                            .foregroundStyle(Theme.Surfaces.textSecondary)
                            .lineLimit(1)
                            .truncationMode(.tail)
                    }
                    .font(Metrics.caption)
                    .monospacedDigit()
                    .padding(.leading, 8)
                    .padding(.trailing, 4)
                }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .leading)
            .contentShape(Rectangle())
            .onHover { inside in
                if inside {
                    gutter.reading.hovered = start
                } else if gutter.reading.hovered == start {
                    gutter.reading.hovered = nil
                }
            }
            .onTapGesture { gutter.show(commit) }
            .help(BlameReading.help(commit))
            .accessibilityElement()
            .accessibilityLabel("\(BlameReading.author(commit)), \(commit.subject)")
            .accessibilityAddTraits(.isButton)
        }
    }
}
