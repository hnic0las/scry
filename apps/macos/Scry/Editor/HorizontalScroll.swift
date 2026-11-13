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
                  bounds.contains(convert(event.locationInWindow, from: nil)),
                  !Self.scrollsItself(window?.contentView?.hitTest(event.locationInWindow))
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

        /// Whether `view` is inside a view that scrolls sideways on its own,
        /// such as the hand editor's text view: the scroll is its own, not
        /// the panes'.
        static func scrollsItself(_ view: NSView?) -> Bool {
            var scrollView = view?.enclosingScrollView
            while let current = scrollView {
                if current.hasHorizontalScroller { return true }
                scrollView = current.enclosingScrollView
            }
            return false
        }
    }
}

/// How far the three panes are scrolled sideways, shared by every cell. An
/// object of its own that only the cells read: a scroll redraws the visible
/// cells without evaluating the editor and its rows again.
@MainActor
@Observable
final class SideScroll {
    /// The offset as scrolled; it can exceed `maximum` until the window
    /// widens again.
    private(set) var offset: CGFloat
    /// How far the longest line can scroll in the narrowest pane; set by
    /// the editor as the window or the rows change.
    var maximum: CGFloat = .greatestFiniteMagnitude

    init(offset: CGFloat = 0) {
        self.offset = offset
    }

    /// The offset the cells show, kept in range.
    var clamped: CGFloat {
        min(max(offset, 0), maximum)
    }

    func scroll(by delta: CGFloat) {
        offset = min(max(clamped + delta, 0), maximum)
    }

    func scroll(to offset: CGFloat) {
        self.offset = min(max(offset, 0), maximum)
    }

    /// Lines go past the narrowest pane: the scroll bars show.
    var canScroll: Bool {
        maximum > 0 && maximum < .greatestFiniteMagnitude
    }
}

/// The scroll bars under the panes, one per pane's code (the gutters stay
/// fixed), all moving the shared offset (SPEC: horizontal scrolling is
/// synchronised, by the scroll bar at the bottom of any pane too). Hidden
/// when every line fits.
struct SideScrollBars: View {
    let scroll: SideScroll
    let widths: PaneWidths
    var part: RowPart = .all

    var body: some View {
        if scroll.canScroll {
            HStack(spacing: 0) {
                switch part {
                case .all:
                    bar(widths.side)
                    Color.clear.frame(width: widths.connector)
                    bar(widths.result)
                    Color.clear.frame(width: widths.connector)
                    bar(widths.side)
                case .sides:
                    bar(widths.side)
                    Rectangle().fill(Theme.Surfaces.hairline).frame(width: widths.connector)
                    bar(widths.side)
                case .result:
                    bar(widths.result)
                }
            }
            .frame(height: SideScrollBar.height)
            .background(Theme.Surfaces.pane)
            .overlay(alignment: .top) { Rectangle().fill(Theme.Surfaces.hairline).frame(height: 1) }
        }
    }

    private func bar(_ pane: CGFloat) -> some View {
        HStack(spacing: 0) {
            Color.clear.frame(width: Metrics.lineNumberWidth + Metrics.markWidth)
            SideScrollBar(scroll: scroll, visible: widths.code(pane))
        }
        .frame(width: pane)
    }
}

/// One pane's scroll bar: a knob as wide as the share of the longest line
/// the pane shows. Drag the knob, or click the track to move a page.
struct SideScrollBar: View {
    let scroll: SideScroll
    /// The code width the pane shows.
    let visible: CGFloat

    static let height: CGFloat = 11

    @State private var dragStart: CGFloat?
    @State private var hovering = false

    var body: some View {
        GeometryReader { geometry in
            let track = geometry.size.width
            let knob = knobWidth(track: track)
            let travel = max(track - knob, 1)
            let x = scroll.maximum > 0 ? scroll.clamped / scroll.maximum * travel : 0
            ZStack(alignment: .leading) {
                Color.clear
                    .contentShape(Rectangle())
                    .onTapGesture { location in
                        // A page towards the click, as AppKit's scroll bars do.
                        scroll.scroll(by: location.x < x ? -visible : visible)
                    }
                Capsule()
                    .fill(Theme.Surfaces.textTertiary.opacity(hovering || dragStart != nil ? 0.8 : 0.5))
                    .frame(width: knob, height: hovering || dragStart != nil ? 7 : 5)
                    .offset(x: x)
                    .gesture(
                        DragGesture(minimumDistance: 0)
                            .onChanged { drag in
                                let start = dragStart ?? scroll.clamped
                                dragStart = start
                                scroll.scroll(to: start + drag.translation.width / travel * scroll.maximum)
                            }
                            .onEnded { _ in dragStart = nil }
                    )
            }
            .frame(maxHeight: .infinity)
        }
        .padding(.horizontal, 2)
        .onHover { hovering = $0 }
        .accessibilityElement()
        .accessibilityLabel("Scroll the lines sideways")
        .accessibilityValue("\(Int(scroll.maximum > 0 ? scroll.clamped / scroll.maximum * 100 : 0)) %")
        .accessibilityAdjustableAction { direction in
            switch direction {
            case .increment: scroll.scroll(by: visible / 2)
            case .decrement: scroll.scroll(by: -visible / 2)
            @unknown default: break
            }
        }
    }

    private func knobWidth(track: CGFloat) -> CGFloat {
        let content = visible + scroll.maximum
        guard content > 0 else { return track }
        return min(max(track * visible / content, 24), track)
    }
}
