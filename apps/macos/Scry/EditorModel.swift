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
        return conflicts[conflict].candidates.first { $0.id == id }
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
