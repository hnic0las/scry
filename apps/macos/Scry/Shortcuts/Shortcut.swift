import Foundation

/// A keyboard shortcut of the app. Every shortcut is listed once, in
/// `Shortcut.all`: menus, buttons, tooltips and the Keyboard Shortcuts
/// sheet read that list, so they always agree (SPEC: keyboard map,
/// `tokens/shortcuts.json`). SwiftUI-free, so view models can read it.
struct Shortcut: Identifiable, Equatable, Sendable {
    /// Where a shortcut works, as the Keyboard Shortcuts sheet groups them.
    enum Section: String, CaseIterable, Sendable {
        case editor = "Editor"
        case editing = "Editing by hand"
        case overview = "Overview"
        case changes = "Changes window"
        case app = "App"
        case sheets = "Special file sheets"
    }

    enum Key: Equatable, Sendable {
        case character(Character)
        case upArrow
        case downArrow
        case `return`
        case escape
    }

    struct Modifiers: OptionSet, Sendable {
        let rawValue: Int
        static let control = Modifiers(rawValue: 1 << 0)
        static let option = Modifiers(rawValue: 1 << 1)
        static let shift = Modifiers(rawValue: 1 << 2)
        static let command = Modifiers(rawValue: 1 << 3)
    }

    let id: String
    /// The action, as menus and the sheet name it.
    let title: String
    let section: Section
    let key: Key
    let modifiers: Modifiers
    /// A detail the sheet shows under the title.
    var note: String?
    /// A shorter name for the hints, beside the keys: "Inspector".
    var hint: String?
    /// The sheet's confirming button (↩), drawn as the default button.
    var isDefaultAction = false

    /// The keys as macOS prints them, modifiers in the system order:
    /// "⌥⌘1", "⇧⌘↩", "esc".
    var keys: String {
        var text = ""
        if modifiers.contains(.control) { text += "⌃" }
        if modifiers.contains(.option) { text += "⌥" }
        if modifiers.contains(.shift) { text += "⇧" }
        if modifiers.contains(.command) { text += "⌘" }
        switch key {
        case .character(let character): text += character.uppercased()
        case .upArrow: text += "↑"
        case .downArrow: text += "↓"
        case .return: text += "↩"
        case .escape: text += "esc"
        }
        return text
    }

    /// The name the hints show beside the keys.
    var hintTitle: String { hint ?? title }

    /// A tooltip naming the shortcut: "Take ours (⌥⌘1)".
    func help(_ text: String? = nil) -> String {
        "\(text ?? title) (\(keys))"
    }
}

extension Shortcut {
    // MARK: Editor

    static let previousConflict = Shortcut(
        id: "previousConflict", title: "Previous conflict", section: .editor, key: .upArrow, modifiers: [.option, .command], hint: "Previous"
    )
    static let nextConflict = Shortcut(
        id: "nextConflict", title: "Next conflict", section: .editor, key: .downArrow, modifiers: [.option, .command], hint: "Next"
    )
    static let takeOurs = Shortcut(
        id: "takeOurs", title: "Take ours", section: .editor, key: .character("1"), modifiers: [.option, .command], hint: "Ours"
    )
    static let takeTheirs = Shortcut(
        id: "takeTheirs", title: "Take theirs", section: .editor, key: .character("2"), modifiers: [.option, .command], hint: "Theirs"
    )
    static let takeBoth = Shortcut(
        id: "takeBoth", title: "Take both", section: .editor, key: .character("3"), modifiers: [.option, .command],
        note: "Press again to put theirs first", hint: "Both"
    )
    static let takeBase = Shortcut(
        id: "takeBase", title: "Take base", section: .editor, key: .character("0"), modifiers: [.option, .command]
    )
    static let takeNone = Shortcut(
        id: "takeNone", title: "Take none", section: .editor, key: .character("9"), modifiers: [.option, .command]
    )
    static let editByHand = Shortcut(
        id: "editByHand", title: "Edit by hand", section: .editor, key: .return, modifiers: [],
        note: "Edits the focused conflict's result"
    )
    static let undo = Shortcut(
        id: "undo", title: "Undo", section: .editor, key: .character("z"), modifiers: [.command]
    )
    static let redo = Shortcut(
        id: "redo", title: "Redo", section: .editor, key: .character("z"), modifiers: [.shift, .command]
    )
    static let toggleInspector = Shortcut(
        id: "toggleInspector", title: "Show or hide the inspector", section: .editor, key: .character("i"),
        modifiers: [.control, .command], hint: "Inspector"
    )
    static let toggleBase = Shortcut(
        id: "toggleBase", title: "Show or hide base", section: .editor, key: .character("b"), modifiers: [.option, .command],
        note: "Base lines above each conflict, in the result", hint: "Base"
    )
    static let switchLayout = Shortcut(
        id: "switchLayout", title: "Switch layout", section: .editor, key: .character("l"), modifiers: [.option, .command],
        note: "Three columns, or the sides above the result", hint: "Layout"
    )
    static let explain = Shortcut(
        id: "explain", title: "Explain the conflict", section: .editor, key: .character("e"),
        modifiers: [.control, .command], note: "Asks Claude Code, or asks again", hint: "Explain"
    )
    static let suggest = Shortcut(
        id: "suggest", title: "Suggest a resolution", section: .editor, key: .character("g"),
        modifiers: [.control, .command], note: "Claude Code proposes one; you accept it or not", hint: "Suggest"
    )
    static let markAsResolved = Shortcut(
        id: "markAsResolved", title: "Mark as Resolved", section: .editor, key: .return, modifiers: [.command],
        note: "Writes the file; under git mergetool, git moves on"
    )

    // MARK: Editing by hand

    static let applyEdit = Shortcut(
        id: "applyEdit", title: "Resolve with this text", section: .editing, key: .return, modifiers: [.command]
    )
    static let cancelEdit = Shortcut(
        id: "cancelEdit", title: "Cancel", section: .editing, key: .escape, modifiers: [],
        note: "Discards this edit"
    )

    // MARK: Overview

    static let openFile = Shortcut(
        id: "openFile", title: "Open the selected file", section: .overview, key: .return, modifiers: []
    )
    static let refresh = Shortcut(
        id: "refresh", title: "Read the files again", section: .overview, key: .character("r"), modifiers: [.command], hint: "Read again"
    )
    static let continueOperation = Shortcut(
        id: "continueOperation", title: "Continue the operation", section: .overview, key: .return,
        modifiers: [.shift, .command],
        note: "Once every file is staged and, on a reword, the message is not empty", hint: "Continue"
    )

    // MARK: Changes window

    static let previousChange = Shortcut(
        id: "previousChange", title: "Previous change", section: .changes, key: .upArrow, modifiers: [.option, .command],
        note: "Before the first one, press again for the previous file", hint: "Previous"
    )
    static let nextChange = Shortcut(
        id: "nextChange", title: "Next change", section: .changes, key: .downArrow, modifiers: [.option, .command],
        note: "After the last one, press again for the next file", hint: "Next"
    )
    static let previousFile = Shortcut(
        id: "previousFile", title: "Previous file", section: .changes, key: .upArrow, modifiers: [.control, .command]
    )
    static let nextFile = Shortcut(
        id: "nextFile", title: "Next file", section: .changes, key: .downArrow, modifiers: [.control, .command]
    )
    static let changesLayout = Shortcut(
        id: "changesLayout", title: "Unified or split", section: .changes, key: .character("l"),
        modifiers: [.option, .command], note: "One column, or the old side beside the new", hint: "Layout"
    )
    static let refreshChanges = Shortcut(
        id: "refreshChanges", title: "Read the files again", section: .changes, key: .character("r"),
        modifiers: [.command], hint: "Read again"
    )

    // MARK: App

    static let openRepository = Shortcut(
        id: "openRepository", title: "Open Repository…", section: .app, key: .character("o"), modifiers: [.command]
    )
    static let openConflictedFile = Shortcut(
        id: "openConflictedFile", title: "Open Conflicted File…", section: .app, key: .character("o"),
        modifiers: [.shift, .command]
    )
    static let keyboardShortcuts = Shortcut(
        id: "keyboardShortcuts", title: "Keyboard Shortcuts", section: .app, key: .character("/"), modifiers: [.command]
    )

    // MARK: Special file sheets

    static let confirmSheet = Shortcut(
        id: "confirmSheet", title: "Confirm", section: .sheets, key: .return, modifiers: [], isDefaultAction: true
    )
    static let cancelSheet = Shortcut(
        id: "cancelSheet", title: "Cancel", section: .sheets, key: .escape, modifiers: []
    )

    /// Every shortcut, in the order the sheet lists them.
    static let all: [Shortcut] = [
        previousConflict, nextConflict, takeOurs, takeTheirs, takeBoth, takeBase, takeNone, editByHand,
        undo, redo, toggleBase, switchLayout, toggleInspector, explain, suggest, markAsResolved,
        applyEdit, cancelEdit,
        openFile, refresh, continueOperation,
        previousChange, nextChange, previousFile, nextFile, changesLayout, refreshChanges,
        openRepository, openConflictedFile, keyboardShortcuts,
        confirmSheet, cancelSheet,
    ]
}
