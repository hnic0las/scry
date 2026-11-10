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

    init() throws {
        let directory = FileManager.default.temporaryDirectory
            .appending(path: "ScryTests-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        url = directory.appending(path: "Sample.swift")
        try Self.text.write(to: url, atomically: true, encoding: .utf8)
    }

    var contents: String {
        (try? String(contentsOf: url, encoding: .utf8)) ?? ""
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
