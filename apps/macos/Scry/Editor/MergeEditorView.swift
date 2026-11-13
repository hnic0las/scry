import SwiftUI

/// S2: ours, result and theirs side by side, with the inspector (S3).
struct MergeEditorView: View {
    @Bindable var viewModel: MergeEditorViewModel
    @State private var showInspector = true
    /// How far the three panes are scrolled sideways, shared by all rows.
    @State private var scroll: SideScroll

    /// `scrollX` starts the panes scrolled sideways, for previews.
    init(viewModel: MergeEditorViewModel, scrollX: CGFloat = 0) {
        self.viewModel = viewModel
        _scroll = State(initialValue: SideScroll(offset: scrollX))
    }

    var body: some View {
        // The inspector is a plain column under the toolbar, as in the
        // prototype: SwiftUI's `.inspector` would run it up through the
        // toolbar on macOS 26 and later, over the editor's toolbar items.
        HStack(spacing: 0) {
            VStack(spacing: 0) {
                GeometryReader { geometry in
                    Group {
                        if viewModel.layout == .stacked {
                            StackedPanes(viewModel: viewModel, scroll: scroll, size: geometry.size)
                        } else {
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
                    }
                    .onChange(of: maxScrollX(width: geometry.size.width), initial: true) { _, maximum in
                        scroll.maximum = maximum
                    }
                }
                .overlay(alignment: .bottom) { HintStrip(shortcuts: viewModel.hintStrip) }
                .overlay(alignment: .top) { ToolbarHintRow(shortcuts: viewModel.toolbarHints) }
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
        .onReceive(NotificationCenter.default.publisher(for: UserDefaults.didChangeNotification)) { _ in
            // Settings ▸ Appearance may have changed the layout.
            viewModel.reloadLayout()
        }
        .onGeometryChange(for: CGFloat.self) { $0.size.width } action: { width in
            viewModel.windowWidthChanged(Double(width))
        }
        .focusedSceneValue(\.editorLayoutActions, EditorLayoutActions(
            layout: viewModel.layout,
            enabled: viewModel.canSwitchLayout,
            set: { viewModel.setLayout($0) },
            toggle: { viewModel.switchLayout() },
            showsBase: viewModel.showsBase,
            canShowBase: viewModel.canShowBase,
            toggleBase: { viewModel.toggleBase() }
        ))
        .navigationTitle(viewModel.title)
        .navigationSubtitle(viewModel.subtitle)
        .toolbar { toolbar }
        .background { hiddenShortcuts }
        .alert("The result does not parse", isPresented: $viewModel.confirmingInvalidWrite) {
            Button("Write Anyway", role: .destructive, action: { viewModel.writeAnyway() })
            Button("Cancel", role: .cancel) {}
        } message: {
            Text(viewModel.invalidWriteMessage)
        }
        .alert("Go back without writing?", isPresented: $viewModel.confirmingLeave) {
            Button("Go Back Without Writing", role: .destructive) { viewModel.leaveWithoutWriting() }
            Button("Cancel", role: .cancel) {}
        } message: {
            Text("\(viewModel.resolvedSummary). Your choices are not written to \(viewModel.title) and will be lost.")
        }
        .alert("The file changed on disk", isPresented: $viewModel.confirmingOverwrite) {
            Button("Overwrite", role: .destructive) { viewModel.overwrite() }
            Button("Cancel", role: .cancel) {}
        } message: {
            Text(viewModel.overwriteMessage)
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
                    let rows = viewModel.rows
                    ForEach(rows.indices, id: \.self) { offset in
                        let row = rows[offset]
                        RowView(
                            row: row,
                            widths: widths,
                            scroll: scroll,
                            take: { viewModel.take($0, in: $1) },
                            editByHand: { viewModel.editByHand(in: $0) },
                            focus: { viewModel.focus($0) }
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
                    scroll.scroll(by: delta)
                }
            }
            .onChange(of: viewModel.currentConflict) { _, conflict in
                if let row = viewModel.scrollTarget(for: conflict) {
                    withAnimation { proxy.scrollTo(row, anchor: .top) }
                }
            }
        }
    }

    /// Far enough for the longest line to show whole in the narrowest pane:
    /// a side, a column or half the top area.
    private func maxScrollX(width: CGFloat) -> CGFloat {
        let widths = viewModel.layout == .stacked ? PaneWidths.stackedSides(total: width) : PaneWidths(total: width)
        let longest = CGFloat(viewModel.longestLine) * Metrics.codeCharacterWidth + Metrics.clipFadeWidth
        return max(longest - widths.code(widths.side), 0)
    }

    @ToolbarContentBuilder
    private var toolbar: some ToolbarContent {
        if viewModel.canGoBack {
            ToolbarItem(placement: .navigation) {
                Button { viewModel.back() } label: {
                    Label("Files", systemImage: "chevron.left")
                }
                .help("Back to the operation's files")
            }
        }
        ToolbarItemGroup {
            Button(action: { viewModel.previous() }) {
                Image(systemName: "chevron.up")
            }
            .keyboardShortcut(Shortcut.previousConflict)
            .help(Shortcut.previousConflict.help())
            .disabled(!viewModel.canGoToPrevious)

            Button(action: { viewModel.next() }) {
                Image(systemName: "chevron.down")
            }
            .keyboardShortcut(Shortcut.nextConflict)
            .help(Shortcut.nextConflict.help())
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
                .help(Shortcut.takeOurs.help())
                Button(action: { viewModel.takeBoth() }) {
                    HStack(spacing: 5) {
                        HStack(spacing: 2) {
                            SideShape.ours
                            SideShape.theirs
                        }
                        Text("Both")
                    }
                }
                .help(Shortcut.takeBoth.help("Take both, ours first; again for theirs first"))
                Button { viewModel.take("theirs") } label: {
                    HStack(spacing: 5) {
                        Text("Theirs")
                        SideShape.theirs
                    }
                }
                .help(Shortcut.takeTheirs.help())
            }
        }

        ToolbarItem {
            Toggle(isOn: Binding(get: { viewModel.showsBase }, set: { _ in viewModel.toggleBase() })) {
                HStack(spacing: 5) {
                    Image(systemName: "square.2.layers.3d")
                    Text("Base")
                }
            }
            .toggleStyle(.button)
            .disabled(!viewModel.canShowBase)
            .help(viewModel.baseHelp)
            .accessibilityLabel(viewModel.showsBase ? "Hide base" : "Show base")
        }

        ToolbarItem {
            Picker("Layout", selection: Binding(get: { viewModel.layout }, set: { viewModel.setLayout($0) })) {
                Image(systemName: "rectangle.split.3x1")
                    .accessibilityLabel("Three columns")
                    .tag(MergeEditorViewModel.EditorLayout.columns)
                Image(systemName: "rectangle.split.1x2")
                    .accessibilityLabel("Stacked: sides above the result")
                    .tag(MergeEditorViewModel.EditorLayout.stacked)
            }
            .pickerStyle(.segmented)
            .disabled(!viewModel.canSwitchLayout)
            .help(viewModel.layoutHelp)
        }

        ToolbarItem {
            Button { showInspector.toggle() } label: {
                Image(systemName: "sidebar.right")
            }
            .keyboardShortcut(Shortcut.toggleInspector)
            .help(Shortcut.toggleInspector.help())
        }

        ToolbarItem(placement: .primaryAction) {
            Button(action: { viewModel.markResolved() }) {
                HStack(spacing: 4) {
                    Image(systemName: "checkmark")
                    Text("Mark as Resolved")
                }
                .padding(.horizontal, 4)
            }
            .keyboardShortcut(Shortcut.markAsResolved)
            .buttonStyle(.borderedProminent)
            // While editing, ⌘↩ applies the edit instead.
            .disabled(!viewModel.canMarkResolved)
            .help(Shortcut.markAsResolved.help("Write the result"))
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
                .keyboardShortcut(Shortcut.takeOurs)
            Button("Take theirs") { viewModel.take("theirs") }
                .keyboardShortcut(Shortcut.takeTheirs)
            Button("Take both", action: { viewModel.takeBoth() })
                .keyboardShortcut(Shortcut.takeBoth)
            Button("Take base") { viewModel.take("base") }
                .keyboardShortcut(Shortcut.takeBase)
            Button("Take none") { viewModel.take("neither") }
                .keyboardShortcut(Shortcut.takeNone)
            // Disabled while editing, so ↩ types a line break and ⌘Z undoes
            // typing in the editor.
            Button("Edit by hand") { viewModel.startEditing() }
                .keyboardShortcut(Shortcut.editByHand)
                .disabled(viewModel.isEditing)
            Button("Undo", action: { viewModel.undo() })
                .keyboardShortcut(Shortcut.undo)
                .disabled(!viewModel.canUndo)
            Button("Redo", action: { viewModel.redo() })
                .keyboardShortcut(Shortcut.redo)
                .disabled(!viewModel.canRedo)
        }
        .frame(width: 0, height: 0)
        .opacity(0)
        .accessibilityHidden(true)
    }

}

/// Column widths: ours 1fr, connector, result 1.25fr, connector, theirs 1fr.
/// Stacked, each area has its own: the sides halves of the top area with a
/// hairline between them, the result the whole bottom area.
struct PaneWidths {
    let side: CGFloat
    let result: CGFloat
    let connector: CGFloat

    init(total: CGFloat) {
        let flexible = max(total - 2 * Metrics.connectorWidth, 0)
        side = flexible / 3.25
        result = flexible * 1.25 / 3.25
        connector = Metrics.connectorWidth
    }

    init(side: CGFloat, result: CGFloat, connector: CGFloat) {
        self.side = side
        self.result = result
        self.connector = connector
    }

    /// The stacked layout's top area: ours and theirs, 1 pt apart.
    static func stackedSides(total: CGFloat) -> PaneWidths {
        PaneWidths(side: max(total - 1, 0) / 2, result: 0, connector: 1)
    }

    /// The stacked layout's bottom area: the result, full width.
    static func stackedResult(total: CGFloat) -> PaneWidths {
        PaneWidths(side: 0, result: total, connector: 0)
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
    /// The headers of the panes shown: all three, or one stacked area.
    var part: RowPart = .all

    var body: some View {
        HStack(alignment: .top, spacing: 0) {
            if part != .result {
                header(ours, color: Theme.Sides.ours, symbol: "circle.fill")
                    .frame(width: widths.side)
                if part == .sides {
                    Rectangle().fill(Theme.Surfaces.hairline).frame(width: widths.connector)
                } else {
                    Color.clear.frame(width: widths.connector)
                }
            }
            if part != .sides {
                header(result, color: Theme.Sides.result, symbol: "diamond.fill")
                    .frame(width: widths.result)
            }
            if part != .result {
                if part == .all { Color.clear.frame(width: widths.connector) }
                header(theirs, color: Theme.Sides.theirs, symbol: "square.fill")
                    .frame(width: widths.side)
            }
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

#Preview("Editor, stacked", traits: .fixedLayout(width: 1440, height: 700)) {
    if let editor = PreviewSample.editor({ editor in
        editor.setLayout(.stacked)
        editor.take("theirs")
    }) {
        MergeEditorView(viewModel: editor)
    }
}

#Preview("Editor, narrow window", traits: .fixedLayout(width: 1000, height: 700)) {
    if let editor = PreviewSample.editor({ $0.windowWidthChanged(1000) }) {
        MergeEditorView(viewModel: editor)
    }
}

#Preview("Editor, base shown", traits: .fixedLayout(width: 1440, height: 600)) {
    if let editor = PreviewSample.editor({ editor in
        editor.toggleBase()
        editor.take("theirs")
    }) {
        MergeEditorView(viewModel: editor)
    }
}

#Preview("Editor, stacked with base", traits: .fixedLayout(width: 1440, height: 700)) {
    if let editor = PreviewSample.editor({ editor in
        editor.setLayout(.stacked)
        editor.toggleBase()
    }) {
        MergeEditorView(viewModel: editor)
    }
}
