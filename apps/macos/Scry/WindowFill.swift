import AppKit
import SwiftUI

/// A double-click on the empty part of the title bar or toolbar makes the
/// window fill the screen, minus the menu bar and the Dock, and a second
/// one puts it back. Not native full screen (the green button): the window
/// stays a window, beside the others. macOS's own double-click action
/// (zoom) sizes a SwiftUI window to its content, which here is little.
struct TitleBarDoubleClick: NSViewRepresentable {
    func makeNSView(context: Context) -> MonitorView {
        MonitorView()
    }

    func updateNSView(_ view: MonitorView, context: Context) {}

    final class MonitorView: NSView {
        private var monitor: Any?
        /// The frame before filling the screen, to go back to.
        private var saved: NSRect?

        // Clicks go to the views above.
        override func hitTest(_ point: NSPoint) -> NSView? { nil }

        override func viewDidMoveToWindow() {
            super.viewDidMoveToWindow()
            if let monitor { NSEvent.removeMonitor(monitor) }
            monitor = nil
            guard window != nil else { return }
            monitor = NSEvent.addLocalMonitorForEvents(matching: .leftMouseDown) { [weak self] event in
                self?.handle(event) ?? event
            }
        }

        private func handle(_ event: NSEvent) -> NSEvent? {
            guard let window, event.window === window, event.clickCount == 2,
                  !window.styleMask.contains(.fullScreen),
                  let screen = window.screen,
                  Self.isEmptyTitleBar(window: window, at: event.locationInWindow)
            else { return event }
            let fill = WindowFill.next(current: window.frame, visible: screen.visibleFrame, saved: saved)
            saved = fill.saved
            window.setFrame(fill.frame, display: true, animate: true)
            // The system's own double-click action would run too.
            return nil
        }

        /// Whether the point is in the title bar or toolbar, and not on one
        /// of their controls. The window's title and subtitle count as
        /// empty, as in other Mac apps.
        static func isEmptyTitleBar(window: NSWindow, at point: NSPoint) -> Bool {
            // A click right after the window shrank can land where its bar
            // was, outside it now: it is not on the bar.
            let bounds = NSRect(origin: .zero, size: window.frame.size)
            guard bounds.contains(point), point.y > window.contentLayoutRect.maxY else { return false }
            guard let frameView = window.contentView?.superview,
                  let hit = frameView.hitTest(frameView.convert(point, from: nil))
            else { return true }
            return !isControl(hit)
        }

        /// A toolbar item (SwiftUI hosts each in an item viewer) or an
        /// AppKit control other than a plain label.
        static func isControl(_ view: NSView) -> Bool {
            var current: NSView? = view
            while let view = current {
                if String(describing: type(of: view)).contains("ToolbarItemViewer") { return true }
                if let field = view as? NSTextField {
                    if field.isEditable || field.isSelectable { return true }
                } else if view is NSControl {
                    return true
                }
                current = view.superview
            }
            return false
        }
    }
}

/// Where a double-click on the title bar takes the window.
enum WindowFill {
    /// The screen's visible frame, or back to the saved frame when the
    /// window already fills it; `saved` is what to keep for next time.
    static func next(current: NSRect, visible: NSRect, saved: NSRect?) -> (frame: NSRect, saved: NSRect?) {
        if fills(current, visible), let saved {
            return (saved, nil)
        }
        if fills(current, visible) {
            // Filled by hand: nothing to go back to, so leave it.
            return (current, nil)
        }
        return (visible, current)
    }

    /// Within a point of the visible frame, as AppKit rounds frames.
    static func fills(_ frame: NSRect, _ visible: NSRect) -> Bool {
        abs(frame.minX - visible.minX) <= 1 && abs(frame.minY - visible.minY) <= 1
            && abs(frame.width - visible.width) <= 1 && abs(frame.height - visible.height) <= 1
    }
}
