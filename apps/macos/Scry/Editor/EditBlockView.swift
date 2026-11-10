import AppKit
import SwiftUI

/// Under the lens row of the conflict being edited: its text in an editor,
/// in the result column, with Cancel and Apply. The core checks the text
/// once applied, like a candidate.
struct EditBlockView: View {
    @Bindable var viewModel: EditBlockViewModel
    let widths: PaneWidths

    var body: some View {
        HStack(spacing: 0) {
            Color.clear.frame(width: widths.side + widths.connector)
            VStack(alignment: .leading, spacing: 6) {
                editor
                actions
            }
            .padding(8)
            .frame(width: widths.result)
            .background(Theme.Provenance.human.opacity(Metrics.blockTintOpacity))
            Color.clear.frame(width: widths.connector + widths.side)
        }
    }

    private var editor: some View {
        CodeTextView(
            text: $viewModel.draft,
            highlight: { viewModel.highlight($0) },
            newline: { viewModel.newline(before: $0, after: $1) },
            typedIndent: { viewModel.typedIndent(before: $0, typed: $1) }
        )
        .frame(height: editorHeight)
        .overlay(
            RoundedRectangle(cornerRadius: 4)
                .stroke(Theme.Provenance.human, lineWidth: 1.5)
        )
    }

    private var actions: some View {
        HStack(spacing: 8) {
            Label("Editing by hand", systemImage: "person.fill")
                .font(Metrics.captionBold)
                .foregroundStyle(Theme.Provenance.human)
            Spacer()
            Button("Cancel") { viewModel.cancel() }
                .keyboardShortcut(.cancelAction)
                .help("Discard this edit (Esc)")
            Button("Apply") { viewModel.apply() }
                .keyboardShortcut(.return, modifiers: .command)
                .buttonStyle(.borderedProminent)
                .help("Resolve the conflict with this text (⌘↩)")
        }
    }

    private var editorHeight: CGFloat {
        CGFloat(viewModel.visibleLines) * Metrics.rowHeight + 8
    }
}

/// A code editor: monospaced, no wrapping, and none of the substitutions
/// (smart quotes, dashes, spelling) that would change code, which SwiftUI's
/// `TextEditor` cannot turn off. The core colours the text as it is typed
/// and decides what Return inserts, so indentation follows the file.
struct CodeTextView: NSViewRepresentable {
    @Binding var text: String
    /// Spans per line of the text, from the core.
    var highlight: (String) -> [[Span]] = { _ in [] }
    /// What Return does, given all the text before the cursor and the rest
    /// of its line: the text replacing the line up to the cursor. By
    /// default, a plain line break.
    var newline: (String, String) -> Newline = { before, _ in
        let line = before.split(separator: "\n", omittingEmptySubsequences: false).last.map(String.init) ?? ""
        return Newline(text: line + "\n", cursor: UInt32(line.utf8.count + 1))
    }
    /// The indentation for the current line once a character is typed,
    /// given the text before the cursor and the character; `nil` to type it
    /// as it is.
    var typedIndent: (String, String) -> String? = { _, _ in nil }

    func makeCoordinator() -> Coordinator {
        Coordinator(parent: self)
    }

    func makeNSView(context: Context) -> NSScrollView {
        let scrollView = NSTextView.scrollableTextView()
        scrollView.hasHorizontalScroller = true
        scrollView.drawsBackground = false
        guard let textView = scrollView.documentView as? NSTextView else { return scrollView }
        textView.font = Coordinator.font
        textView.isRichText = false
        textView.allowsUndo = true
        textView.isAutomaticQuoteSubstitutionEnabled = false
        textView.isAutomaticDashSubstitutionEnabled = false
        textView.isAutomaticTextReplacementEnabled = false
        textView.isAutomaticSpellingCorrectionEnabled = false
        textView.isContinuousSpellCheckingEnabled = false
        textView.isAutomaticLinkDetectionEnabled = false
        textView.drawsBackground = false
        textView.textContainerInset = NSSize(width: 4, height: 4)
        // No wrapping: lines scroll sideways, as in the panes.
        textView.isHorizontallyResizable = true
        textView.textContainer?.widthTracksTextView = false
        textView.textContainer?.containerSize = NSSize(
            width: CGFloat.greatestFiniteMagnitude,
            height: CGFloat.greatestFiniteMagnitude
        )
        textView.string = text
        textView.delegate = context.coordinator
        context.coordinator.colour(textView)
        // Ready to type as soon as it opens.
        DispatchQueue.main.async { textView.window?.makeFirstResponder(textView) }
        return scrollView
    }

    func updateNSView(_ scrollView: NSScrollView, context: Context) {
        context.coordinator.parent = self
        guard let textView = scrollView.documentView as? NSTextView,
              textView.string != text
        else { return }
        textView.string = text
        context.coordinator.colour(textView)
    }

    @MainActor
    final class Coordinator: NSObject, NSTextViewDelegate {
        static let font = NSFont.monospacedSystemFont(ofSize: 12, weight: .regular)
        static let italic = NSFontManager.shared.convert(font, toHaveTrait: .italicFontMask)

        var parent: CodeTextView

        init(parent: CodeTextView) {
            self.parent = parent
        }

        func textDidChange(_ notification: Notification) {
            guard let textView = notification.object as? NSTextView else { return }
            parent.text = textView.string
            colour(textView)
        }

        /// Return does what the core says: the current line, moved back
        /// when it closes a block (`end`, `fi`), then the line break and the
        /// indentation, and for a block opened between a pair the closing
        /// part on its own line. One edit, as typing, so ⌘Z undoes it.
        func textView(_ textView: NSTextView, doCommandBy selector: Selector) -> Bool {
            guard selector == #selector(NSResponder.insertNewline(_:)) else { return false }
            let string = textView.string as NSString
            let selection = textView.selectedRange()
            let line = string.lineRange(for: NSRange(location: selection.location, length: 0))
            var lineEnd = line.location + line.length
            while lineEnd > line.location,
                  let scalar = UnicodeScalar(string.character(at: lineEnd - 1)),
                  CharacterSet.newlines.contains(scalar) {
                lineEnd -= 1
            }
            let before = string.substring(to: selection.location)
            let selectionEnd = selection.location + selection.length
            let after = string.substring(with: NSRange(
                location: selectionEnd,
                length: max(lineEnd - selectionEnd, 0)
            ))
            let newline = parent.newline(before, after)
            // Replaces the line from its start, so it can move back.
            let replaced = NSRange(location: line.location, length: selectionEnd - line.location)
            textView.insertText(newline.text, replacementRange: replaced)
            // The core counts UTF-8 bytes; AppKit counts UTF-16 units.
            let cursor = String(decoding: newline.text.utf8.prefix(Int(newline.cursor)), as: UTF8.self)
            textView.setSelectedRange(NSRange(location: line.location + cursor.utf16.count, length: 0))
            return true
        }

        /// Characters that can close a block as they are typed: brackets on
        /// a blank line, the colon of `case …:` or Python's `else:`, and the
        /// `>` of an HTML closing tag.
        private static let closing: Set<String> = ["}", ")", "]", ":", ">"]

        /// When the core says the line closes a block, it moves back to the
        /// indentation of the block's opening line as the character is
        /// typed.
        func textView(
            _ textView: NSTextView,
            shouldChangeTextIn range: NSRange,
            replacementString text: String?
        ) -> Bool {
            guard let text, range.length == 0, Self.closing.contains(text) else { return true }
            let string = textView.string as NSString
            let before = string.substring(to: range.location)
            guard let indentation = parent.typedIndent(before, text) else { return true }
            let lineStart = string.lineRange(for: NSRange(location: range.location, length: 0)).location
            let typed = string.substring(with: NSRange(location: lineStart, length: range.location - lineStart))
            let content = typed.drop { $0 == " " || $0 == "\t" }
            // The line's new indentation, its text and the character in one
            // edit, so ⌘Z undoes them together.
            textView.insertText(
                indentation + content + text,
                replacementRange: NSRange(location: lineStart, length: range.location - lineStart)
            )
            return false
        }

        /// Applies the core's spans to the text, without changing it.
        func colour(_ textView: NSTextView) {
            guard let storage = textView.textStorage else { return }
            let string = textView.string
            let spans = parent.highlight(string)
            storage.beginEditing()
            let whole = NSRange(location: 0, length: (string as NSString).length)
            storage.addAttributes(
                [.foregroundColor: NSColor(Theme.Syntax.plain), .font: Self.font],
                range: whole
            )
            var lineStart = 0
            for (index, line) in string.split(separator: "\n", omittingEmptySubsequences: false).enumerated() {
                if index < spans.count {
                    for span in spans[index] {
                        guard let range = Self.range(of: span, in: line) else { continue }
                        var attributes: [NSAttributedString.Key: Any] = [.foregroundColor: NSColor(span.role.color)]
                        if span.role == .comment { attributes[.font] = Self.italic }
                        storage.addAttributes(
                            attributes,
                            range: NSRange(location: lineStart + range.location, length: range.length)
                        )
                    }
                }
                lineStart += line.utf16.count + 1
            }
            storage.endEditing()
            textView.typingAttributes[.foregroundColor] = NSColor(Theme.Syntax.plain)
            textView.typingAttributes[.font] = Self.font
        }

        /// A span's UTF-8 byte offsets as a UTF-16 range within its line.
        private static func range(of span: Span, in line: Substring) -> NSRange? {
            let utf8 = line.utf8
            guard let from = utf8.index(utf8.startIndex, offsetBy: Int(span.start), limitedBy: utf8.endIndex),
                  let to = utf8.index(utf8.startIndex, offsetBy: Int(span.end), limitedBy: utf8.endIndex),
                  from <= to
            else { return nil }
            let location = line.utf16.distance(from: line.startIndex, to: from)
            let length = line.utf16.distance(from: from, to: to)
            return NSRange(location: location, length: length)
        }
    }
}

#Preview("Edit block", traits: .fixedLayout(width: 1100, height: 200)) {
    if let editor = PreviewSample.editor({ $0.startEditing(1) }), let block = editor.editBlock {
        EditBlockView(viewModel: block, widths: PaneWidths(total: 1100))
    }
}

#Preview("Edit block, longer draft", traits: .fixedLayout(width: 1100, height: 300)) {
    if let editor = PreviewSample.editor({ editor in
        editor.startEditing(1)
        editor.editBlock?.draft = "    // Stop every upload, then keep the queue for later.\n    queue.cancelAll()\n    queue.suspend()\n    log(\"cancelled\")\n"
    }), let block = editor.editBlock {
        EditBlockView(viewModel: block, widths: PaneWidths(total: 1100))
            .preferredColorScheme(.dark)
    } else {
        Text("No edit block")
    }
}

#Preview("Code editor", traits: .fixedLayout(width: 420, height: 120)) {
    @Previewable @State var text = "func cancel() {\n    queue.cancelAll()\n}\n"
    CodeTextView(text: $text)
        .padding(8)
}
