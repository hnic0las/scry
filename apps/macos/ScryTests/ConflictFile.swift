import Foundation
@testable import Scry

/// A Swift file with two conflicts, written to a temporary directory outside
/// any repository: the core then reads the sides' labels from the markers.
struct ConflictFile {
    static let text = """
    <<<<<<< HEAD
    let a = 1
    ||||||| base
    let a = 0
    =======
    let a = 2
    >>>>>>> other
    let b = 0
    <<<<<<< HEAD
    let c = 1
    ||||||| base
    let c = 0
    =======
    let c = 2
    >>>>>>> other

    """

    let url: URL

    /// The sample with Windows line endings.
    static let crlfText = text.replacingOccurrences(of: "\n", with: "\r\n")

    init(text: String = Self.text) throws {
        let directory = FileManager.default.temporaryDirectory
            .appending(path: "ScryTests-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        url = directory.appending(path: "Sample.swift")
        try text.write(to: url, atomically: true, encoding: .utf8)
    }

    var contents: String {
        (try? String(contentsOf: url, encoding: .utf8)) ?? ""
    }

    /// The four files `git mergetool` writes for a file in git's default
    /// `merge` style, without the base, in a temporary directory.
    static func mergetool() throws -> MergetoolFiles {
        let directory = FileManager.default.temporaryDirectory
            .appending(path: "ScryTests-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        func write(_ name: String, _ text: String) throws -> String {
            let url = directory.appending(path: name)
            try text.write(to: url, atomically: true, encoding: .utf8)
            return url.path
        }
        return MergetoolFiles(
            base: try write("Sample_BASE.swift", "let a = 0\n"),
            local: try write("Sample_LOCAL.swift", "let a = 1\n"),
            remote: try write("Sample_REMOTE.swift", "let a = 2\n"),
            merged: try write("Sample.swift", "<<<<<<< ours\nlet a = 1\n=======\nlet a = 2\n>>>>>>> theirs\n")
        )
    }

    @MainActor
    func editor() throws -> MergeEditorViewModel {
        MergeEditorViewModel(model: try MergeModel(path: url.path))
    }
}

extension MergeEditorViewModel {
    /// The lens rows, in order.
    var lensRows: [LensRowState] {
        rows.compactMap { row in
            if case .lens(let lens) = row { return lens }
            return nil
        }
    }

    /// The chosen candidate of a conflict, as the inspector shows it.
    func chosen(_ conflict: Int) -> String? {
        focus(conflict)
        return inspector.editedCard?.id ?? inspector.cards.first { $0.selected }?.id
    }
}
