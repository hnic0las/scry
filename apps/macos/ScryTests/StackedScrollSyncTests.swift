import Testing
@testable import Scry

struct StackedScrollSyncTests {
    /// A jump to a conflict scrolls the result area alone, and the sides
    /// follow it: when both areas scrolled and each moved the other, they
    /// pulled each other back and stopped short of the conflict (reported
    /// on 0.1.0 with ⌥⌘↓ and the toolbar arrows).
    @Test func aJumpIsLedByTheResultArea() {
        var sync = StackedScrollSync()
        sync.jump()

        #expect(sync.jumpArea == .bottom)
        #expect(sync.follows(from: .bottom))
        #expect(!sync.follows(from: .top))

        // Once the result area is at rest, either area leads again.
        sync.stopped(.bottom)
        #expect(sync.follows(from: .top))
    }

    /// The area the user scrolls moves the other, never the reverse, so
    /// the other's clamping never cuts its momentum or bounce.
    @Test func theScrolledAreaLeads() {
        var sync = StackedScrollSync()
        sync.userScrolls(.top)
        #expect(sync.follows(from: .top))
        #expect(!sync.follows(from: .bottom))

        // Another area coming to rest does not end it.
        sync.stopped(.bottom)
        #expect(!sync.follows(from: .bottom))
        sync.stopped(.top)
        #expect(sync.driver == nil)
    }
}
