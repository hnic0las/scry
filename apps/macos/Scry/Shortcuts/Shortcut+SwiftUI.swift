import SwiftUI

extension Shortcut {
    /// The shortcut as SwiftUI binds it. Esc is the cancel action, so ⌘.
    /// cancels too, as everywhere on macOS.
    var keyboardShortcut: KeyboardShortcut {
        if isDefaultAction { return .defaultAction }
        if key == .escape && modifiers.isEmpty { return .cancelAction }
        return KeyboardShortcut(keyEquivalent, modifiers: eventModifiers)
    }

    private var keyEquivalent: KeyEquivalent {
        switch key {
        case .character(let character): KeyEquivalent(character)
        case .upArrow: .upArrow
        case .downArrow: .downArrow
        case .return: .return
        case .escape: .escape
        }
    }

    private var eventModifiers: EventModifiers {
        var result: EventModifiers = []
        if modifiers.contains(.control) { result.insert(.control) }
        if modifiers.contains(.option) { result.insert(.option) }
        if modifiers.contains(.shift) { result.insert(.shift) }
        if modifiers.contains(.command) { result.insert(.command) }
        return result
    }
}

extension View {
    /// Binds a shortcut of the app's list (`Shortcut.all`).
    func keyboardShortcut(_ shortcut: Shortcut) -> some View {
        keyboardShortcut(shortcut.keyboardShortcut)
    }
}
