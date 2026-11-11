import SwiftUI
import Testing
@testable import Scry

@MainActor
struct SyntaxTextTests {
    /// A span past the end of its line (a stale one) is ignored instead of
    /// cutting the text out of range, which would crash the app.
    @Test func spansPastTheTextAreIgnored() {
        let cell = Cell(
            number: 1,
            text: "let a",
            columns: 5,
            kind: .common,
            spans: [
                Span(start: 0, end: 3, role: .keyword),
                Span(start: 4, end: 40, role: .variable),
            ],
            change: .unchanged
        )

        #expect(String(cell.highlighted(wordTint: nil).characters) == "let a")
    }
}
