import Foundation
import Observation

/// The open merge, as the views see it. Every fact comes from the Rust core
/// (`MergeDocument`); this only keeps the latest snapshot and forwards the
/// user's actions.
@MainActor
@Observable
final class EditorModel {
    let document: MergeDocument
    let header: Header
    private(set) var rows: [Row] = []
    /// The longest line of any pane, in characters: how far the panes can
    /// scroll sideways.
    private(set) var longestLine = 0
    private(set) var conflicts: [ConflictInfo] = []
    private(set) var status: Status
    /// The focused conflict, from 0.
    var current = 0
    var errorMessage: String?
    /// The conflict being edited by hand, and the text being typed.
    private(set) var editing: Int?
    var draft = ""
    private(set) var written = false

    init(path: String) throws {
        document = try MergeDocument.open(path: path)
        header = document.header()
        status = document.status()
        refresh()
    }

    var currentConflict: ConflictInfo? {
        conflicts.indices.contains(current) ? conflicts[current] : nil
    }

    var allResolved: Bool {
        status.total > 0 && status.resolved == status.total
    }

    var mergedDoesNotParse: Bool {
        if case .doesNotParse = status.merged { return true }
        return false
    }

    func choose(_ candidate: String, conflict index: Int? = nil) {
        let index = index ?? current
        do {
            try document.choose(conflict: UInt32(index), candidate: candidate)
            written = false
            refresh()
        } catch {
            errorMessage = "\(error)"
        }
    }

    /// Takes a candidate for the focused conflict, when it offers it.
    func take(_ candidate: String) {
        guard let conflict = currentConflict,
              conflict.candidates.contains(where: { $0.id == candidate })
        else { return }
        choose(candidate)
    }

    /// Both sides, ours first; pressed again, theirs first.
    func takeBoth() {
        let id = currentConflict?.chosen == "ours-then-theirs" ? "theirs-then-ours" : "ours-then-theirs"
        take(id)
    }

    /// Opens the focused conflict's text for editing, from its current
    /// resolution, or from ours while unresolved.
    func startEditing(conflict index: Int? = nil) {
        let index = index ?? current
        guard conflicts.indices.contains(index) else { return }
        current = index
        draft = conflicts[index].editText
        editing = index
    }

    /// Resolves the edited conflict with the draft; the core checks it.
    func applyEdit() {
        guard let index = editing else { return }
        do {
            try document.edit(conflict: UInt32(index), text: draft)
            editing = nil
            written = false
            refresh()
        } catch {
            errorMessage = "\(error)"
        }
    }

    /// Spans per line of the text being typed, coloured in place in the
    /// file by the core.
    func highlightDraft(_ text: String) -> [[Span]] {
        guard let editing else { return [] }
        return document.highlightDraft(conflict: UInt32(editing), text: text)
    }

    /// What Return inserts in the editor, from the core.
    func newline(before: String, after: String) -> Newline {
        document.newline(before: before, after: after)
    }

    func cancelEdit() {
        editing = nil
    }

    func undo() {
        if document.undo() {
            written = false
            refresh()
        }
    }

    func redo() {
        if document.redo() {
            written = false
            refresh()
        }
    }

    func next() {
        if current + 1 < conflicts.count { current += 1 }
    }

    func previous() {
        if current > 0 { current -= 1 }
    }

    func save() {
        do {
            try document.save()
            written = true
        } catch {
            errorMessage = "\(error)"
        }
    }

    /// The row index of a conflict's lens row, to scroll to it.
    func lensRow(of conflict: Int) -> Int? {
        rows.firstIndex { row in
            if case .lens(let index) = row { return Int(index) == conflict }
            return false
        }
    }

    /// The chosen candidate of a conflict, if any.
    func chosenCandidate(of conflict: Int) -> CandidateInfo? {
        guard conflicts.indices.contains(conflict), let id = conflicts[conflict].chosen else { return nil }
        let info = conflicts[conflict]
        if id == CandidateInfo.editedID {
            // Not a candidate the core offers: shown like one, with its check.
            let lines = info.editText.split(separator: "\n", omittingEmptySubsequences: false).dropLast()
            return CandidateInfo(
                id: CandidateInfo.editedID,
                title: "Edited by you",
                subtitle: "Written by hand",
                preview: lines.prefix(Metrics.previewLines).map(String.init),
                truncated: lines.count > Metrics.previewLines,
                parse: info.editedParse ?? .notChecked,
                suggested: false
            )
        }
        return info.candidates.first { $0.id == id }
    }

    private func refresh() {
        rows = document.rows()
        longestLine = rows.reduce(0) { longest, row in
            guard case .code(let ours, let result, let theirs, _) = row else { return longest }
            return max(longest, ours.text.count, result.text.count, theirs.text.count)
        }
        conflicts = document.conflicts()
        status = document.status()
    }
}
