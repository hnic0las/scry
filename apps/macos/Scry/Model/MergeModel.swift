import Foundation
import Observation

/// The open merge: the model every view model shares. Every fact comes from
/// the Rust core (`MergeDocument`); this keeps the latest snapshot, the
/// state shared across screens (the focused conflict, the one being edited)
/// and sends the user's actions to the core as commands, reading the
/// snapshot again when the events say the choices changed. Views never use
/// it directly: they go through their view model.
@MainActor
@Observable
final class MergeModel {
    let header: Header
    private(set) var rows: [Row] = []
    /// The widest line of any pane, in columns: how far the panes can
    /// scroll sideways.
    private(set) var longestLine = 0
    /// Every conflict; only the focused one is sure to have its candidates
    /// checked by parsing (`checked`), since checking them all took seconds
    /// on a large file.
    private(set) var conflicts: [ConflictInfo] = []
    private(set) var status: Status
    /// The focused conflict, from 0.
    private(set) var current = 0
    /// The conflict open for editing by hand.
    private(set) var editing: Int?
    /// The result was written to disk and has not changed since.
    private(set) var written = false
    /// The result was written at least once since the file was opened: the
    /// file on disk is resolved, whatever changed after.
    private(set) var everWritten = false
    /// Each conflict's base lines are shown above its block (⌥⌘B).
    private(set) var showsBase = false
    /// The last action the core refused, until it is shown.
    var failure: String?
    /// Goes up whenever what the editor shows changes (the snapshot, the
    /// focus, the conflict being edited), so view models can keep what
    /// they derive from it until then.
    private(set) var revision = 0
    /// Goes up whenever the core's snapshot changes (rows, conflicts), but
    /// not for a change of focus: the editor's rows depend on it, and only
    /// the lens rows depend on the focus.
    private(set) var snapshot = 0

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

    /// Opens the files `git mergetool` passes off the main thread, as
    /// `git-scry resolve` does: the base is rebuilt when the file has none.
    static func open(mergetool files: MergetoolFiles) async throws -> MergeModel {
        let document = try await Task.detached {
            try MergeDocument.openMergetool(
                base: files.base,
                local: files.local,
                remote: files.remote,
                merged: files.merged
            )
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

    /// Shows or hides the base lines: the core lays the rows out again.
    func setShowsBase(_ shows: Bool) {
        guard shows != showsBase else { return }
        showsBase = shows
        refresh()
    }

    func focus(_ index: Int) {
        guard conflicts.indices.contains(index), index != current else { return }
        current = index
        checkCurrent()
        revision += 1
    }

    // MARK: Resolution

    func choose(_ candidate: String, conflict index: Int) {
        send(.choose(conflict: UInt32(index), candidate: candidate))
    }

    func undo() {
        send(.undo)
    }

    func redo() {
        send(.redo)
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
    /// The file is no longer what was read or last written: a save
    /// without `overwrite` would be refused.
    func changedOnDisk() -> Bool {
        document.changedOnDisk()
    }

    func save(overwrite: Bool = false) -> SaveOutcome {
        do {
            try apply(.save(overwrite: overwrite))
            return .written
        } catch let error as ScryError {
            if case .ChangedOnDisk = error { return .changedOnDisk }
            failure = error.message
            return .failed
        } catch {
            failure = error.message
            return .failed
        }
    }

    // MARK: Editing by hand

    /// Opens a conflict for editing; one at a time.
    func startEditing(_ index: Int) {
        guard conflicts.indices.contains(index) else { return }
        current = index
        checkCurrent()
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
        send(.edit(conflict: UInt32(index), text: text))
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

    /// Sends a command, keeping the reason when the core refuses it.
    /// Tells whether the core took it.
    @discardableResult
    private func send(_ command: Command) -> Bool {
        do {
            try apply(command)
            return true
        } catch {
            failure = error.message
            return false
        }
    }

    /// Sends a command and follows what it changed. A command that changes
    /// nothing returns no event, and nothing is read again.
    private func apply(_ command: Command) throws {
        for event in try document.apply(command: command) {
            switch event {
            case .choicesChanged:
                written = false
                refresh()
            case .saved:
                written = true
                everWritten = true
            case .undone, .redone, .historyChanged:
                // The status read with the choices says what undo can do.
                break
            }
        }
    }

    private func refresh() {
        revision += 1
        snapshot += 1
        rows = document.rows(showBase: showsBase)
        longestLine = rows.reduce(0) { longest, row in
            guard case .code(let ours, let result, let theirs, _) = row else { return longest }
            return max(longest, Int(ours.columns), Int(result.columns), Int(theirs.columns))
        }
        conflicts = document.conflicts()
        checkCurrent()
        status = document.status()
    }

    /// Reads the focused conflict with its candidates checked, once: the
    /// core keeps the check.
    private func checkCurrent() {
        guard conflicts.indices.contains(current), !conflicts[current].checked,
              let checked = document.conflict(index: UInt32(current))
        else { return }
        conflicts[current] = checked
        snapshot += 1
    }
}

/// The four files `git mergetool` passes to a merge tool, as absolute paths.
struct MergetoolFiles: Equatable {
    let base: String
    let local: String
    let remote: String
    let merged: String
}
