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
    /// The window was asked to close with choices not written yet.
    var confirmingClose = false

    /// The user agreed to lose the unwritten choices.
    @ObservationIgnored private var closeConfirmed = false

    /// Opens the file given on the command line, if any.
    func openLaunchFile(arguments: [String] = CommandLine.arguments) async {
        guard editor == nil, let path = Self.launchPath(in: arguments) else { return }
        await open(path)
    }

    func open(_ path: String) async {
        opening = URL(fileURLWithPath: path).lastPathComponent
        defer { opening = nil }
        do {
            editor = MergeEditorViewModel(model: try await MergeModel.open(path: path))
            openError = nil
        } catch {
            openError = "\(error)"
        }
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
