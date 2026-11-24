import Foundation
import Observation

/// The Keyboard Shortcuts sheet (⌘/): the shortcuts of `Shortcut.all`,
/// the current screen's section first, filtered by a search.
@MainActor
@Observable
final class KeyboardShortcutsViewModel: Identifiable {
    struct Row: Identifiable, Equatable {
        let id: String
        let title: String
        let keys: String
        let note: String?
    }

    struct Group: Identifiable, Equatable {
        let title: String
        let rows: [Row]
        var id: String { title }
    }

    /// Matched against the action, its note and its keys.
    var query = ""

    @ObservationIgnored private let current: Shortcut.Section?
    /// Without Explain (Settings › Agent), ⌃⌘E does nothing and is not
    /// listed.
    @ObservationIgnored private let shortcuts: [Shortcut]

    /// `current` is the screen the sheet opens on: its section comes first.
    /// `defaults` holds whether Explain is on.
    init(current: Shortcut.Section?, defaults: UserDefaults = .standard) {
        self.current = current
        shortcuts = InspectorViewModel.explainEnabled(in: defaults)
            ? Shortcut.all
            : Shortcut.all.filter { $0 != .explain && $0 != .suggest }
    }

    var groups: [Group] {
        let needle = query.trimmingCharacters(in: .whitespaces).lowercased()
        var sections = Shortcut.Section.allCases
        if let current, let index = sections.firstIndex(of: current) {
            sections.remove(at: index)
            sections.insert(current, at: 0)
        }
        return sections.compactMap { section in
            let rows = shortcuts
                .filter { $0.section == section }
                .filter { needle.isEmpty || Self.matches($0, needle) }
                .map { Row(id: $0.id, title: $0.title, keys: $0.keys, note: $0.note) }
            return rows.isEmpty ? nil : Group(title: section.rawValue, rows: rows)
        }
    }

    var isEmpty: Bool { groups.isEmpty }

    /// Only some shortcuts have a menu item (Open, Undo, Redo, Show Base,
    /// Switch Layout, Keyboard Shortcuts); the others work in the window.
    var footer: String { "Shortcuts with a menu item are also in the menu bar; the others work in the window." }

    private static func matches(_ shortcut: Shortcut, _ needle: String) -> Bool {
        shortcut.title.lowercased().contains(needle)
            || (shortcut.note?.lowercased().contains(needle) ?? false)
            || shortcut.keys.lowercased().contains(needle)
    }
}
