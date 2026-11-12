import AppKit
import SwiftUI

/// Watches for ⌘ held alone: after a moment the window shows each
/// control's shortcut (SPEC: shortcut hints while ⌘ is held). Any other
/// key, a click, a scroll or another modifier hides them, so ⌘S, ⌘-click
/// and ⌘-drag never flash them.
@MainActor
@Observable
final class CommandHold {
    /// Long enough not to show while typing a shortcut; the SPEC says
    /// 0.5 s, Claude Design is asked about 0.7 s.
    static let delay: Duration = .milliseconds(700)

    private(set) var isHeld = false

    @ObservationIgnored private var monitor: Any?
    @ObservationIgnored private var pending: Task<Void, Never>?
    @ObservationIgnored private var resign: NSObjectProtocol?

    func start() {
        guard monitor == nil else { return }
        monitor = NSEvent.addLocalMonitorForEvents(
            matching: [.flagsChanged, .keyDown, .leftMouseDown, .rightMouseDown, .otherMouseDown, .scrollWheel]
        ) { [weak self] event in
            MainActor.assumeIsolated { self?.handle(event) }
            return event
        }
        resign = NotificationCenter.default.addObserver(
            forName: NSApplication.didResignActiveNotification, object: nil, queue: .main
        ) { [weak self] _ in
            MainActor.assumeIsolated { self?.release() }
        }
    }

    func stop() {
        if let monitor { NSEvent.removeMonitor(monitor) }
        if let resign { NotificationCenter.default.removeObserver(resign) }
        monitor = nil
        resign = nil
        release()
    }

    private func handle(_ event: NSEvent) {
        let alone = event.type == .flagsChanged
            && event.modifierFlags.intersection(.deviceIndependentFlagsMask).subtracting(.capsLock) == .command
        if alone {
            pending?.cancel()
            pending = Task { [weak self] in
                try? await Task.sleep(for: Self.delay)
                guard !Task.isCancelled else { return }
                self?.isHeld = true
            }
        } else {
            release()
        }
    }

    private func release() {
        pending?.cancel()
        pending = nil
        isHeld = false
    }
}

extension EnvironmentValues {
    /// ⌘ is held in this window: controls show their shortcut.
    @Entry var showsShortcutHints = false
}

/// The keys of a shortcut, as the hints draw them.
struct HintBadge: View {
    let text: String

    var body: some View {
        Text(text)
            .font(.system(size: 11, weight: .semibold))
            .foregroundStyle(Theme.KeyboardHintsAndCommitMessage.shortcutHintText)
            .padding(.horizontal, 6)
            .frame(height: 18)
            .background(
                RoundedRectangle(cornerRadius: 5)
                    .fill(Theme.KeyboardHintsAndCommitMessage.shortcutHint)
            )
            .overlay(
                RoundedRectangle(cornerRadius: 5)
                    .strokeBorder(.white.opacity(0.15))
            )
            .fixedSize()
            .accessibilityHidden(true)
    }
}

/// Shows a badge on the bottom edge of a control while ⌘ is held, only
/// when the control works now: apply it inside `.disabled(_:)`.
private struct ShortcutHintModifier: ViewModifier {
    let text: String
    /// `.top` for a control at the bottom of the window, where a badge
    /// below would fall outside it.
    var edge: VerticalEdge = .bottom
    @Environment(\.showsShortcutHints) private var shows
    @Environment(\.isEnabled) private var isEnabled

    func body(content: Content) -> some View {
        content.overlay(alignment: edge == .bottom ? .bottom : .top) {
            if shows && isEnabled {
                HintBadge(text: text)
                    .offset(y: edge == .bottom ? 12 : -12)
                    .allowsHitTesting(false)
            }
        }
    }
}

extension View {
    /// The shortcut's keys under the control while ⌘ is held.
    /// Not for toolbar items, which clip it: see `ToolbarHintRow`.
    func shortcutHint(_ shortcut: Shortcut, edge: VerticalEdge = .bottom) -> some View {
        modifier(ShortcutHintModifier(text: shortcut.keys, edge: edge))
    }
}

/// The shortcuts of the toolbar's buttons, in a row just under the
/// toolbar, in the toolbar's order: a toolbar item clips anything drawn
/// outside its own capsule, so badges cannot hang below its buttons.
struct ToolbarHintRow: View {
    let shortcuts: [Shortcut]
    @Environment(\.showsShortcutHints) private var shows

    var body: some View {
        if shows && !shortcuts.isEmpty {
            HStack(spacing: 12) {
                ForEach(shortcuts) { shortcut in
                    HStack(spacing: 5) {
                        HintBadge(text: shortcut.keys)
                        Text(shortcut.hintTitle)
                            .font(.system(size: 11, weight: .medium))
                            .foregroundStyle(Theme.KeyboardHintsAndCommitMessage.shortcutHintText)
                    }
                }
            }
            .padding(.leading, 4)
            .padding(.trailing, 10)
            .padding(.vertical, 4)
            .background(Capsule().fill(Theme.KeyboardHintsAndCommitMessage.shortcutHint))
            .fixedSize()
            .padding(.top, 6)
            .padding(.trailing, 10)
            .frame(maxWidth: .infinity, alignment: .trailing)
            .allowsHitTesting(false)
            .accessibilityHidden(true)
        }
    }
}

/// The shortcuts without a visible control, in a pill at the bottom of the
/// window while ⌘ is held.
struct HintStrip: View {
    let shortcuts: [Shortcut]
    @Environment(\.showsShortcutHints) private var shows

    var body: some View {
        if shows && !shortcuts.isEmpty {
            HStack(spacing: 10) {
                ForEach(Array(shortcuts.enumerated()), id: \.element.id) { index, shortcut in
                    if index > 0 {
                        Text("·").foregroundStyle(Theme.KeyboardHintsAndCommitMessage.shortcutHintText.opacity(0.5))
                    }
                    HStack(spacing: 5) {
                        Text(shortcut.keys).fontWeight(.semibold)
                        Text(shortcut.hintTitle)
                    }
                }
            }
            .font(.system(size: 12))
            .foregroundStyle(Theme.KeyboardHintsAndCommitMessage.shortcutHintText)
            .padding(.horizontal, 14)
            .padding(.vertical, 7)
            .background(Capsule().fill(Theme.KeyboardHintsAndCommitMessage.shortcutHint))
            .fixedSize()
            .padding(.bottom, 12)
            .allowsHitTesting(false)
            .accessibilityHidden(true)
        }
    }
}
