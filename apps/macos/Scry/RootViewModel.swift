import Foundation
import Observation

/// The window's content: the merge editor once a file is open.
@MainActor
@Observable
final class RootViewModel {
    private(set) var editor: MergeEditorViewModel?
    /// Why the last file could not be opened.
    private(set) var openError: String?

    /// Opens the file given on the command line, if any.
    func openLaunchFile(arguments: [String] = CommandLine.arguments) {
        guard editor == nil, let path = Self.launchPath(in: arguments) else { return }
        open(path)
    }

    func open(_ path: String) {
        do {
            editor = MergeEditorViewModel(model: try MergeModel(path: path))
            openError = nil
        } catch {
            openError = "\(error)"
        }
    }

    /// The first argument naming an existing file. Xcode adds its own
    /// arguments (`-NSDocumentRevisionsDebugMode YES`), so others are skipped.
    static func launchPath(in arguments: [String]) -> String? {
        arguments.dropFirst().first { argument in
            !argument.hasPrefix("-") && FileManager.default.fileExists(atPath: argument)
        }
    }
}
