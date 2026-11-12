import Foundation
import Testing
@testable import Scry

@MainActor
struct ShortcutTests {
    /// Keys print as macOS prints them, modifiers in the system order, so
    /// tooltips, menus and the sheet read like the rest of the Mac.
    @Test func printsKeysInTheSystemOrder() {
        #expect(Shortcut.takeOurs.keys == "⌥⌘1")
        #expect(Shortcut.continueOperation.keys == "⇧⌘↩")
        #expect(Shortcut.toggleInspector.keys == "⌃⌘I")
        #expect(Shortcut.redo.keys == "⇧⌘Z")
        #expect(Shortcut.cancelEdit.keys == "esc")
        #expect(Shortcut.takeOurs.help() == "Take ours (⌥⌘1)")
        #expect(Shortcut.markAsResolved.help("Write the result") == "Write the result (⌘↩)")
    }

    /// Two actions on the same keys where both can fire would make one of
    /// them unreachable: each section, with the app's shortcuts that work
    /// everywhere, has every key combination once. Editing by hand reuses
    /// ⌘↩ on purpose, since Mark as Resolved is disabled meanwhile.
    @Test func noKeysAreSharedWhereBothWork() {
        let app = Shortcut.all.filter { $0.section == .app }
        for section in Shortcut.Section.allCases where section != .app {
            let shortcuts = Shortcut.all.filter { $0.section == section } + app
            let keys = shortcuts.map(\.keys)
            #expect(Set(keys).count == keys.count, "\(section): \(keys)")
        }
        #expect(Set(Shortcut.all.map(\.id)).count == Shortcut.all.count)
    }

    /// The sheet opens on the screen's own section, so the shortcuts the
    /// user needs now come first.
    @Test func listsTheCurrentSectionFirst() {
        #expect(KeyboardShortcutsViewModel(current: .overview).groups.first?.title == "Overview")
        #expect(KeyboardShortcutsViewModel(current: .editor).groups.first?.title == "Editor")
        let all = KeyboardShortcutsViewModel(current: nil).groups
        #expect(all.map(\.title) == Shortcut.Section.allCases.map(\.rawValue))
        #expect(all.flatMap(\.rows).count == Shortcut.all.count)
    }

    /// The search finds an action by its name or by its keys, and says
    /// when nothing matches.
    @Test func searchesByActionOrKeys() {
        let sheet = KeyboardShortcutsViewModel(current: .editor)
        sheet.query = "take"
        #expect(sheet.groups.flatMap(\.rows).map(\.title) == ["Take ours", "Take theirs", "Take both", "Take base", "Take none"])
        sheet.query = "⇧⌘"
        #expect(sheet.groups.flatMap(\.rows).map(\.keys).allSatisfy { $0.contains("⇧⌘") })
        sheet.query = "nothing like this"
        #expect(sheet.isEmpty)
    }
}
