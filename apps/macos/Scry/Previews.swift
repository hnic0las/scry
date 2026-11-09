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

/// A demo editor, set up before the first render: a static preview is
/// captured before `onAppear` runs.
private struct DemoEditor: View {
    @State private var model: EditorModel?
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
        let model = try? EditorModel(path: PreviewFiles.demo(operation))
        if let model {
            for (conflict, candidate) in choose {
                model.choose(candidate, conflict: conflict)
            }
            if let (conflict, text) = edited {
                model.startEditing(conflict: conflict)
                model.draft = text
                model.applyEdit()
            }
            if let editing {
                model.startEditing(conflict: editing)
            }
        }
        _model = State(initialValue: model)
    }

    var body: some View {
        if let model {
            MergeEditorView(model: model, scrollX: scrollX)
        } else {
            EmptyStateView(error: "Run scripts/make-conflicts.sh first.", open: {})
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
