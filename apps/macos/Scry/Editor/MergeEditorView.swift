import SwiftUI

/// S2: ours, result and theirs side by side, with the inspector (S3).
struct MergeEditorView: View {
    @Bindable var viewModel: MergeEditorViewModel
    @State private var showInspector = true
    /// How far the three panes are scrolled sideways, shared by all rows.
    @State private var scrollX: CGFloat

    /// `scrollX` starts the panes scrolled sideways, for previews.
    init(viewModel: MergeEditorViewModel, scrollX: CGFloat = 0) {
        self.viewModel = viewModel
        _scrollX = State(initialValue: scrollX)
    }

    var body: some View {
        // The inspector is a plain column under the toolbar, as in the
        // prototype: SwiftUI's `.inspector` would run it up through the
        // toolbar on macOS 26 and later, over the editor's toolbar items.
        HStack(spacing: 0) {
            VStack(spacing: 0) {
                GeometryReader { geometry in
                    let widths = PaneWidths(total: geometry.size.width)
                    VStack(spacing: 0) {
                        PaneHeaders(
                            ours: viewModel.oursHeader,
                            result: viewModel.resultHeader,
                            theirs: viewModel.theirsHeader,
                            widths: widths
                        )
                        Rectangle().fill(Theme.Surfaces.hairline).frame(height: 1)
                        rowsList(widths: widths)
                    }
                }
                StatusBar(
                    resolved: viewModel.resolvedSummary,
                    verification: viewModel.verification,
                    written: viewModel.written,
                    language: viewModel.language
                )
            }
            .background(Theme.Surfaces.pane)
            if showInspector {
                Rectangle().fill(Theme.Surfaces.hairline).frame(width: 1)
                InspectorView(viewModel: viewModel.inspector)
                    .frame(width: Metrics.inspectorWidth)
            }
        }
        .navigationTitle(viewModel.title)
        .navigationSubtitle(viewModel.subtitle)
        .toolbar { toolbar }
        .background { hiddenShortcuts }
        .alert("The result does not parse", isPresented: $viewModel.confirmingInvalidWrite) {
            Button("Write Anyway", role: .destructive, action: viewModel.writeAnyway)
            Button("Cancel", role: .cancel) {}
        } message: {
            Text(viewModel.invalidWriteMessage)
        }
        .alert(
            "Something went wrong",
            isPresented: Binding(
                get: { viewModel.failure != nil },
                set: { if !$0 { viewModel.failure = nil } }
            )
        ) {
            Button("OK") { viewModel.failure = nil }
        } message: {
            Text(viewModel.failure ?? "")
        }
    }

    private func rowsList(widths: PaneWidths) -> some View {
        ScrollViewReader { proxy in
            ScrollView(.vertical) {
                LazyVStack(alignment: .leading, spacing: 0) {
                    ForEach(Array(viewModel.rows.enumerated()), id: \.offset) { offset, row in
                        RowView(
                            row: row,
                            widths: widths,
                            scrollX: clampedScrollX(widths),
                            focus: viewModel.focus
                        )
                        .id(offset)
                        if case .lens(let lens) = row, lens.editing, let editBlock = viewModel.editBlock {
                            EditBlockView(viewModel: editBlock, widths: widths)
                        }
                    }
                }
            }
            .background {
                HorizontalScrollMonitor { delta in
                    scrollX = min(max(clampedScrollX(widths) + delta, 0), maxScrollX(widths))
                }
            }
            .onChange(of: viewModel.currentConflict) { _, conflict in
                if let row = viewModel.lensRowIndex(of: conflict) {
                    withAnimation { proxy.scrollTo(row, anchor: .top) }
                }
            }
        }
    }

    /// Far enough for the longest line to show whole in the narrowest pane.
    private func maxScrollX(_ widths: PaneWidths) -> CGFloat {
        let longest = CGFloat(viewModel.longestLine) * Metrics.codeCharacterWidth + Metrics.clipFadeWidth
        return max(longest - widths.code(widths.side), 0)
    }

    /// The offset, kept in range when the window narrows or the rows change.
    private func clampedScrollX(_ widths: PaneWidths) -> CGFloat {
        min(scrollX, maxScrollX(widths))
    }

    @ToolbarContentBuilder
    private var toolbar: some ToolbarContent {
        ToolbarItemGroup {
            Button(action: viewModel.previous) {
                Image(systemName: "chevron.up")
            }
            .keyboardShortcut(.upArrow, modifiers: [.option, .command])
            .help("Previous conflict (⌥⌘↑)")
            .disabled(!viewModel.canGoToPrevious)

            Button(action: viewModel.next) {
                Image(systemName: "chevron.down")
            }
            .keyboardShortcut(.downArrow, modifiers: [.option, .command])
            .help("Next conflict (⌥⌘↓)")
            .disabled(!viewModel.canGoToNext)
        }

        // A label, not a control: its own item, with room on both sides, and
        // without the glass capsule toolbar items get on macOS 26 and later.
        if #available(macOS 26, *) {
            ToolbarItem { conflictCounter }
                .sharedBackgroundVisibility(.hidden)
        } else {
            ToolbarItem { conflictCounter }
        }

        ToolbarItem {
            ControlGroup {
                // Icon and title in one HStack: a Label would show only one of
                // them, at the toolbar's choice. Each side keeps its shape and
                // colour (● ours, ■ theirs), so it reads without colour too.
                Button { viewModel.take("ours") } label: {
                    HStack(spacing: 5) {
                        SideShape.ours
                        Text("Ours")
                    }
                }
                .help("Take ours (⌥⌘1)")
                Button(action: viewModel.takeBoth) {
                    HStack(spacing: 5) {
                        HStack(spacing: 2) {
                            SideShape.ours
                            SideShape.theirs
                        }
                        Text("Both")
                    }
                }
                .help("Take both, ours first; again for theirs first (⌥⌘3)")
                Button { viewModel.take("theirs") } label: {
                    HStack(spacing: 5) {
                        Text("Theirs")
                        SideShape.theirs
                    }
                }
                .help("Take theirs (⌥⌘2)")
            }
        }

        ToolbarItem {
            Button { showInspector.toggle() } label: {
                Image(systemName: "sidebar.right")
            }
            .keyboardShortcut("i", modifiers: [.control, .command])
            .help("Show or hide the inspector (⌃⌘I)")
        }

        ToolbarItem(placement: .primaryAction) {
            Button(action: viewModel.markResolved) {
                HStack(spacing: 4) {
                    Image(systemName: "checkmark")
                    Text("Mark as Resolved")
                }
                .padding(.horizontal, 4)
            }
            .keyboardShortcut(.return, modifiers: .command)
            .buttonStyle(.borderedProminent)
            // While editing, ⌘↩ applies the edit instead.
            .disabled(!viewModel.canMarkResolved)
            .help("Write the result (⌘↩)")
        }
    }

    private var conflictCounter: some View {
        Text(viewModel.counter)
            .foregroundStyle(Theme.Surfaces.textSecondary)
            .monospacedDigit()
            .fixedSize()
            .padding(.horizontal, 8)
    }

    /// Shortcuts without a visible button (SPEC: keyboard map).
    private var hiddenShortcuts: some View {
        Group {
            Button("Take ours") { viewModel.take("ours") }
                .keyboardShortcut("1", modifiers: [.option, .command])
            Button("Take theirs") { viewModel.take("theirs") }
                .keyboardShortcut("2", modifiers: [.option, .command])
            Button("Take both", action: viewModel.takeBoth)
                .keyboardShortcut("3", modifiers: [.option, .command])
            Button("Take base") { viewModel.take("base") }
                .keyboardShortcut("0", modifiers: [.option, .command])
            Button("Take none") { viewModel.take("neither") }
                .keyboardShortcut("9", modifiers: [.option, .command])
            // Disabled while editing, so ↩ types a line break and ⌘Z undoes
            // typing in the editor.
            Button("Edit by hand") { viewModel.startEditing() }
                .keyboardShortcut(.return, modifiers: [])
                .disabled(viewModel.isEditing)
            Button("Undo", action: viewModel.undo)
                .keyboardShortcut("z", modifiers: .command)
                .disabled(!viewModel.canUndo)
            Button("Redo", action: viewModel.redo)
                .keyboardShortcut("z", modifiers: [.shift, .command])
                .disabled(!viewModel.canRedo)
        }
        .frame(width: 0, height: 0)
        .opacity(0)
        .accessibilityHidden(true)
    }

}

/// Column widths: ours 1fr, connector, result 1.25fr, connector, theirs 1fr.
struct PaneWidths {
    let side: CGFloat
    let result: CGFloat
    let connector = Metrics.connectorWidth

    init(total: CGFloat) {
        let flexible = max(total - 2 * Metrics.connectorWidth, 0)
        side = flexible / 3.25
        result = flexible * 1.25 / 3.25
    }

    /// The width left for code in a pane, after the line-number and mark
    /// gutters, which do not scroll.
    func code(_ pane: CGFloat) -> CGFloat {
        max(pane - Metrics.lineNumberWidth - Metrics.markWidth, 0)
    }
}

struct PaneHeaders: View {
    let ours: MergeEditorViewModel.PaneHeader
    let result: MergeEditorViewModel.PaneHeader
    let theirs: MergeEditorViewModel.PaneHeader
    let widths: PaneWidths

    var body: some View {
        HStack(alignment: .top, spacing: 0) {
            header(ours, color: Theme.Sides.ours, symbol: "circle.fill")
            .frame(width: widths.side)
            Color.clear.frame(width: widths.connector)
            header(result, color: Theme.Sides.result, symbol: "diamond.fill")
            .frame(width: widths.result)
            Color.clear.frame(width: widths.connector)
            header(theirs, color: Theme.Sides.theirs, symbol: "square.fill")
            .frame(width: widths.side)
        }
        // The spacer columns would otherwise stretch the row to the whole
        // editor height.
        .fixedSize(horizontal: false, vertical: true)
        .background(Theme.Surfaces.window)
    }

    private func header(_ header: MergeEditorViewModel.PaneHeader, color: Color, symbol: String) -> some View {
        VStack(alignment: .leading, spacing: 2) {
            Rectangle().fill(color).frame(height: Metrics.paneHeaderRule)
            VStack(alignment: .leading, spacing: 2) {
                Label {
                    Text(header.label).font(Font.system(size: 12, weight: .semibold))
                } icon: {
                    Image(systemName: symbol).font(.system(size: 8))
                }
                .foregroundStyle(color)
                .lineLimit(2)
                Text(header.detail)
                    .font(Metrics.caption)
                    .foregroundStyle(Theme.Surfaces.textSecondary)
                    .lineLimit(1)
            }
            .padding(.horizontal, 10)
            .padding(.top, 5)
            .padding(.bottom, 7)
        }
        .accessibilityElement(children: .combine)
    }
}

struct StatusBar: View {
    let resolved: String
    let verification: MergeEditorViewModel.Verification
    let written: Bool
    let language: String?

    var body: some View {
        HStack(spacing: 16) {
            Text(resolved)
            verificationLabel
            Spacer()
            if written {
                Label("Written to disk", systemImage: "checkmark.circle")
                    .foregroundStyle(Theme.States.resolved)
            }
            if let language {
                Text(language)
            }
        }
        .font(Metrics.caption)
        .foregroundStyle(Theme.Surfaces.textSecondary)
        .padding(.horizontal, 10)
        .frame(height: Metrics.statusBarHeight)
        .background(Theme.Surfaces.chrome)
        .overlay(alignment: .top) {
            Rectangle().fill(Theme.Surfaces.hairline).frame(height: 1)
        }
    }

    @ViewBuilder
    private var verificationLabel: some View {
        switch verification {
        case .parses(let language):
            Label("Parses as \(language)", systemImage: "checkmark")
                .foregroundStyle(Theme.States.resolved)
        case .doesNotParse(let line):
            Label("Does not parse · line \(line)", systemImage: "xmark")
                .foregroundStyle(Theme.Severity.error)
        case .notChecked:
            EmptyView()
        }
    }
}

/// The shape of a side in its colour: ours ● circle, theirs ■ square.
enum SideShape {
    static var ours: some View {
        Image(systemName: "circle.fill")
            .font(.system(size: 9))
            .foregroundStyle(Theme.Sides.ours)
            .accessibilityHidden(true)
    }

    static var theirs: some View {
        Image(systemName: "square.fill")
            .font(.system(size: 9))
            .foregroundStyle(Theme.Sides.theirs)
            .accessibilityHidden(true)
    }
}

#Preview("Editor, unresolved", traits: .fixedLayout(width: 1440, height: 600)) {
    if let editor = PreviewSample.editor() {
        MergeEditorView(viewModel: editor)
    }
}

#Preview("Editor, resolved and written", traits: .fixedLayout(width: 1440, height: 600)) {
    if let editor = PreviewSample.editor({ editor in
        editor.take("ours-then-theirs")
        editor.next()
        editor.take("theirs")
        editor.markResolved()
    }) {
        MergeEditorView(viewModel: editor)
    }
}
