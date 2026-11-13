import AppKit
import SwiftUI
import Testing
@testable import Scry

@MainActor
struct HorizontalScrollTests {
    /// A sideways scroll over the hand editor scrolls its text, not the
    /// panes: without this, the end of a long line typed by hand cannot
    /// be reached with a trackpad.
    @Test func leavesSidewaysScrollingToTheHandEditor() throws {
        let editor = NSTextView.scrollableTextView()
        editor.hasHorizontalScroller = true
        let text = try #require(editor.documentView)

        #expect(HorizontalScrollMonitor.MonitorView.scrollsItself(text))
    }

    /// Over the rows, which only scroll up and down, the panes scroll
    /// sideways together.
    @Test func scrollsThePanesOverTheRows() {
        let rows = NSScrollView()
        rows.hasHorizontalScroller = false
        let row = NSView()
        rows.documentView = row

        #expect(!HorizontalScrollMonitor.MonitorView.scrollsItself(row))
        #expect(!HorizontalScrollMonitor.MonitorView.scrollsItself(NSView()))
        #expect(!HorizontalScrollMonitor.MonitorView.scrollsItself(nil))
    }

    /// The rows live in SwiftUI's vertical `ScrollView`, itself an
    /// `NSScrollView`: it must not count as scrolling sideways, or the
    /// panes would never scroll.
    @Test func swiftUIRowsLetThePanesScroll() throws {
        let hosting = NSHostingView(rootView: ScrollView(.vertical) {
            Text("let a = 1").frame(width: 300, height: 400)
        })
        let window = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 300, height: 200),
            styleMask: [.titled],
            backing: .buffered,
            defer: false
        )
        window.contentView = hosting
        hosting.layoutSubtreeIfNeeded()

        let scrollView = try #require(Self.firstScrollView(in: hosting))
        let inside = try #require(scrollView.documentView)
        #expect(!HorizontalScrollMonitor.MonitorView.scrollsItself(inside))
    }

    private static func firstScrollView(in view: NSView) -> NSScrollView? {
        if let scrollView = view as? NSScrollView { return scrollView }
        return view.subviews.lazy.compactMap { firstScrollView(in: $0) }.first
    }

    /// The scroll bars show only when a line goes past the narrowest pane,
    /// and dragging one keeps the shared offset within the longest line,
    /// so no pane scrolls into empty space.
    @Test func scrollBarsMoveTheSharedOffsetWithinRange() {
        let scroll = SideScroll()
        #expect(!scroll.canScroll)
        scroll.maximum = 0
        #expect(!scroll.canScroll)
        scroll.maximum = 120
        #expect(scroll.canScroll)

        scroll.scroll(to: 500)
        #expect(scroll.clamped == 120)
        scroll.scroll(to: -10)
        #expect(scroll.clamped == 0)
        scroll.scroll(to: 40)
        #expect(scroll.clamped == 40)
    }
}
