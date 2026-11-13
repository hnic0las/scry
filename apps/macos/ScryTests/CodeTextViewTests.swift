import AppKit
import SwiftUI
import Testing
@testable import Scry

@MainActor
struct CodeTextViewTests {
    /// A text view driven by the coordinator, the core's answer for a
    /// closing character given as `indentation`.
    private func editor(_ text: String, indentation: String?) -> (NSTextView, CodeTextView.Coordinator) {
        let view = CodeTextView(text: .constant(text), typedIndent: { _, _ in indentation })
        let coordinator = CodeTextView.Coordinator(parent: view)
        let textView = NSTextView()
        textView.string = text
        textView.delegate = coordinator
        return (textView, coordinator)
    }

    /// Typing `}` at column 0 where the core wants no indentation used to
    /// recurse until the stack overflowed and the app crashed, losing every
    /// unwritten choice: the brace is now typed as it is.
    @Test func closingBraceAtColumnZeroIsTypedOnce() {
        let (textView, coordinator) = editor("func a() {\n", indentation: "")
        withExtendedLifetime(coordinator) {
            textView.setSelectedRange(NSRange(location: 11, length: 0))
            textView.insertText("}", replacementRange: textView.selectedRange())
        }
        #expect(textView.string == "func a() {\n}")
    }

    /// A brace typed on an indented blank line still moves back to the
    /// opening line's indentation, in one edit.
    @Test func closingBraceMovesBackToTheOpeningIndentation() {
        let (textView, coordinator) = editor("func a() {\n    ", indentation: "")
        withExtendedLifetime(coordinator) {
            textView.setSelectedRange(NSRange(location: 15, length: 0))
            textView.insertText("}", replacementRange: textView.selectedRange())
        }
        #expect(textView.string == "func a() {\n}")
    }

    /// The re-indented line is `nil` when nothing would change.
    @Test func reindentedIsNilWhenTheLineAlreadyFits() {
        #expect(CodeTextView.Coordinator.reindented("", to: "") == nil)
        #expect(CodeTextView.Coordinator.reindented("    ", to: "    ") == nil)
        #expect(CodeTextView.Coordinator.reindented("\t", to: "    ") == "    ")
        #expect(CodeTextView.Coordinator.reindented("        x", to: "    ") == "    x")
    }
}
