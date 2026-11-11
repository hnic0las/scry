import Foundation
import Observation

/// A conflict's text being edited by hand: the draft, and what the core
/// says about it as it is typed (colours, indentation).
///
/// The draft always breaks lines with `\n`, whatever the file uses: Swift
/// sees `\r\n` as one character, so lines would not split, and the text
/// view inserts `\n` on Return. The file's line ending comes back on
/// apply, so the written file keeps one kind.
@MainActor
@Observable
final class EditBlockViewModel {
    let conflict: Int
    var draft: String
    /// The line ending of the conflict's text: `\r\n` or `\n`.
    let lineEnding: String

    @ObservationIgnored private let model: MergeModel
    @ObservationIgnored private let onClose: () -> Void

    /// Starts from the conflict's current resolution, or from ours while
    /// unresolved. `onClose` runs once the edit is applied or cancelled.
    init(model: MergeModel, conflict: Int, onClose: @escaping () -> Void) {
        self.model = model
        self.conflict = conflict
        self.onClose = onClose
        let text = model.conflict(conflict)?.editText ?? ""
        lineEnding = text.contains("\r\n") ? "\r\n" : "\n"
        draft = text.replacingOccurrences(of: "\r\n", with: "\n")
    }

    /// Rows the editor shows: one per line plus one to type into, between
    /// 3 and 20.
    var visibleLines: Int {
        let lines = draft.split(separator: "\n", omittingEmptySubsequences: false).count
        return min(max(lines + 1, 3), 20)
    }

    /// Resolves the conflict with the draft; the editor stays open when the
    /// core refuses it.
    func apply() {
        // Pasted text may bring its own `\r\n`.
        let text = draft
            .replacingOccurrences(of: "\r\n", with: "\n")
            .replacingOccurrences(of: "\n", with: lineEnding)
        if model.edit(conflict, text: text) { onClose() }
    }

    func cancel() {
        onClose()
    }

    func highlight(_ text: String) -> [[Span]] {
        model.highlightDraft(conflict, text: text)
    }

    func newline(before: String, after: String) -> Newline {
        model.newline(conflict, before: before, after: after)
    }

    func typedIndent(before: String, typed: String) -> String? {
        model.typedIndent(conflict, before: before, typed: typed)
    }
}
