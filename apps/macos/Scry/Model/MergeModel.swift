import Foundation
import Observation

/// The open merge: the model every view model shares. Every fact comes from
/// the Rust core (`MergeDocument`); this keeps the latest snapshot, the
/// state shared across screens (the focused conflict, the one being edited)
/// and forwards actions. Views never use it directly: they go through their
/// view model.
@MainActor
@Observable
final class MergeModel {
    let header: Header
    private(set) var rows: [Row] = []
    /// The longest line of any pane, in characters: how far the panes can
    /// scroll sideways.
    private(set) var longestLine = 0
    private(set) var conflicts: [ConflictInfo] = []
    private(set) var status: Status
    /// The focused conflict, from 0.
    private(set) var current = 0
    /// The conflict open for editing by hand.
    private(set) var editing: Int?
    /// The result was written to disk and has not changed since.
    private(set) var written = false
    /// The last action the core refused, until it is shown.
    var failure: String?
    /// Goes up whenever what the editor shows changes (the snapshot, the
    /// focus, the conflict being edited), so view models can keep what
    /// they derive from it until then.
    private(set) var revision = 0

    @ObservationIgnored private let document: MergeDocument

    /// Opens a file off the main thread: checking every candidate and
    /// asking git for the lens commits takes a moment on a large file, and
    /// the window must stay responsive meanwhile.
    static func open(path: String) async throws -> MergeModel {
        let document = try await Task.detached {
            try MergeDocument.open(path: path)
        }.value
        return MergeModel(document: document)
    }

    /// Opens a file on the calling thread, for tests and previews.
    convenience init(path: String) throws {
        self.init(document: try MergeDocument.open(path: path))
    }

    init(document: MergeDocument) {
        self.document = document
        header = document.header()
        status = document.status()
        refresh()
    }

    func conflict(_ index: Int) -> ConflictInfo? {
        conflicts.indices.contains(index) ? conflicts[index] : nil
    }

    // MARK: Focus

    func focus(_ index: Int) {
        guard conflicts.indices.contains(index), index != current else { return }
        current = index
        revision += 1
    }

    // MARK: Resolution

    func choose(_ candidate: String, conflict index: Int) {
        run { try document.choose(conflict: UInt32(index), candidate: candidate) }
    }

    func undo() {
        if document.undo() { changed() }
    }

    func redo() {
        if document.redo() { changed() }
    }

    /// How a save ended.
    enum SaveOutcome {
        case written
        /// The file changed on disk since it was opened; nothing written.
        case changedOnDisk
        case failed
    }

    /// Writes the result. Refuses a file changed on disk since it was
    /// opened, unless `overwrite`.
    @discardableResult
    func save(overwrite: Bool = false) -> SaveOutcome {
        do {
            try document.save(overwrite: overwrite)
            written = true
            return .written
        } catch let error as ScryError {
            if case .ChangedOnDisk = error { return .changedOnDisk }
            failure = "\(error)"
            return .failed
        } catch {
            failure = "\(error)"
            return .failed
        }
    }

    // MARK: Editing by hand

    /// Opens a conflict for editing; one at a time.
    func startEditing(_ index: Int) {
        guard conflicts.indices.contains(index) else { return }
        current = index
        editing = index
        revision += 1
    }

    func stopEditing() {
        editing = nil
        revision += 1
    }

    /// Resolves a conflict with text typed by hand; the core checks it.
    /// Tells whether the core took it.
    @discardableResult
    func edit(_ index: Int, text: String) -> Bool {
        run { try document.edit(conflict: UInt32(index), text: text) }
    }

    /// Spans per line of a draft, coloured in place in the file.
    func highlightDraft(_ index: Int, text: String) -> [[Span]] {
        document.highlightDraft(conflict: UInt32(index), text: text)
    }

    /// What Return inserts in a draft: the text replacing the current line
    /// up to the cursor.
    func newline(_ index: Int, before: String, after: String) -> Newline {
        document.newline(conflict: UInt32(index), before: before, after: after)
    }

    /// The indentation for the draft's current line once `typed` is typed;
    /// `nil` to type it as it is.
    func typedIndent(_ index: Int, before: String, typed: String) -> String? {
        document.typedIndent(conflict: UInt32(index), before: before, typed: typed)
    }

    // MARK: Private

    @discardableResult
    private func run(_ action: () throws -> Void) -> Bool {
        do {
            try action()
            changed()
            return true
        } catch {
            failure = "\(error)"
            return false
        }
    }

    private func changed() {
        written = false
        refresh()
    }

    private func refresh() {
        revision += 1
        rows = document.rows()
        longestLine = rows.reduce(0) { longest, row in
            guard case .code(let ours, let result, let theirs, _) = row else { return longest }
            return max(longest, ours.text.count, result.text.count, theirs.text.count)
        }
        conflicts = document.conflicts()
        status = document.status()
    }
}
