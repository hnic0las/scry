import SwiftUI

/// S17, the Changes window: the changed files on the left, the selected
/// file whole in the middle, unified or split, with its change map.
struct ChangesView: View {
    @Bindable var viewModel: ChangesViewModel
    /// How far the code is scrolled sideways, shared by every row.
    @State private var scroll = SideScroll()
    /// No for a preview, which loaded its files already.
    var loads = true
    /// The line saying what `git difftool`'s window lacks, until hidden.
    @State private var showsReducedNote = true

    static let sidebarWidth: CGFloat = 284

    @Environment(\.trail) private var trail

    var body: some View {
        VStack(spacing: 0) {
            if viewModel.header.reduced, showsReducedNote {
                ReducedModeNote { showsReducedNote = false }
                Rectangle().fill(Theme.Surfaces.hairline).frame(height: 1)
            }
            HStack(spacing: 0) {
                // Two files from git difftool: nothing to choose from.
                if !viewModel.header.singleFile {
                    ChangesSidebar(viewModel: viewModel)
                        .frame(width: Self.sidebarWidth)
                    Rectangle().fill(Theme.Surfaces.hairline).frame(width: 1)
                }
                ChangesFileArea(viewModel: viewModel, scroll: scroll)
            }
        }
        .background(Theme.Surfaces.pane)
        .overlay(alignment: .bottom) { HintStrip(shortcuts: viewModel.hintStrip) }
        .overlay(alignment: .top) { ToolbarHintRow(shortcuts: viewModel.toolbarHints) }
        .navigationTitle(viewModel.header.repository)
        .navigationSubtitle(viewModel.header.sentence)
        .toolbar {
            // git difftool's window has one place: nowhere to go back to.
            TrailToolbar(trail: viewModel.header.reduced ? nil : trail)
            ChangesToolbar(
                viewModel: viewModel,
                history: viewModel.header.hasHistory
                    ? HistoryButton(isOn: false, enabled: viewModel.canShowHistory) { viewModel.showHistory() }
                    : nil,
                // git difftool's files are copies: there is nothing to read again.
                reload: viewModel.header.reduced ? nil : { Task { await viewModel.load() } }
            )
        }
        .focusedSceneValue(\.changesActions, ChangesActions(
            layout: viewModel.layout,
            toggleLayout: { viewModel.layout = viewModel.layout == .unified ? .split : .unified }
        ))
        .task {
            if loads {
                await viewModel.load()
                await viewModel.watch()
            }
        }
    }
}

/// The selected file of a change: its header, its rows or what stands
/// for them, and the status bar.
struct ChangesFileArea: View {
    let viewModel: ChangesViewModel
    /// How far the code is scrolled sideways, shared by every row.
    let scroll: SideScroll
    /// More buttons at the end of the file's header.
    var trailing: AnyView?

    var body: some View {
        VStack(spacing: 0) {
            if let file = viewModel.selected {
                ChangesFileHeader(viewModel: viewModel, file: file, trailing: trailing)
                Rectangle().fill(Theme.Surfaces.hairline).frame(height: 1)
            }
            HStack(spacing: 0) {
                content
                if viewModel.showsInspector {
                    Rectangle().fill(Theme.Surfaces.hairline).frame(width: 1)
                    ChangesInspector(viewModel: viewModel)
                        .frame(width: Metrics.inspectorWidth)
                }
            }
            ChangesStatusBar(
                status: viewModel.status, live: viewModel.header.live, updatedAt: viewModel.updatedAt,
                language: viewModel.language, watching: viewModel.watching
            )
        }
        // Blame is the window's: a file shown again follows it.
        .onChange(of: viewModel.blameSwitch.isOn, initial: true) { viewModel.updateBlame() }
        .background { hiddenShortcuts }
        // The agent answers from outside the app: where Explain stands on
        // the file's lines, and whether a session watches, is read every
        // second.
        .task(id: pollKey) {
            guard viewModel.pollsExplanations, viewModel.canSelectLines else { return }
            while !Task.isCancelled {
                await viewModel.refreshExplanations()
                try? await Task.sleep(for: .seconds(1))
            }
        }
    }

    /// The file whose explanations are read: another file, or other
    /// content, starts the reading again.
    private var pollKey: String {
        "\(viewModel.selected?.path ?? "")|\(viewModel.selected?.contentId ?? "")|\(viewModel.canSelectLines)"
    }

    /// ⌃⌘E asks about the selected lines; esc clears the selection.
    private var hiddenShortcuts: some View {
        Group {
            Button("Explain the selected lines") { Task { await viewModel.explainSelection() } }
                .keyboardShortcut(Shortcut.explainLines)
                .disabled(viewModel.selection == nil)
            if viewModel.selection != nil {
                Button("Clear the selection") { viewModel.clearSelection() }
                    .keyboardShortcut(.cancelAction)
            }
        }
        .frame(width: 0, height: 0)
        .opacity(0)
        .accessibilityHidden(true)
    }

    @ViewBuilder
    private var content: some View {
        switch viewModel.content {
        case .text(let diff):
            DiffFileView(viewModel: viewModel, diff: diff, scroll: scroll)
        case .loading:
            ProgressView()
                .controlSize(.large)
                .frame(maxWidth: .infinity, maxHeight: .infinity)
        case .none:
            if viewModel.loadingFiles {
                ProgressView()
                    .controlSize(.large)
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else {
                ChangesCard(
                    symbol: viewModel.listError == nil ? "checkmark.circle" : "exclamationmark.triangle",
                    title: viewModel.listError == nil ? "Nothing to show" : "Scry could not read the changes",
                    message: viewModel.listError ?? viewModel.emptyMessage
                )
            }
        case .binary(let old, let new):
            ChangesCard(
                symbol: "doc",
                title: "Binary file",
                message: "Scry does not show a line-by-line difference for a binary file.\n\(ChangesViewModel.size(old)) → \(ChangesViewModel.size(new))"
            )
        case .submodule:
            ChangesCard(
                symbol: "shippingbox",
                title: "A repository of its own",
                message: "This folder is another git repository. Open it in Scry to read its changes."
            )
        case .conflicted:
            ChangesCard(
                symbol: "exclamationmark.triangle",
                title: "This file has conflicts",
                message: "A merge is in progress and git left conflict markers in the file. A difference of the markers would not mean much: resolve the conflicts first.",
                action: ("Open in the Merge Editor", { viewModel.openInMergeEditor() })
            )
        case .failed(let message):
            ChangesCard(symbol: "exclamationmark.triangle", title: "Scry could not read this file", message: message)
        }
    }
}

/// The toolbar's History button: on while a file's history is shown.
struct HistoryButton {
    let isOn: Bool
    let enabled: Bool
    let action: () -> Void
}

/// The toolbar of a file being read: previous and next change, the
/// layout, History, and reading again. `viewModel` is `nil` while a
/// history has no file to show yet.
struct ChangesToolbar: ToolbarContent {
    let viewModel: ChangesViewModel?
    var history: HistoryButton?
    /// `nil` where there is nothing to read again.
    var reload: (() -> Void)?

    var body: some ToolbarContent {
        ToolbarItemGroup {
            Button { Task { await viewModel?.previousChange() } } label: {
                Image(systemName: "chevron.up")
            }
            .keyboardShortcut(Shortcut.previousChange)
            .help(Shortcut.previousChange.help())
            .disabled(viewModel?.canGoToPreviousChange != true)

            Button { Task { await viewModel?.nextChange() } } label: {
                Image(systemName: "chevron.down")
            }
            .keyboardShortcut(Shortcut.nextChange)
            .help(Shortcut.nextChange.help())
            .disabled(viewModel?.canGoToNextChange != true)
        }
        ToolbarItem {
            Text(viewModel?.changeCounter ?? "")
                .font(Metrics.callout)
                .foregroundStyle(Theme.Surfaces.textSecondary)
                .monospacedDigit()
                .padding(.horizontal, 4)
        }
        ToolbarItem {
            Picker("Layout", selection: Binding(
                get: { viewModel?.layout ?? .unified },
                set: { viewModel?.layout = $0 }
            )) {
                Label("Unified", systemImage: "rectangle").tag(ChangesViewModel.Layout.unified)
                Label("Split", systemImage: "rectangle.split.2x1").tag(ChangesViewModel.Layout.split)
            }
            .pickerStyle(.segmented)
            .labelStyle(.titleAndIcon)
            .help(Shortcut.changesLayout.help())
            .disabled(viewModel == nil)
        }
        if let viewModel, !viewModel.header.reduced {
            ToolbarItem {
                Toggle(isOn: Binding(get: { viewModel.blameSwitch.isOn }, set: { _ in viewModel.toggleBlame() })) {
                    Label("Blame", systemImage: "text.alignleft")
                }
                .labelStyle(.titleAndIcon)
                .keyboardShortcut(Shortcut.blame)
                .help(Shortcut.blame.help(viewModel.blameSwitch.isOn ? "Hide blame" : "Who last wrote each line"))
                .disabled(!viewModel.canBlame && !viewModel.blameSwitch.isOn)
            }
        }
        if let history {
            ToolbarItem {
                Toggle(isOn: Binding(get: { history.isOn }, set: { _ in history.action() })) {
                    Label("History", systemImage: "clock.arrow.circlepath")
                }
                .labelStyle(.titleAndIcon)
                .keyboardShortcut(Shortcut.history)
                .help(Shortcut.history.help(history.isOn ? "Back to the changed files" : "History of this file"))
                .disabled(!history.enabled)
            }
        }
        if let viewModel, viewModel.canSelectLines {
            ToolbarItem {
                // The key is the file area's: it also works with this
                // button out of the toolbar.
                Button { Task { await viewModel.explainSelection() } } label: {
                    Label("Explain", systemImage: "sparkles")
                }
                .labelStyle(.titleAndIcon)
                .help(
                    viewModel.selection == nil
                        ? "Click line numbers to select lines, then ask Claude Code to explain them"
                        : Shortcut.explainLines.help("Ask Claude Code what the selected lines do and why they changed")
                )
                .disabled(viewModel.selection == nil)
            }
        }
        if let reload {
            ToolbarItem {
                Button(action: reload) {
                    Image(systemName: "arrow.clockwise")
                }
                .keyboardShortcut(Shortcut.refreshChanges)
                .help(Shortcut.refreshChanges.help())
            }
        }
        if let viewModel, viewModel.canShowInspector {
            ToolbarItem {
                Toggle(isOn: Binding(get: { viewModel.inspectorShown }, set: { _ in viewModel.toggleInspector() })) {
                    Image(systemName: "sidebar.right")
                }
                .keyboardShortcut(Shortcut.toggleInspector)
                .help(Shortcut.toggleInspector.help(viewModel.inspectorShown ? "Hide the inspector" : "Show the inspector"))
            }
        }
    }
}

/// S20: one line, in neutral colours, saying why this window has less
/// than the Changes window, and how to get the rest.
struct ReducedModeNote: View {
    let hide: () -> Void

    static let command = "git scry diff"

    var body: some View {
        HStack(spacing: 10) {
            (Text("Opened by git difftool.").bold()
                + Text(" Git gave Scry two files and nothing else, so History, Blame and Explain are not available in this window, and git opens one window per file. ")
                + Text(Self.command).font(Metrics.codeSmall)
                + Text(" opens the whole change with all of them."))
                .font(Metrics.caption)
                .foregroundStyle(Theme.Surfaces.textSecondary)
                .lineLimit(2)
            Spacer(minLength: 8)
            Button("Copy Command") {
                NSPasteboard.general.clearContents()
                NSPasteboard.general.setString(Self.command, forType: .string)
            }
            .controlSize(.small)
            Button(action: hide) {
                Image(systemName: "xmark")
            }
            .buttonStyle(.borderless)
            .help("Hide")
            .accessibilityLabel("Hide")
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 6)
        .background(Theme.Surfaces.chrome)
    }
}

/// What View ▸ Switch Layout does in the Changes window that has the focus.
struct ChangesActions {
    let layout: ChangesViewModel.Layout
    let toggleLayout: () -> Void
}

extension FocusedValues {
    @Entry var changesActions: ChangesActions?
}

/// The list of changed files, with what is compared above it.
struct ChangesSidebar: View {
    @Bindable var viewModel: ChangesViewModel

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            VStack(alignment: .leading, spacing: 3) {
                Text(viewModel.header.title)
                    .font(Metrics.headline)
                    .lineLimit(1)
                Text(viewModel.scopeDetail)
                    .font(Metrics.caption)
                    .foregroundStyle(Theme.Surfaces.textSecondary)
                    .lineLimit(2)
                if !viewModel.files.isEmpty {
                    Text(viewModel.listSummary)
                        .font(Metrics.caption)
                        .foregroundStyle(Theme.Surfaces.textSecondary)
                        .monospacedDigit()
                    if viewModel.canMarkViewed {
                        ViewedProgress(share: viewModel.viewedShare, summary: viewModel.viewedSummary)
                            .padding(.top, 3)
                    }
                    ListModePicker(mode: $viewModel.listMode)
                        .padding(.top, 6)
                }
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 10)
            Rectangle().fill(Theme.Surfaces.hairline).frame(height: 1)
            if viewModel.files.isEmpty, !viewModel.loadingFiles {
                Text("No file")
                    .font(Metrics.callout)
                    .foregroundStyle(Theme.Surfaces.textTertiary)
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else {
                ScrollView(.vertical) {
                    LazyVStack(alignment: .leading, spacing: 0) {
                        ForEach(viewModel.sections) { section in
                            if let title = section.title, let group = section.group {
                                ChangesSectionHeader(
                                    title: title, count: section.files.count, collapsed: section.collapsed
                                ) { viewModel.toggle(group) }
                                    .contextMenu {
                                        if viewModel.canMarkViewed {
                                            Button("Mark All as Viewed") { viewModel.setViewed(in: group, true) }
                                                .disabled(section.files.allSatisfy { $0.viewed == .viewed })
                                            Button("Mark All as Not Viewed") { viewModel.setViewed(in: group, false) }
                                                .disabled(!section.files.contains { $0.viewed == .viewed })
                                        }
                                    }
                            }
                            ForEach(section.collapsed ? [] : section.rows) { row in
                                switch row {
                                case let .folder(folder):
                                    ChangesFolderRow(folder: folder) { viewModel.toggle(folder) }
                                        .contextMenu {
                                            if viewModel.canMarkViewed {
                                                let files = viewModel.files(in: folder)
                                                Button("Mark All as Viewed") { viewModel.setViewed(in: folder, true) }
                                                    .disabled(files.allSatisfy { $0.viewed == .viewed })
                                                Button("Mark All as Not Viewed") {
                                                    viewModel.setViewed(in: folder, false)
                                                }
                                                .disabled(!files.contains { $0.viewed == .viewed })
                                            }
                                        }
                                case let .file(file, depth):
                                    ChangesFileRow(
                                        file: file, selected: file == viewModel.selected,
                                        arrival: viewModel.arrived[file.path],
                                        depth: depth, underFolder: viewModel.listMode == .folders,
                                        select: { Task { await viewModel.select(file) } },
                                        toggleViewed: viewModel.canMarkViewed ? { viewModel.toggleViewed(file) } : nil
                                    )
                                }
                            }
                        }
                    }
                    .padding(.bottom, 8)
                }
            }
        }
        .background(Theme.Surfaces.sidebar)
    }
}

/// The sidebar's choice between the flat list and the list by folder,
/// as the design's two segments, with their names.
struct ListModePicker: View {
    @Binding var mode: ChangesViewModel.ListMode

    var body: some View {
        Picker("Show the files", selection: $mode) {
            Label("List", systemImage: "list.bullet").tag(ChangesViewModel.ListMode.flat)
            Label("Folders", systemImage: "folder").tag(ChangesViewModel.ListMode.folders)
        }
        .pickerStyle(.segmented)
        .labelStyle(.titleAndIcon)
        .labelsHidden()
        .controlSize(.small)
        .help("Show the files one after the other, or under their folders")
    }
}

/// A folder of the list by folder: a click hides its files, or shows
/// them.
struct ChangesFolderRow: View {
    let folder: ChangesViewModel.Folder
    let toggle: () -> Void

    static let height: CGFloat = 24
    /// How far a row moves right for each folder above it.
    static let indent: CGFloat = 12

    /// "6 · +148 −31": the files under it and their lines.
    private var counts: String {
        var parts = ["\(folder.count)"]
        var lines: [String] = []
        if folder.added > 0 { lines.append("+\(folder.added)") }
        if folder.removed > 0 { lines.append("−\(folder.removed)") }
        if !lines.isEmpty { parts.append(lines.joined(separator: " ")) }
        return parts.joined(separator: " · ")
    }

    var body: some View {
        Button(action: toggle) {
            HStack(spacing: 5) {
                Image(systemName: "chevron.right")
                    .font(.system(size: 8, weight: .semibold))
                    .rotationEffect(.degrees(folder.collapsed ? 0 : 90))
                    .frame(width: 10)
                Image(systemName: "folder")
                    .font(.system(size: 11))
                Text(folder.name)
                    .font(.system(size: 12, design: .monospaced))
                    .lineLimit(1)
                    .truncationMode(.head)
                Spacer(minLength: 4)
                Text(counts)
                    .font(Metrics.caption)
                    .monospacedDigit()
                    .foregroundStyle(Theme.Surfaces.textTertiary)
                    .fixedSize()
            }
            .foregroundStyle(Theme.Surfaces.textSecondary)
            .padding(.leading, 12 + CGFloat(folder.depth) * Self.indent)
            .padding(.trailing, 12)
            .frame(height: Self.height)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .help(folder.path)
        .accessibilityLabel("\(folder.name), \(folder.count == 1 ? "1 file" : "\(folder.count) files")")
        .accessibilityValue(folder.collapsed ? "collapsed" : "expanded")
    }
}

/// A group's title and count: a click hides its files, or shows them.
struct ChangesSectionHeader: View {
    let title: String
    let count: Int
    let collapsed: Bool
    let toggle: () -> Void

    var body: some View {
        Button(action: toggle) {
            HStack(spacing: 4) {
                Image(systemName: "chevron.right")
                    .font(.system(size: 9, weight: .semibold))
                    .rotationEffect(.degrees(collapsed ? 0 : 90))
                    .frame(width: 10)
                Text(title)
                Spacer()
                Text("\(count)").monospacedDigit()
            }
            .font(Metrics.captionBold)
            .foregroundStyle(Theme.Surfaces.textSecondary)
            .padding(.horizontal, 12)
            .padding(.top, 10)
            .padding(.bottom, 4)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .help(collapsed ? "Show these files" : "Hide these files")
        .accessibilityLabel("\(title), \(count == 1 ? "1 file" : "\(count) files")")
        .accessibilityValue(collapsed ? "collapsed" : "expanded")
    }
}

/// "partly staged", on a file whose change is half in the index.
struct PartlyStagedTag: View {
    var body: some View {
        Text("partly staged")
            .font(.system(size: 10, weight: .medium))
            .foregroundStyle(Theme.States.staged)
            .padding(.horizontal, 5)
            .frame(height: 15)
            .overlay(Capsule().stroke(Theme.States.staged.opacity(0.6), lineWidth: 1))
            .fixedSize()
            .help(ChangesViewModel.partlyStagedHelp)
    }
}

/// A file's state as a letter in an outlined square: never colour alone.
struct ChangesBadge: View {
    let file: ChangesFile

    var body: some View {
        Text(ChangesViewModel.badge(file))
            .font(.system(size: 10, weight: .semibold, design: .monospaced))
            .foregroundStyle(color)
            .frame(width: 16, height: 16)
            .overlay(RoundedRectangle(cornerRadius: 3).stroke(color.opacity(0.7), lineWidth: 1))
            .help(ChangesViewModel.stateName(file))
            .accessibilityLabel(ChangesViewModel.stateName(file))
    }

    private var color: Color {
        if file.binary { return Theme.Surfaces.textSecondary }
        switch file.state {
        case .added, .untracked: return Theme.Diff.addedMark
        case .deleted: return Theme.Diff.removedMark
        case .conflicted: return Theme.States.unresolved
        case .modified: return Theme.Severity.info
        case .renamed, .typeChanged: return Theme.Severity.warning
        }
    }
}

/// "+16 −7", each count in its colour; nothing for a file not counted.
struct ChangesCounts: View {
    let added: UInt32?
    let removed: UInt32?

    var body: some View {
        HStack(spacing: 5) {
            if let added, added > 0 {
                Text("+\(added)").foregroundStyle(Theme.Diff.addedMark)
            }
            if let removed, removed > 0 {
                Text("−\(removed)").foregroundStyle(Theme.Diff.removedMark)
            }
        }
        .font(Metrics.lineNumber)
        .monospacedDigit()
    }
}

/// "7 of 13 viewed", with a thin bar of the share that is.
struct ViewedProgress: View {
    let share: Double
    let summary: String

    var body: some View {
        HStack(spacing: 8) {
            GeometryReader { geometry in
                Capsule().fill(Theme.Surfaces.hairline)
                Capsule().fill(Theme.Interaction.accent).frame(width: geometry.size.width * share)
            }
            .frame(height: 3)
            Text(summary)
                .font(Metrics.caption)
                .foregroundStyle(Theme.Surfaces.textSecondary)
                .monospacedDigit()
                .fixedSize()
        }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(summary)
    }
}

/// The "Viewed" box of a file: empty, checked, or dashed when the file
/// changed since it was viewed.
struct ViewedBox: View {
    let state: ViewedState
    let toggle: () -> Void

    static let changedHelp = "Changed since you viewed it. Click to mark it viewed with what it holds now."

    var body: some View {
        Button(action: toggle) {
            ViewedMark(state: state)
                .frame(width: 22, height: 22)
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .help(state == .viewed ? "Viewed: click to remove the mark" : state == .changedSince ? Self.changedHelp : "Mark as viewed")
        .accessibilityLabel("Viewed")
        .accessibilityValue(state == .viewed ? "yes" : state == .changedSince ? "changed since viewed" : "no")
        .accessibilityAddTraits(.isToggle)
    }
}

/// The box alone, 14 pt.
struct ViewedMark: View {
    let state: ViewedState

    var body: some View {
        let shape = RoundedRectangle(cornerRadius: 3.5)
        switch state {
        case .viewed:
            shape.fill(Theme.Interaction.accent)
                .frame(width: 14, height: 14)
                .overlay {
                    Image(systemName: "checkmark")
                        .font(.system(size: 9, weight: .bold))
                        .foregroundStyle(.white)
                }
        case .changedSince:
            shape.stroke(Theme.States.needsReview, style: StrokeStyle(lineWidth: 1.5, dash: [3, 2]))
                .frame(width: 14, height: 14)
        case .notViewed:
            shape.stroke(Theme.Surfaces.controlBorder, lineWidth: 1)
                .frame(width: 14, height: 14)
        }
    }
}

struct ChangesFileRow: View {
    let file: ChangesFile
    let selected: Bool
    /// What happened to the file while the window was open.
    var arrival: ChangesViewModel.Arrival?
    /// The folders above it in the list by folder.
    var depth = 0
    /// The list is by folder: the row does not say the folder again.
    var underFolder = false
    var select: () -> Void = {}
    /// `nil` where files cannot be marked viewed.
    var toggleViewed: (() -> Void)?

    static let height: CGFloat = 38
    /// With the name alone.
    static let shortHeight: CGFloat = 26

    private var detail: String {
        ChangesViewModel.detail(file, arrival: arrival, folder: !underFolder)
    }

    /// Not beside "changed since viewed", which needs the line: the file
    /// header has both.
    private var showsTag: Bool { file.partlyStaged && file.viewed != .changedSince }

    private var height: CGFloat {
        detail.isEmpty && !showsTag ? Self.shortHeight : Self.height
    }

    var body: some View {
        HStack(spacing: 4) {
            Button(action: select) {
                HStack(spacing: 8) {
                    ChangesBadge(file: file)
                    VStack(alignment: .leading, spacing: 1) {
                        Text(file.name)
                            .font(.system(size: 12, weight: .semibold, design: .monospaced))
                            .foregroundStyle(Theme.Surfaces.text)
                            .lineLimit(1)
                            .truncationMode(.middle)
                        // The tag sits under the name, which keeps the row's width.
                        if !detail.isEmpty || showsTag {
                            HStack(spacing: 5) {
                                Text(detail)
                                    .font(Metrics.caption)
                                    .foregroundStyle(marked ? Theme.States.needsReview : Theme.Surfaces.textSecondary)
                                    .lineLimit(1)
                                    .truncationMode(file.viewed == .changedSince ? .tail : .head)
                                if showsTag { PartlyStagedTag() }
                            }
                        }
                    }
                    Spacer(minLength: 4)
                    ChangesCounts(added: file.added, removed: file.removed)
                }
                .frame(height: height)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .accessibilityAddTraits(selected ? .isSelected : [])
            if let toggleViewed {
                ViewedBox(state: file.viewed, toggle: toggleViewed)
            }
        }
        .padding(.leading, 8 + CGFloat(depth) * ChangesFolderRow.indent)
        .padding(.trailing, toggleViewed == nil ? 8 : 4)
        .frame(height: height)
        .background(RoundedRectangle(cornerRadius: 6).fill(selected ? Theme.Interaction.selection : Color.clear))
        .padding(.horizontal, 4)
        .overlay(alignment: .leading) {
            // Changed or arrived while the window was open.
            if arrival != nil {
                Circle()
                    .fill(Theme.States.needsReview)
                    .frame(width: 5, height: 5)
                    .padding(.leading, 3)
                    .accessibilityLabel(arrival == .new ? "New" : "Changed")
            }
        }
        .help(file.path)
    }

    /// The second line says what happened, in the marker's colour.
    private var marked: Bool {
        file.viewed == .changedSince || arrival == .new
    }
}

/// The selected file's name and counts, with previous and next file.
struct ChangesFileHeader: View {
    let viewModel: ChangesViewModel
    let file: ChangesFile
    /// More buttons at the end, from the screen the header is in.
    var trailing: AnyView?

    static let height: CGFloat = 36

    var body: some View {
        HStack(spacing: 8) {
            ChangesBadge(file: file)
            Text(file.name)
                .font(.system(size: 13, weight: .semibold, design: .monospaced))
                .lineLimit(1)
            Text(file.directory)
                .font(Metrics.code)
                .foregroundStyle(Theme.Surfaces.textSecondary)
                .lineLimit(1)
                .truncationMode(.head)
            if let oldPath = file.oldPath {
                Text("renamed from \(oldPath)")
                    .font(Metrics.caption)
                    .foregroundStyle(Theme.Surfaces.textSecondary)
                    .lineLimit(1)
                    .truncationMode(.head)
            }
            if file.partlyStaged { PartlyStagedTag() }
            ChangesCounts(added: file.added, removed: file.removed)
            Spacer(minLength: 8)
            if let queue = viewModel.queue, let label = viewModel.queueLabel {
                // git difftool opens its files one window after the other.
                Text(label)
                    .font(Metrics.caption)
                    .foregroundStyle(Theme.Surfaces.textSecondary)
                    .monospacedDigit()
                if !queue.isLast {
                    Button("Stop") { viewModel.stopQueue() }
                        .help("Close this window and open none of the files left")
                }
                Button(queue.isLast ? "Done" : "Next File") { viewModel.nextInQueue() }
                    .keyboardShortcut(Shortcut.nextFile)
                    .help(queue.isLast
                        ? "Close this window: it is the last file"
                        : Shortcut.nextFile.help("Close this window: git opens the next file"))
            } else if !viewModel.header.singleFile {
                if let position = viewModel.filePosition {
                    Text(position)
                        .font(Metrics.caption)
                        .foregroundStyle(Theme.Surfaces.textSecondary)
                        .monospacedDigit()
                }
                Button { Task { await viewModel.previousFile() } } label: {
                    Image(systemName: "chevron.left")
                }
                .buttonStyle(.borderless)
                .keyboardShortcut(Shortcut.previousFile)
                .help(Shortcut.previousFile.help())
                .disabled(!viewModel.canGoToPreviousFile)
                Button { Task { await viewModel.nextFile() } } label: {
                    Image(systemName: "chevron.right")
                }
                .buttonStyle(.borderless)
                .keyboardShortcut(Shortcut.nextFile)
                .help(Shortcut.nextFile.help())
                .disabled(!viewModel.canGoToNextFile)
            }
            if viewModel.canMarkViewed {
                Button { Task { await viewModel.markViewedAndGoOn() } } label: {
                    HStack(spacing: 5) {
                        ViewedMark(state: file.viewed)
                        Text("Viewed")
                    }
                }
                .keyboardShortcut(Shortcut.viewed)
                .help(Shortcut.viewed.help(
                    file.viewed == .viewed ? "Remove the mark" : "Mark as viewed, and go to the next file that is not"
                ))
                .accessibilityValue(file.viewed == .viewed ? "yes" : "no")
            }
            if let trailing { trailing }
        }
        .controlSize(.small)
        .padding(.horizontal, 12)
        .frame(height: Self.height)
        .background(Theme.Surfaces.pane)
    }
}

/// A centred card for a file with no rows to show, or an empty change.
struct ChangesCard: View {
    let symbol: String
    let title: String
    let message: String
    var action: (String, () -> Void)?

    var body: some View {
        VStack(spacing: 10) {
            Image(systemName: symbol)
                .font(.system(size: 34))
                .foregroundStyle(Theme.Surfaces.textTertiary)
            Text(title)
                .font(.title3.weight(.semibold))
            Text(message)
                .font(Metrics.callout)
                .foregroundStyle(Theme.Surfaces.textSecondary)
                .multilineTextAlignment(.center)
                .frame(maxWidth: 420)
            if let action {
                Button(action.0, action: action.1)
                    .keyboardShortcut(.defaultAction)
                    .padding(.top, 4)
            }
        }
        .padding(40)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(Theme.Surfaces.pane)
    }
}

struct ChangesStatusBar: View {
    let status: String
    /// The working copy or the index is read: the files can still change.
    let live: Bool
    /// When the list last changed by itself.
    var updatedAt: Date?
    let language: String?
    /// Whether a Claude Code session watches the repository; `nil` where
    /// it does not matter.
    var watching: Bool?

    /// How long "updated just now" stays after a refresh.
    static let fresh: TimeInterval = 5

    var body: some View {
        HStack(spacing: 16) {
            Text(status).monospacedDigit()
            Spacer()
            if live {
                TimelineView(.periodic(from: .now, by: 1)) { timeline in
                    let fresh = updatedAt.map { timeline.date.timeIntervalSince($0) < Self.fresh } ?? false
                    HStack(spacing: 5) {
                        if fresh {
                            Circle().fill(Theme.States.needsReview).frame(width: 6, height: 6)
                        }
                        Text(fresh ? "Live · updated just now" : "Live")
                    }
                    .help("Scry reads the files again when they change on disk")
                }
            }
            if let watching {
                Presence(watching: watching)
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
}
