import AppKit
import Testing
@testable import Scry

struct WindowFillTests {
    private let visible = NSRect(x: 0, y: 80, width: 1512, height: 870)
    private let window = NSRect(x: 200, y: 300, width: 1100, height: 600)

    /// A first double-click fills the screen and keeps the frame it had,
    /// a second one puts it back exactly, so the toggle never loses the
    /// user's own window size.
    @Test func fillsTheScreenThenGoesBack() {
        let filled = WindowFill.next(current: window, visible: visible, saved: nil)
        #expect(filled.frame == visible)
        #expect(filled.saved == window)

        let back = WindowFill.next(current: filled.frame, visible: visible, saved: filled.saved)
        #expect(back.frame == window)
        #expect(back.saved == nil)
    }

    /// A window the user already stretched to fill the screen, with
    /// nothing saved, stays as it is rather than jumping to some size.
    @Test func leavesAWindowFilledByHand() {
        let result = WindowFill.next(current: visible.insetBy(dx: 0.5, dy: 0.5), visible: visible, saved: nil)
        #expect(result.frame == visible.insetBy(dx: 0.5, dy: 0.5))
        #expect(result.saved == nil)
    }

    /// A double-click on a button or a search field in the toolbar is the
    /// control's own, while the window's title label lets it through, so
    /// the gesture never fights a control.
    @Test @MainActor func controlsKeepTheirDoubleClick() {
        let button = NSButton(title: "Base", target: nil, action: nil)
        let inside = NSView()
        button.addSubview(inside)
        #expect(TitleBarDoubleClick.MonitorView.isControl(button))
        #expect(TitleBarDoubleClick.MonitorView.isControl(inside))
        #expect(TitleBarDoubleClick.MonitorView.isControl(NSSearchField()))
        #expect(!TitleBarDoubleClick.MonitorView.isControl(NSTextField(labelWithString: "SyncWorker.kt")))
        #expect(!TitleBarDoubleClick.MonitorView.isControl(NSView()))
    }

    /// A click that lands outside the window, where its bar was before it
    /// shrank back, is not on the bar: otherwise a quick extra click after
    /// going back would fill the screen again at once.
    @Test @MainActor func ignoresClicksOutsideTheWindow() {
        let window = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 900, height: 600),
            styleMask: [.titled, .resizable, .fullSizeContentView],
            backing: .buffered, defer: true
        )
        let bar = window.contentLayoutRect.maxY
        #expect(TitleBarDoubleClick.MonitorView.isEmptyTitleBar(window: window, at: NSPoint(x: 450, y: bar + 2)))
        #expect(!TitleBarDoubleClick.MonitorView.isEmptyTitleBar(window: window, at: NSPoint(x: 450, y: window.frame.height + 200)))
        #expect(!TitleBarDoubleClick.MonitorView.isEmptyTitleBar(window: window, at: NSPoint(x: 450, y: bar - 50)))
    }
}
