import AppKit
import SwiftUI

/// Asks `shouldClose` before its window closes, from the close button,
/// ⌘W or quitting. SwiftUI has no hook for this, and owns the window's
/// delegate, so the guard sits in front of it and forwards everything else.
struct WindowCloseGuard: NSViewRepresentable {
    let shouldClose: () -> Bool

    func makeCoordinator() -> Coordinator {
        Coordinator()
    }

    func makeNSView(context: Context) -> NSView {
        let view = GuardView()
        view.coordinator = context.coordinator
        return view
    }

    func updateNSView(_ view: NSView, context: Context) {
        context.coordinator.shouldClose = shouldClose
    }

    final class GuardView: NSView {
        var coordinator: Coordinator?

        override func viewDidMoveToWindow() {
            super.viewDidMoveToWindow()
            guard let window, let coordinator, window.delegate !== coordinator else { return }
            coordinator.original = window.delegate
            window.delegate = coordinator
        }
    }

    /// The window's delegate while the guard is on screen; the window keeps
    /// it weakly, the view keeps it alive.
    final class Coordinator: NSObject, NSWindowDelegate {
        var shouldClose: () -> Bool = { true }
        weak var original: NSWindowDelegate?

        func windowShouldClose(_ sender: NSWindow) -> Bool {
            guard shouldClose() else { return false }
            return original?.windowShouldClose?(sender) ?? true
        }

        override func responds(to selector: Selector!) -> Bool {
            super.responds(to: selector) || (original?.responds(to: selector) ?? false)
        }

        override func forwardingTarget(for selector: Selector!) -> Any? {
            original?.responds(to: selector) == true ? original : super.forwardingTarget(for: selector)
        }
    }
}
