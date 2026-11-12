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
    /// Why the last file could not be opened.
    private(set) var openError: String?
    /// The name of the file being opened, while it is.
    private(set) var opening: String?
    /// Launched by `git mergetool` (through `git-scry resolve --app`): the
    /// app answers with its exit code, and quits once the file is written.
    private(set) var isMergetool = false
    /// The window was asked to close with choices not written yet.
    var confirmingClose = false
    /// The Keyboard Shortcuts sheet, while it is shown.
    private(set) var shortcuts: KeyboardShortcutsViewModel?

    /// The user agreed to lose the unwritten choices.
    @ObservationIgnored private var closeConfirmed = false

    /// Shows every shortcut, those of the screen on display first.
    func showShortcuts() {
        let current: Shortcut.Section? = if let editor {
            editor.isEditing ? .editing : .editor
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
        guard editor == nil, overview == nil else { return }
        if let files = Self.mergetoolFiles(in: arguments) {
            await open(mergetool: files)
        } else if let path = Self.launchPath(in: arguments) {
            var isDirectory: ObjCBool = false
            if FileManager.default.fileExists(atPath: path, isDirectory: &isDirectory), isDirectory.boolValue {
                await openRepository(path)
            } else {
                await open(path)
            }
        }
    }

    /// Shows the operation in progress in the repository holding `path`.
    func openRepository(_ path: String) async {
        opening = URL(fileURLWithPath: path).lastPathComponent
        defer { opening = nil }
        do {
            let repository = try await Task.detached { try RepositoryOverview.open(path: path) }.value
            await open(overview: repository)
            openError = nil
        } catch {
            openError = "\(error)"
        }
    }

    /// Shows an overview read from `source`.
    func open(overview source: OverviewSource) async {
        editor = nil
        overview = OverviewViewModel(source: source) { [weak self] file in
            Task { await self?.open(fromOverview: file) }
        }
    }

    /// Opens a file of the overview in the editor, with a way back.
    private func open(fromOverview file: OverviewFile) async {
        await open(file.absolutePath)
        editor?.onBack = { [weak self] in self?.leaveEditor() }
    }

    /// Back from the editor to the overview, read again.
    func leaveEditor() {
        guard overview != nil else { return }
        editor = nil
        overview?.refresh()
    }

    /// The file opened from the overview is written: back to the list.
    var returnsToOverview: Bool {
        overview != nil && !isMergetool && editor?.written == true
    }

    func open(_ path: String) async {
        await open(named: path) { try await MergeModel.open(path: path) }
    }

    func open(mergetool files: MergetoolFiles) async {
        isMergetool = true
        await open(named: files.merged) { try await MergeModel.open(mergetool: files) }
    }

    private func open(named path: String, _ load: () async throws -> MergeModel) async {
        opening = URL(fileURLWithPath: path).lastPathComponent
        defer { opening = nil }
        do {
            editor = MergeEditorViewModel(model: try await load())
            openError = nil
        } catch {
            openError = "\(error)"
        }
    }

    // MARK: Merge tool

    /// What `git mergetool` reads once the app quits: 0 when the file was
    /// written, so git stages it, 1 when it is left conflicted. Outside the
    /// merge tool, 0.
    var exitStatus: Int32 {
        guard isMergetool else { return 0 }
        return editor?.everWritten == true ? 0 : 1
    }

    /// Whether File > Open can replace what the window shows: not over a
    /// file in the editor, whose choices may not be written, nor under
    /// `git mergetool`, which gave the window its file.
    var canOpen: Bool {
        !isMergetool && editor == nil && opening == nil
    }

    /// The merge tool's file is written: the window closes, and `git
    /// mergetool` moves on to the next file.
    var finished: Bool {
        isMergetool && editor?.written == true
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
