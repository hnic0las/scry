import SwiftUI

/// Previews on the demo repositories built by `scripts/make-conflicts.sh`
/// (run it first). Paths are found from this file's location in the
/// checkout.
enum PreviewFiles {
    static let checkout = URL(fileURLWithPath: #filePath)
        .deletingLastPathComponent() // Scry
        .deletingLastPathComponent() // macos
        .deletingLastPathComponent() // apps
        .deletingLastPathComponent()

    static func demo(_ operation: String) -> String {
        checkout.appending(path: "conflicts/\(operation)/SyncEngine.swift").path
    }
}

/// A small conflicted Swift file, written to a temporary directory, for
/// previews of a single screen: unlike the demos, it needs no script.
@MainActor
enum PreviewSample {
    static let text = """
    import Foundation

    final class Uploader {
    <<<<<<< HEAD
        let timeout: Duration = .seconds(30)
    ||||||| base
        let timeout: Duration = .seconds(10)
    =======
        let retries = 3
    >>>>>>> feature
    }

    func cancel() {
    <<<<<<< HEAD
        queue.cancelAll()
    ||||||| base
    =======
        queue.suspend()
    >>>>>>> feature
    }

    """

    /// The editor on the sample, after `setUp`, as a screen opens it.
    static func editor(_ setUp: (MergeEditorViewModel) -> Void = { _ in }) -> MergeEditorViewModel? {
        let directory = FileManager.default.temporaryDirectory.appending(path: "ScryPreview-\(UUID().uuidString)")
        let url = directory.appending(path: "Uploader.swift")
        guard (try? FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)) != nil,
              (try? text.write(to: url, atomically: true, encoding: .utf8)) != nil,
              let model = try? MergeModel(path: url.path)
        else { return nil }
        // Settings of their own: a preview never changes the user's.
        let editor = MergeEditorViewModel(model: model, defaults: .preview)
        setUp(editor)
        return editor
    }
}

/// A demo editor, set up before the first render: a static preview is
/// captured before `onAppear` runs.
private struct DemoEditor: View {
    @State private var viewModel: MergeEditorViewModel?
    private let scrollX: CGFloat

    /// `choose`: candidates by conflict; `edited`: a conflict resolved by
    /// hand with this text; `editing`: a conflict left open in the editor.
    init(
        operation: String,
        choose: [(Int, String)] = [],
        scrollX: CGFloat = 0,
        edited: (Int, String)? = nil,
        editing: Int? = nil
    ) {
        self.scrollX = scrollX
        var viewModel: MergeEditorViewModel?
        if let model = try? MergeModel(path: PreviewFiles.demo(operation)) {
            for (conflict, candidate) in choose {
                model.choose(candidate, conflict: conflict)
            }
            if let (conflict, text) = edited {
                model.edit(conflict, text: text)
            }
            viewModel = MergeEditorViewModel(model: model, defaults: .preview)
            if let editing {
                viewModel?.startEditing(editing)
            }
        }
        _viewModel = State(initialValue: viewModel)
    }

    var body: some View {
        if let viewModel {
            MergeEditorView(viewModel: viewModel, scrollX: scrollX)
        } else {
            EmptyStateView(message: RootViewModel().emptyMessage, error: "Run scripts/make-conflicts.sh first.", canOpen: true, open: {}, openRepository: {})
        }
    }
}

#Preview("Rebase, unresolved", traits: .fixedLayout(width: 1440, height: 900)) {
    DemoEditor(operation: "rebase")
}

#Preview("Rebase, invalid both", traits: .fixedLayout(width: 1440, height: 900)) {
    DemoEditor(operation: "rebase", choose: [(0, "theirs"), (1, "ours-then-theirs")])
}

#Preview("Rebase, dark", traits: .fixedLayout(width: 1440, height: 900)) {
    DemoEditor(operation: "rebase", choose: [(0, "theirs"), (1, "ours-then-theirs-closed")])
        .preferredColorScheme(.dark)
}

#Preview("Rebase, scrolled sideways", traits: .fixedLayout(width: 1100, height: 600)) {
    DemoEditor(operation: "rebase", scrollX: 120)
}

#Preview("Rebase, editing by hand", traits: .fixedLayout(width: 1440, height: 900)) {
    DemoEditor(operation: "rebase", choose: [(0, "theirs")], editing: 1)
}

#Preview("Rebase, edited by hand", traits: .fixedLayout(width: 1440, height: 900)) {
    DemoEditor(
        operation: "rebase",
        choose: [(0, "theirs")],
        edited: (1, "func cancel() {\n    queue.cancelAll()\n}\n\nfunc pause() {\n    queue.suspend()")
    )
}

extension UserDefaults {
    /// A suite of its own, so a preview never reads or changes the user's
    /// settings.
    static var preview: UserDefaults {
        UserDefaults(suiteName: "ScryPreview.\(UUID().uuidString)")!
    }
}
