import Foundation
import Observation

/// The window's content: the merge editor once a file is open.
@MainActor
@Observable
final class RootViewModel {
    private(set) var editor: MergeEditorViewModel?
    /// Why the last file could not be opened.
    private(set) var openError: String?
    /// The name of the file being opened, while it is.
    private(set) var opening: String?
    /// Launched by `git mergetool` (through `git-scry resolve --app`): the
    /// app answers with its exit code, and quits once the file is written.
    private(set) var isMergetool = false
    /// The window was asked to close with choices not written yet.
    var confirmingClose = false

    /// The user agreed to lose the unwritten choices.
    @ObservationIgnored private var closeConfirmed = false

    /// Opens what the command line gives: the files of `git mergetool`, or
    /// a file.
    func openLaunchFile(arguments: [String] = CommandLine.arguments) async {
        guard editor == nil else { return }
        if let files = Self.mergetoolFiles(in: arguments) {
            await open(mergetool: files)
        } else if let path = Self.launchPath(in: arguments) {
            await open(path)
        }
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
