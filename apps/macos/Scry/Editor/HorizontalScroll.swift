import AppKit
import SwiftUI

/// Reports sideways scrolling (trackpad, or shift with a mouse wheel) over
/// the view it backs, and lets vertical scrolling through to the scroll view.
/// The three panes share one horizontal offset, which SwiftUI's scroll views
/// cannot share across thousands of rows, hence an event monitor.
struct HorizontalScrollMonitor: NSViewRepresentable {
    /// Called with the distance to move the content, in points; positive
    /// moves it left, revealing the end of long lines.
    let onScroll: (CGFloat) -> Void

    func makeNSView(context: Context) -> MonitorView {
        MonitorView(onScroll: onScroll)
    }

    func updateNSView(_ view: MonitorView, context: Context) {
        view.onScroll = onScroll
    }

    final class MonitorView: NSView {
        var onScroll: (CGFloat) -> Void
        private var monitor: Any?

        init(onScroll: @escaping (CGFloat) -> Void) {
            self.onScroll = onScroll
            super.init(frame: .zero)
        }

        @available(*, unavailable)
        required init?(coder: NSCoder) {
            fatalError("not used from a nib")
        }

        // Clicks go to the rows above.
        override func hitTest(_ point: NSPoint) -> NSView? { nil }

        override func viewDidMoveToWindow() {
            super.viewDidMoveToWindow()
            if let monitor { NSEvent.removeMonitor(monitor) }
            monitor = nil
            guard window != nil else { return }
            monitor = NSEvent.addLocalMonitorForEvents(matching: .scrollWheel) { [weak self] event in
                self?.handle(event) ?? event
            }
        }

        private func handle(_ event: NSEvent) -> NSEvent? {
            guard event.window === window,
                  bounds.contains(convert(event.locationInWindow, from: nil))
            else { return event }
            var dx = event.scrollingDeltaX
            var dy = event.scrollingDeltaY
            // A mouse wheel with shift scrolls sideways; some mice report it
            // as vertical.
            if event.modifierFlags.contains(.shift), dx == 0 {
                dx = dy
                dy = 0
            }
            guard abs(dx) > abs(dy) else { return event }
            // A wheel reports lines, a trackpad points.
            let points = event.hasPreciseScrollingDeltas ? dx : dx * Metrics.codeCharacterWidth * 3
            onScroll(-points)
            return nil
        }
    }
}
