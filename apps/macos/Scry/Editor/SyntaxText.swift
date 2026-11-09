import SwiftUI

extension SyntaxRole {
    var color: Color {
        switch self {
        case .keyword: Theme.Syntax.keyword
        case .string: Theme.Syntax.string
        case .number: Theme.Syntax.number
        case .comment: Theme.Syntax.comment
        case .type: Theme.Syntax.type
        case .function: Theme.Syntax.function
        case .variable: Theme.Syntax.variable
        case .constant: Theme.Syntax.constant
        case .operator: Theme.Syntax.operator
        case .punctuation: Theme.Syntax.punctuation
        case .attribute: Theme.Syntax.attribute
        }
    }
}

extension Cell {
    /// The cell's text coloured by the core's spans. Span offsets are UTF-8
    /// bytes, so the text is cut through its UTF-8 view.
    var highlighted: AttributedString {
        let utf8 = text.utf8
        var result = AttributedString()
        var position = 0
        func append(_ start: Int, _ end: Int, role: SyntaxRole?) {
            guard start < end else { return }
            let from = utf8.index(utf8.startIndex, offsetBy: start)
            let to = utf8.index(utf8.startIndex, offsetBy: end)
            var run = AttributedString(String(Substring(utf8[from..<to])))
            run.foregroundColor = role?.color ?? Theme.Syntax.plain
            if role == .comment {
                // The design shows comments in italics.
                run.inlinePresentationIntent = .emphasized
            }
            result.append(run)
        }
        for span in spans {
            append(position, Int(span.start), role: nil)
            append(Int(span.start), Int(span.end), role: span.role)
            position = Int(span.end)
        }
        append(position, utf8.count, role: nil)
        return result
    }
}
