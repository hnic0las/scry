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

    /// The first argument naming an existing file. Xcode adds its own
    /// arguments (`-NSDocumentRevisionsDebugMode YES`), so others are skipped.
    static func launchPath(in arguments: [String]) -> String? {
        arguments.dropFirst().first { argument in
            !argument.hasPrefix("-") && FileManager.default.fileExists(atPath: argument)
        }
    }
}
