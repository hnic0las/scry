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
    /// The changed words of an edited line; none otherwise.
    var changedWords: [ByteRange] {
        if case .edited(let words) = change { return words }
        return []
    }

    /// The cell's text coloured by the core's spans, with changed words on
    /// `wordTint`. Offsets are UTF-8 bytes, so the text is cut through its
    /// UTF-8 view, at every span and word boundary.
    func highlighted(wordTint: Color?) -> AttributedString {
        let utf8 = text.utf8
        let words = wordTint == nil ? [] : changedWords
        var cuts: Set<Int> = [0, utf8.count]
        for span in spans {
            cuts.insert(Int(span.start))
            cuts.insert(Int(span.end))
        }
        for word in words {
            cuts.insert(Int(word.start))
            cuts.insert(Int(word.end))
        }
        let bounds = cuts.sorted()

        var result = AttributedString()
        for (start, end) in zip(bounds, bounds.dropFirst()) where start < end {
            let from = utf8.index(utf8.startIndex, offsetBy: start)
            let to = utf8.index(utf8.startIndex, offsetBy: end)
            var run = AttributedString(String(Substring(utf8[from..<to])))
            let role = spans.first { Int($0.start) <= start && end <= Int($0.end) }?.role
            run.foregroundColor = role?.color ?? Theme.Syntax.plain
            if role == .comment {
                // The design shows comments in italics.
                run.inlinePresentationIntent = .emphasized
            }
            if let wordTint, words.contains(where: { Int($0.start) <= start && end <= Int($0.end) }) {
                run.backgroundColor = wordTint
            }
            result.append(run)
        }
        return result
    }
}
