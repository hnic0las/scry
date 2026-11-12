import AppKit
import SwiftUI

/// The file row menu's items, the same in the context menu and the File
/// menu: open (↩), stage or unstage, then Reveal in Finder and Copy Path.
/// A disabled item keeps its place and says why on a second line.
struct FileRowMenuItems: View {
    let menu: OverviewViewModel.RowMenu
    /// The context menu shows ↩ beside Open; the File menu does not bind
    /// it, so Return keeps working in text fields.
    let showsReturn: Bool
    let open: () -> Void
    let stage: () -> Void

    var body: some View {
        item(menu.open, action: open)
            .keyboardShortcut(showsReturn ? Shortcut.openFile.keyboardShortcut : nil)
        item(menu.stage, action: stage)
        Divider()
        Button("Reveal in Finder") { FileRowActions.reveal(menu.absolutePaths) }
        Button(menu.paths.count == 1 ? "Copy Path" : "Copy \(menu.paths.count) Paths") {
            FileRowActions.copy(menu.absolutePaths)
        }
    }

    /// A menu item with its reason as a second line.
    private func item(_ item: OverviewViewModel.MenuItem, action: @escaping () -> Void) -> some View {
        Button(action: action) {
            Text(item.title)
            if let detail = item.detail {
                Text(detail)
            }
        }
        .disabled(!item.enabled)
    }
}

/// What the File menu's file items act on: the overview's selected rows.
struct FileRowCommandActions {
    let menu: OverviewViewModel.RowMenu
    let open: () -> Void
    let stage: () -> Void
}

extension FocusedValues {
    @Entry var fileRowActions: FileRowCommandActions?
}

/// The File menu's items for the selected rows, disabled when no row is
/// selected.
struct FileRowCommands: View {
    @FocusedValue(\.fileRowActions) private var actions

    var body: some View {
        Divider()
        if let actions {
            FileRowMenuItems(menu: actions.menu, showsReturn: false, open: actions.open, stage: actions.stage)
        } else {
            Button("Open in Merge Editor") {}.disabled(true)
            Button("Stage") {}.disabled(true)
            Divider()
            Button("Reveal in Finder") {}.disabled(true)
            Button("Copy Path") {}.disabled(true)
        }
    }
}

/// The Finder and the pasteboard, which view models do not reach.
enum FileRowActions {
    static func reveal(_ paths: [String]) {
        NSWorkspace.shared.activateFileViewerSelecting(paths.map { URL(fileURLWithPath: $0) })
    }

    /// One path per line.
    static func copy(_ paths: [String]) {
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(paths.joined(separator: "\n"), forType: .string)
    }
}
