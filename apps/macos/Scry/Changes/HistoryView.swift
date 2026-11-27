import SwiftUI

/// Back and Forward in the window that has them: the places the window
/// showed, as a browser keeps its pages.
struct TrailActions {
    var canGoBack = false
    var canGoForward = false
    /// "Back to the history of SyncEngine.swift".
    var backHelp = "Back"
    var forwardHelp = "Forward"
    var back: () -> Void = {}
    var forward: () -> Void = {}
}

extension EnvironmentValues {
    @Entry var trail: TrailActions?
}

/// The toolbar's Back and Forward, at its leading edge.
struct TrailToolbar: ToolbarContent {
    let trail: TrailActions?

    var body: some ToolbarContent {
        if let trail {
            ToolbarItemGroup(placement: .navigation) {
                Button(action: trail.back) {
                    Image(systemName: "chevron.backward")
                }
                .keyboardShortcut(Shortcut.goBack)
                .help(Shortcut.goBack.help(trail.backHelp))
                .accessibilityLabel(trail.backHelp)
                .disabled(!trail.canGoBack)

                Button(action: trail.forward) {
                    Image(systemName: "chevron.forward")
                }
                .keyboardShortcut(Shortcut.goForward)
                .help(Shortcut.goForward.help(trail.forwardHelp))
                .accessibilityLabel(trail.forwardHelp)
                .disabled(!trail.canGoForward)
            }
        }
    }
}

/// S18, one file's history: its points in the sidebar, in place of the
/// changed files, and the file at the selected point.
struct HistoryView: View {
    let viewModel: HistoryViewModel
    /// How far the code is scrolled sideways, shared by every row.
    @State private var scroll = SideScroll()
    /// No for a preview, which loaded its points already.
    var loads = true
    @Environment(\.trail) private var trail

    var body: some View {
        HStack(spacing: 0) {
            HistorySidebar(viewModel: viewModel)
                .frame(width: ChangesView.sidebarWidth)
            Rectangle().fill(Theme.Surfaces.hairline).frame(width: 1)
            fileArea
        }
        .background(Theme.Surfaces.pane)
        .overlay(alignment: .bottom) { HintStrip(shortcuts: viewModel.hintStrip) }
        .overlay(alignment: .top) { ToolbarHintRow(shortcuts: viewModel.toolbarHints) }
        .navigationTitle(viewModel.header.repository)
        .navigationSubtitle("History of \(viewModel.header.name)")
        .toolbar {
            TrailToolbar(trail: trail)
            ChangesToolbar(
                viewModel: viewModel.file,
                history: HistoryButton(isOn: true, enabled: true) { viewModel.showChangedFiles() },
                reload: { Task { await viewModel.load() } }
            )
        }
        .focusedSceneValue(\.changesActions, viewModel.file.map { file in
            ChangesActions(layout: file.layout, toggleLayout: { file.layout = file.layout == .unified ? .split : .unified })
        })
        .task {
            if loads { await viewModel.loadOnce() }
        }
    }

    @ViewBuilder
    private var fileArea: some View {
        if let file = viewModel.file {
            ChangesFileArea(
                viewModel: file, scroll: scroll,
                trailing: viewModel.canShowWholeCommit
                    ? AnyView(Button("Show the Whole Commit") { viewModel.showWholeCommit() }
                        .help("Every file this commit changed"))
                    : nil
            )
            // Another point is another file to read from its first block.
            .id(ObjectIdentifier(file))
        } else if let error = viewModel.error {
            ChangesCard(symbol: "exclamationmark.triangle", title: "Scry could not read this history", message: error)
        } else if viewModel.loading || viewModel.selected != nil {
            ProgressView()
                .controlSize(.large)
                .frame(maxWidth: .infinity, maxHeight: .infinity)
        } else {
            ChangesCard(
                symbol: "clock.arrow.circlepath", title: "No history",
                message: "No commit touched \(viewModel.header.path), and nothing of it is not committed."
            )
        }
    }
}

/// The points of the file's history, newest first, with the two modes
/// above them.
struct HistorySidebar: View {
    let viewModel: HistoryViewModel

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            header
            Rectangle().fill(Theme.Surfaces.hairline).frame(height: 1)
            if viewModel.points.isEmpty {
                Text(viewModel.loading ? "" : "No point")
                    .font(Metrics.callout)
                    .foregroundStyle(Theme.Surfaces.textTertiary)
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else {
                list
            }
        }
        .background(Theme.Surfaces.sidebar)
    }

    private var header: some View {
        VStack(alignment: .leading, spacing: 6) {
            Button { viewModel.showChangedFiles() } label: {
                HStack(spacing: 3) {
                    Image(systemName: "chevron.backward").font(.system(size: 9, weight: .semibold))
                    Text("Changed files")
                }
                .font(Metrics.caption)
                .foregroundStyle(Theme.Interaction.accent)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .help(Shortcut.history.help("Back to the changed files"))
            VStack(alignment: .leading, spacing: 2) {
                (Text("History of ").font(Metrics.headline)
                    + Text(viewModel.header.name).font(.system(size: 13, weight: .semibold, design: .monospaced)))
                    .lineLimit(1)
                    .truncationMode(.middle)
                    .help(viewModel.header.path)
                Text(viewModel.summary)
                    .font(Metrics.caption)
                    .foregroundStyle(Theme.Surfaces.textSecondary)
                    .monospacedDigit()
            }
            Picker("Mode", selection: Binding(
                get: { viewModel.mode },
                set: { mode in Task { await viewModel.setMode(mode) } }
            )) {
                Text("Each change").tag(HistoryViewModel.Mode.eachChange)
                Text("Against a pin").tag(HistoryViewModel.Mode.againstPin)
            }
            .pickerStyle(.segmented)
            .labelsHidden()
            .disabled(!viewModel.canPin)
            Text(viewModel.modeSentence)
                .font(Metrics.caption)
                .foregroundStyle(Theme.Surfaces.textSecondary)
                .fixedSize(horizontal: false, vertical: true)
            HStack(spacing: 2) {
                Button { Task { await viewModel.older() } } label: {
                    Label("Older", systemImage: "arrow.down")
                }
                .keyboardShortcut(Shortcut.olderPoint)
                .help(Shortcut.olderPoint.help())
                .disabled(!viewModel.canGoToOlder)
                Button { Task { await viewModel.newer() } } label: {
                    Label("Newer", systemImage: "arrow.up")
                }
                .keyboardShortcut(Shortcut.newerPoint)
                .help(Shortcut.newerPoint.help())
                .disabled(!viewModel.canGoToNewer)
                Spacer()
                Button { Task { await viewModel.togglePinOnSelected() } } label: {
                    Label(selectedIsPinned ? "Unpin" : "Pin", systemImage: selectedIsPinned ? "pin.slash" : "pin")
                }
                .keyboardShortcut(Shortcut.pinPoint)
                .help(Shortcut.pinPoint.help(
                    selectedIsPinned ? "Unpin: back to each change" : "Pin the selected point: compare every point with it"
                ))
                .disabled(!viewModel.canPinSelected)
            }
            .labelStyle(.titleAndIcon)
            .controlSize(.small)
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 10)
    }

    private var selectedIsPinned: Bool {
        viewModel.selected.map(viewModel.isPinned) ?? false
    }

    private var list: some View {
        ScrollViewReader { proxy in
            ScrollView(.vertical) {
                LazyVStack(alignment: .leading, spacing: 0) {
                    ForEach(viewModel.points, id: \.index) { point in
                        HistoryPointRow(
                            point: point,
                            detail: viewModel.detail(point),
                            counts: viewModel.counts(point),
                            selected: point.index == viewModel.selected?.index,
                            pinned: viewModel.isPinned(point),
                            first: point.index == viewModel.points.first?.index,
                            last: point.index == viewModel.points.last?.index && !viewModel.more
                        ) { Task { await viewModel.select(point) } }
                            .contextMenu { menu(point) }
                            .id(point.index)
                    }
                    if viewModel.more { older }
                }
                .padding(.vertical, 6)
            }
            .focusable()
            .focusEffectDisabled()
            .onKeyPress(keys: [.upArrow, .downArrow], phases: [.down, .repeat]) { press in
                Task { press.key == .upArrow ? await viewModel.newer() : await viewModel.older() }
                return .handled
            }
            .onChange(of: viewModel.selected?.index) { _, index in
                if let index { proxy.scrollTo(index) }
            }
        }
    }

    /// The end of the loaded page: more is loaded when it comes in view.
    private var older: some View {
        VStack(spacing: 4) {
            Button("Load \(50) Older") { Task { await viewModel.loadOlder() } }
                .controlSize(.small)
                .disabled(viewModel.loadingOlder)
            if let loaded = viewModel.loadedSummary {
                Text(loaded)
                    .font(Metrics.caption)
                    .foregroundStyle(Theme.Surfaces.textSecondary)
                    .monospacedDigit()
            }
        }
        .frame(maxWidth: .infinity)
        .padding(.vertical, 10)
        .onAppear { Task { await viewModel.loadOlder() } }
    }

    @ViewBuilder
    private func menu(_ point: HistoryPoint) -> some View {
        if viewModel.canPin(point) {
            Button(viewModel.isPinned(point) ? "Unpin" : point.kind == .notCommitted ? "Pin the Working Copy" : "Pin This Commit") {
                Task { await viewModel.togglePin(point) }
            }
        }
        if point.kind == .commit {
            Button("Show the Whole Commit") { viewModel.showWholeCommit(point) }
            Divider()
            Button("Copy the Commit's ID") {
                NSPasteboard.general.clearContents()
                NSPasteboard.general.setString(point.id, forType: .string)
            }
        }
    }
}

/// A point on the rail: what it is, who and when, and its counts.
struct HistoryPointRow: View {
    let point: HistoryPoint
    let detail: String
    let counts: (added: UInt32?, removed: UInt32?)
    let selected: Bool
    let pinned: Bool
    /// The rail starts, or ends, at this point.
    var first = false
    var last = false
    var select: () -> Void = {}

    static let rail: CGFloat = 22

    var body: some View {
        Button(action: select) {
            HStack(alignment: .top, spacing: 6) {
                node
                VStack(alignment: .leading, spacing: 2) {
                    HStack(alignment: .firstTextBaseline, spacing: 6) {
                        Text(HistoryViewModel.title(point))
                            .font(.system(size: 12, weight: selected ? .semibold : .regular))
                            .italic(point.kind != .commit)
                            .foregroundStyle(Theme.Surfaces.text)
                            .lineLimit(1)
                        Spacer(minLength: 4)
                        ChangesCounts(added: counts.added, removed: counts.removed)
                    }
                    HStack(spacing: 5) {
                        Text(detail)
                            .font(Metrics.caption)
                            .foregroundStyle(Theme.Surfaces.textSecondary)
                            .monospacedDigit()
                            .lineLimit(1)
                        if point.head { tag("HEAD") }
                        if pinned { tag("pinned") }
                    }
                    if let oldPath = point.oldPath {
                        // The path is cut at its head, not the words before it.
                        HStack(spacing: 3) {
                            Text("Renamed from").fixedSize()
                            Text(oldPath).lineLimit(1).truncationMode(.head)
                        }
                        .font(Metrics.caption)
                        .foregroundStyle(Theme.Surfaces.textSecondary)
                    }
                }
                .padding(.vertical, 6)
            }
            // The rail runs behind the node, through the row's whole
            // height, but not past the ends of the list.
            .background(alignment: .leading) {
                VStack(spacing: 0) {
                    Rectangle().fill(first ? Color.clear : Theme.Surfaces.controlBorder).frame(width: 1, height: 13)
                    Rectangle().fill(last ? Color.clear : Theme.Surfaces.controlBorder).frame(width: 1)
                }
                .frame(width: Self.rail)
            }
            .padding(.leading, 4)
            .padding(.trailing, 8)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .background(RoundedRectangle(cornerRadius: 6).fill(selected ? Theme.Interaction.selection : Color.clear))
        .padding(.horizontal, 4)
        .accessibilityAddTraits(selected ? .isSelected : [])
        .help(point.kind == .commit ? "\(point.subject)\n\(point.shortId) · \(point.author) · \(HistoryViewModel.date(point))" : detail)
    }

    private func tag(_ text: String) -> some View {
        Text(text)
            .font(.system(size: 10, weight: .medium))
            .foregroundStyle(Theme.Interaction.accent)
            .padding(.horizontal, 5)
            .frame(height: 15)
            .overlay(Capsule().stroke(Theme.Interaction.accent.opacity(0.6), lineWidth: 1))
            .fixedSize()
    }

    /// The point's node on the rail: dashed off a commit, a pin when
    /// pinned, ringed when selected.
    private var node: some View {
        ZStack(alignment: .top) {
            Group {
                if pinned {
                    Image(systemName: "pin.fill")
                        .font(.system(size: 9))
                        .foregroundStyle(Theme.Interaction.accent)
                        .frame(width: 13, height: 13)
                        .background(Circle().fill(Theme.Surfaces.sidebar))
                } else if point.kind == .commit {
                    Circle()
                        .fill(selected ? Theme.Interaction.accent : Theme.Surfaces.sidebar)
                        .overlay(Circle().stroke(
                            selected ? Theme.Interaction.accent : Theme.Surfaces.textSecondary, lineWidth: 1.5
                        ))
                        .frame(width: 9, height: 9)
                } else {
                    Circle()
                        .fill(Theme.Surfaces.sidebar)
                        .overlay(Circle().stroke(
                            selected ? Theme.Interaction.accent : Theme.Surfaces.textSecondary,
                            style: StrokeStyle(lineWidth: 1.5, dash: [2.5, 2])
                        ))
                        .frame(width: 9, height: 9)
                }
            }
            .frame(width: Self.rail, height: 13)
            .padding(.top, 7)
        }
        .frame(width: Self.rail)
    }
}
