import Foundation
import Observation

/// The window's content: the merge editor once a file is open, or the
/// overview of a repository's operation, from which files open.
@MainActor
@Observable
final class RootViewModel {
    private(set) var editor: MergeEditorViewModel?
    /// The repository's operation (S1); the editor goes back to it.
    private(set) var overview: OverviewViewModel?
    /// What differs between two versions of the repository (S17), outside
    /// an operation; a conflicted file opens in the editor and comes back.
    private(set) var changes: ChangesViewModel?
    /// Why the last file could not be opened.
    private(set) var openError: String?
    /// What could not be opened over the overview, shown as an alert
    /// until dismissed: the empty window, which shows `openError`, is
    /// not on display then.
    var failedOverOverview: Bool {
        (overview != nil || changes != nil) && editor == nil && opening == nil && openError != nil
    }
    /// The name of the file being opened, while it is.
    private(set) var opening: String?
    /// Launched by `git mergetool` (through `git-scry resolve --app`): the
    /// app answers with its exit code, and quits once the file is written.
    private(set) var isMergetool = false
    /// The window was asked to close with choices not written yet.
    var confirmingClose = false
    /// The Keyboard Shortcuts sheet, while it is shown.
    private(set) var shortcuts: KeyboardShortcutsViewModel?

    /// Under `git difftool`, the user asked for no other file.
    private(set) var difftoolStopped = false
    /// Under `git difftool`, the window's Next File or Stop was pressed:
    /// the window closes.
    private(set) var difftoolLeft = false

    /// The user agreed to lose the unwritten choices.
    @ObservationIgnored private var closeConfirmed = false
    /// The editor `git mergetool` gave the window: only its file being
    /// written answers 0.
    @ObservationIgnored private var mergetoolEditor: MergeEditorViewModel?

    /// The repositories opened lately, for the window with nothing open.
    @ObservationIgnored let recents: RecentRepositories
    /// What waits in a repository: decides between the overview and the
    /// changes.
    @ObservationIgnored private let waiting: @Sendable (String) -> RepositoryWaiting

    /// `defaults` keeps the recent repositories; tests pass their own.
    init(
        defaults: UserDefaults = .standard,
        waiting: @escaping @Sendable (String) -> RepositoryWaiting = { repositoryWaiting(path: $0) }
    ) {
        recents = RecentRepositories(defaults: defaults)
        self.waiting = waiting
    }

    /// The list of recent repositories, for the window with nothing open.
    func openRepositoryList() -> OpenRepositoryViewModel {
        OpenRepositoryViewModel(recents: recents, waiting: waiting) { [weak self] path in
            Task { await self?.openRepository(path) }
        }
    }

    /// Shows every shortcut, those of the screen on display first.
    func showShortcuts() {
        let current: Shortcut.Section? = if let editor {
            editor.isEditing ? .editing : .editor
        } else if changes != nil {
            .changes
        } else if overview != nil {
            .overview
        } else {
            nil
        }
        shortcuts = KeyboardShortcutsViewModel(current: current)
    }

    func closeShortcuts() {
        shortcuts = nil
    }

    /// Opens what the command line gives: the files of `git mergetool`, a
    /// repository's folder, or a file.
    func openLaunchFile(arguments: [String] = CommandLine.arguments) async {
        guard editor == nil, overview == nil, changes == nil else { return }
        if let files = Self.mergetoolFiles(in: arguments) {
            await open(mergetool: files)
        } else if let files = Self.difftoolFiles(in: arguments) {
            open(changes: ChangeSet.openDifftool(old: files.old, new: files.new, name: files.name), queue: files.queue)
        } else if let request = Self.changesRequest(in: arguments) {
            await openChanges(request)
        } else if let path = Self.launchPath(in: arguments) {
            var isDirectory: ObjCBool = false
            if FileManager.default.fileExists(atPath: path, isDirectory: &isDirectory), isDirectory.boolValue {
                // `git scry` passes the root alone for an operation.
                await openOverview(path)
            } else {
                await open(path)
            }
        }
    }

    /// Shows the repository holding `path`: its merge, rebase,
    /// cherry-pick or revert in progress, or files left with conflicts;
    /// otherwise what is not committed, as `git scry` does.
    func openRepository(_ path: String) async {
        guard opening == nil else { return }
        let waiting = waiting
        if case .resolving = await Task.detached(operation: { waiting(path) }).value {
            await openOverview(path)
        } else {
            await openChanges(ChangesRequest(root: path))
        }
    }

    /// Shows the operation in progress in the repository holding `path`.
    func openOverview(_ path: String) async {
        guard opening == nil else { return }
        opening = URL(fileURLWithPath: path).lastPathComponent
        defer { opening = nil }
        do {
            let (repository, snapshot) = try await Task.detached {
                let repository = try RepositoryOverview.open(path: path)
                return (repository, try repository.snapshot())
            }.value
            await open(overview: repository, snapshot: snapshot)
            openError = nil
            recents.add(snapshot.root)
        } catch {
            openError = error.message
        }
    }

    /// Shows an overview read from `source`, starting from `snapshot` when
    /// it was read already.
    func open(overview source: OverviewSource, snapshot: OverviewSnapshot? = nil) async {
        editor = nil
        overview = OverviewViewModel(source: source, snapshot: snapshot) { [weak self] file in
            Task { await self?.open(fromOverview: file) }
        }
        OpenRepository.path = overview?.snapshot?.root
    }

    /// What `git scry diff` asks the window to show.
    struct ChangesRequest: Equatable, Sendable {
        /// The repository, by any folder in it.
        var root: String
        /// The index in place of the working copy.
        var staged = false
        /// A commit, `a..b` or `a...b`; everything not committed without.
        var revisions: String?
        /// Only these files or folders, from the repository's root.
        var paths: [String] = []
    }

    /// `--diff <root> [--staged] [--revisions <r>] [--path <p>]…`, as
    /// `git-scry diff` passes them: each value after its option, since
    /// AppKit takes a bare path for a file to open.
    static func changesRequest(in arguments: [String]) -> ChangesRequest? {
        guard let root = value(of: "--diff", in: arguments) else { return nil }
        var request = ChangesRequest(root: root)
        request.staged = arguments.contains("--staged")
        request.revisions = value(of: "--revisions", in: arguments)
        request.paths = arguments.indices
            .filter { arguments[$0] == "--path" && $0 + 1 < arguments.count }
            .map { arguments[$0 + 1] }
        return request
    }

    /// The two files, or two folders, `git difftool` hands over.
    struct DifftoolFiles: Equatable, Sendable {
        var old: String
        var new: String
        /// The file's path in the repository; empty when git gave none.
        var name = ""
        /// The file's place among those git opens one after the other,
        /// when there are several.
        var queue: ChangesViewModel.Queue?
    }

    /// `--difftool --old <o> --new <n> [--name <path>]`, as `git-scry
    /// difftool` passes them.
    static func difftoolFiles(in arguments: [String]) -> DifftoolFiles? {
        guard arguments.contains("--difftool"),
              let old = value(of: "--old", in: arguments),
              let new = value(of: "--new", in: arguments)
        else { return nil }
        var queue: ChangesViewModel.Queue?
        if let place = value(of: "--place", in: arguments).flatMap(Int.init),
           let total = value(of: "--of", in: arguments).flatMap(Int.init),
           total > 1, (1...total).contains(place) {
            queue = ChangesViewModel.Queue(place: place, total: total)
        }
        return DifftoolFiles(old: old, new: new, name: value(of: "--name", in: arguments) ?? "", queue: queue)
    }

    /// Shows what `request` compares in its repository.
    func openChanges(_ request: ChangesRequest) async {
        guard opening == nil else { return }
        opening = URL(fileURLWithPath: request.root).lastPathComponent
        defer { opening = nil }
        do {
            let source = try await Task.detached {
                try ChangeSet.open(
                    path: request.root, staged: request.staged, revisions: request.revisions, paths: request.paths
                )
            }.value
            open(changes: source)
            openError = nil
            recents.add(source.header().root)
        } catch {
            openError = error.message
        }
    }

    /// Shows the changes read from `source`.
    func open(changes source: ChangesSource, queue: ChangesViewModel.Queue? = nil) {
        editor = nil
        overview = nil
        let root = source.header().root
        changes = ChangesViewModel(
            source: source,
            queue: queue,
            leaveQueue: { [weak self] stop in
                self?.difftoolStopped = stop
                self?.difftoolLeft = true
            },
            openConflicted: { [weak self] path in
                Task { await self?.open(fromChanges: URL(fileURLWithPath: root).appendingPathComponent(path).path) }
            }
        )
        OpenRepository.path = root.isEmpty ? nil : root
    }

    /// Opens a conflicted file of the changes in the editor, with a way
    /// back.
    private func open(fromChanges path: String) async {
        await open(path)
        editor?.onBack = { [weak self] in self?.leaveEditor() }
    }

    func dismissOpenError() {
        openError = nil
    }

    /// Opens a file of the overview in the editor, with a way back.
    private func open(fromOverview file: OverviewFile) async {
        await open(file.absolutePath)
        editor?.onBack = { [weak self] in self?.leaveEditor() }
    }

    /// Back from the editor to the overview or the changes, read again.
    func leaveEditor() {
        guard overview != nil || changes != nil else { return }
        editor = nil
        overview?.refresh()
        if let changes {
            Task { await changes.load() }
        }
    }

    /// The file opened from the overview is written: back to the list.
    var returnsToOverview: Bool {
        (overview != nil || changes != nil) && !isMergetool && editor?.written == true
    }

    func open(_ path: String) async {
        await open(named: path) { try await MergeModel.open(path: path) }
    }

    func open(mergetool files: MergetoolFiles) async {
        isMergetool = true
        await open(named: files.merged) { try await MergeModel.open(mergetool: files) }
        mergetoolEditor = editor
    }

    /// One file or repository at a time: a second open while the first
    /// one loads is ignored, so the slower one cannot replace the editor
    /// the user is already using.
    private func open(named path: String, _ load: () async throws -> MergeModel) async {
        guard opening == nil else { return }
        opening = URL(fileURLWithPath: path).lastPathComponent
        defer { opening = nil }
        do {
            editor = MergeEditorViewModel(model: try await load())
            openError = nil
            if overview == nil, changes == nil { OpenRepository.path = path }
        } catch {
            openError = error.message
        }
    }

    // MARK: Merge tool

    /// What `git mergetool` reads once the app quits: 0 when the file was
    /// written, so git stages it, 1 when it is left conflicted. Outside the
    /// merge tool, 0.
    var exitStatus: Int32 {
        // `git-scry difftool` reads 1 as Stop, and opens no other file.
        if difftoolStopped { return 1 }
        guard isMergetool else { return 0 }
        return mergetoolEditor?.everWritten == true ? 0 : 1
    }

    /// Whether File > Open can replace what the window shows: not over a
    /// file in the editor, whose choices may not be written, nor under
    /// `git mergetool`, which gave the window its file.
    var canOpen: Bool {
        !isMergetool && editor == nil && opening == nil
    }

    /// The merge tool's file is written, or the diff tool's window was
    /// left by its own buttons: the window closes, and git moves on.
    var finished: Bool {
        (isMergetool && editor?.written == true) || difftoolLeft
    }

    /// `--base B --local L --remote R --merged M`, as `git-scry resolve
    /// --app` passes them: each path after its option, since AppKit takes
    /// a bare path for a file to open, and SwiftUI then opens no window.
    static func mergetoolFiles(in arguments: [String]) -> MergetoolFiles? {
        guard let base = value(of: "--base", in: arguments),
              let local = value(of: "--local", in: arguments),
              let remote = value(of: "--remote", in: arguments),
              let merged = value(of: "--merged", in: arguments)
        else { return nil }
        return MergetoolFiles(base: base, local: local, remote: remote, merged: merged)
    }

    /// Where to write the exit status: `git-scry resolve` opens the app
    /// through `open`, which does not pass the app's exit code on.
    static func resultPath(in arguments: [String]) -> String? {
        value(of: "--result", in: arguments)
    }

    /// The argument after `option`.
    private static func value(of option: String, in arguments: [String]) -> String? {
        guard let index = arguments.firstIndex(of: option), index + 1 < arguments.count else {
            return nil
        }
        return arguments[index + 1]
    }

    // MARK: Closing

    /// Whether the window may close now. With choices not written, it asks
    /// first: the window closes once the user confirms with
    /// `closeWithoutWriting`.
    func shouldClose() -> Bool {
        guard !closeConfirmed, editor?.hasUnwrittenWork == true else { return true }
        confirmingClose = true
        return false
    }

    /// Lets the window close, losing the unwritten choices.
    func closeWithoutWriting() {
        closeConfirmed = true
    }

    /// What the empty window says: under `git mergetool`, only that the
    /// file could not be opened, since another one would not answer git.
    var emptyMessage: String {
        isMergetool
            ? "git mergetool gave Scry a file it could not open. Quit Scry to hand it back to git, which leaves it conflicted."
            : "Open a repository with an operation in progress, a file with conflict markers, or run Scry from git mergetool."
    }

    var closeMessage: String {
        guard let editor else { return "" }
        return "\(editor.resolvedSummary). Your choices are not written to \(editor.title) and will be lost."
    }

    /// The first argument naming an existing file. Xcode adds its own
    /// arguments (`-NSDocumentRevisionsDebugMode YES`), so others are skipped.
    static func launchPath(in arguments: [String]) -> String? {
        arguments.dropFirst().first { argument in
            !argument.hasPrefix("-") && FileManager.default.fileExists(atPath: argument)
        }
    }
}
