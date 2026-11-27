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
                fileArea
            }
        }
        .background(Theme.Surfaces.pane)
        .overlay(alignment: .bottom) { HintStrip(shortcuts: viewModel.hintStrip) }
        .overlay(alignment: .top) { ToolbarHintRow(shortcuts: viewModel.toolbarHints) }
        .navigationTitle(viewModel.header.repository)
        .navigationSubtitle(viewModel.header.sentence)
        .toolbar { toolbar }
        .focusedSceneValue(\.changesActions, ChangesActions(
            layout: viewModel.layout,
            toggleLayout: { viewModel.layout = viewModel.layout == .unified ? .split : .unified }
        ))
        .task {
            if loads { await viewModel.load() }
        }
    }

    @ViewBuilder
    private var fileArea: some View {
        VStack(spacing: 0) {
            if let file = viewModel.selected {
                ChangesFileHeader(viewModel: viewModel, file: file)
                Rectangle().fill(Theme.Surfaces.hairline).frame(height: 1)
            }
            content
            ChangesStatusBar(status: viewModel.status, live: viewModel.header.live, language: viewModel.language)
        }
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

    @ToolbarContentBuilder
    private var toolbar: some ToolbarContent {
        ToolbarItemGroup {
            Button { Task { await viewModel.previousChange() } } label: {
                Image(systemName: "chevron.up")
            }
            .keyboardShortcut(Shortcut.previousChange)
            .help(Shortcut.previousChange.help())
            .disabled(!viewModel.canGoToPreviousChange)

            Button { Task { await viewModel.nextChange() } } label: {
                Image(systemName: "chevron.down")
            }
            .keyboardShortcut(Shortcut.nextChange)
            .help(Shortcut.nextChange.help())
            .disabled(!viewModel.canGoToNextChange)
        }
        ToolbarItem {
            Text(viewModel.changeCounter)
                .font(Metrics.callout)
                .foregroundStyle(Theme.Surfaces.textSecondary)
                .monospacedDigit()
                .padding(.horizontal, 4)
        }
        ToolbarItem {
            Picker("Layout", selection: $viewModel.layout) {
                Label("Unified", systemImage: "rectangle").tag(ChangesViewModel.Layout.unified)
                Label("Split", systemImage: "rectangle.split.2x1").tag(ChangesViewModel.Layout.split)
            }
            .pickerStyle(.segmented)
            .labelStyle(.titleAndIcon)
            .help(Shortcut.changesLayout.help())
        }
        // git difftool's files are copies: there is nothing to read again.
        if !viewModel.header.reduced {
            ToolbarItem {
                Button { Task { await viewModel.load() } } label: {
                    Image(systemName: "arrow.clockwise")
                }
                .keyboardShortcut(Shortcut.refreshChanges)
                .help(Shortcut.refreshChanges.help())
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
    let viewModel: ChangesViewModel

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
                            }
                            ForEach(section.collapsed ? [] : section.files, id: \.index) { file in
                                Button { Task { await viewModel.select(file) } } label: {
                                    ChangesFileRow(file: file, selected: file == viewModel.selected)
                                }
                                .buttonStyle(.plain)
                                .accessibilityAddTraits(file == viewModel.selected ? .isSelected : [])
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

struct ChangesFileRow: View {
    let file: ChangesFile
    let selected: Bool

    static let height: CGFloat = 38

    var body: some View {
        HStack(spacing: 8) {
            ChangesBadge(file: file)
            VStack(alignment: .leading, spacing: 1) {
                Text(file.name)
                    .font(.system(size: 12, weight: .semibold, design: .monospaced))
                    .foregroundStyle(Theme.Surfaces.text)
                    .lineLimit(1)
                    .truncationMode(.middle)
                Text(ChangesViewModel.detail(file))
                    .font(Metrics.caption)
                    .foregroundStyle(Theme.Surfaces.textSecondary)
                    .lineLimit(1)
                    .truncationMode(.head)
            }
            Spacer(minLength: 4)
            if file.partlyStaged { PartlyStagedTag() }
            ChangesCounts(added: file.added, removed: file.removed)
        }
        .padding(.horizontal, 8)
        .frame(height: Self.height)
        .background(RoundedRectangle(cornerRadius: 6).fill(selected ? Theme.Interaction.selection : Color.clear))
        .padding(.horizontal, 4)
        .contentShape(Rectangle())
        .help(file.path)
    }
}

/// The selected file's name and counts, with previous and next file.
struct ChangesFileHeader: View {
    let viewModel: ChangesViewModel
    let file: ChangesFile

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
    let language: String?

    var body: some View {
        HStack(spacing: 16) {
            Text(status).monospacedDigit()
            Spacer()
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
