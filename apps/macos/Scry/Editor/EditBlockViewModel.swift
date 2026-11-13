import Foundation
import Observation

/// A conflict's text being edited by hand: the draft, and what the core
/// says about it as it is typed (colours, indentation).
///
/// The draft always breaks lines with `\n`, whatever the file uses: Swift
/// sees `\r\n` as one character, so lines would not split, and the text
/// view inserts `\n` on Return. The core writes the file's own line break
/// on apply, so the written file keeps one kind.
@MainActor
@Observable
final class EditBlockViewModel {
    let conflict: Int
    var draft: String

    @ObservationIgnored private let model: MergeModel
    @ObservationIgnored private let onClose: () -> Void

    /// Starts from the conflict's current resolution, or from ours while
    /// unresolved. `onClose` runs once the edit is applied or cancelled.
    init(model: MergeModel, conflict: Int, onClose: @escaping () -> Void) {
        self.model = model
        self.conflict = conflict
        self.onClose = onClose
        let text = model.conflict(conflict)?.editText ?? ""
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
        if model.edit(conflict, text: draft) { onClose() }
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
