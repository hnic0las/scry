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

private struct DemoEditor: View {
    let operation: String
    var choose: [(Int, String)] = []

    var body: some View {
        if let model = try? EditorModel(path: PreviewFiles.demo(operation)) {
            MergeEditorView(model: model)
                .onAppear {
                    for (conflict, candidate) in choose {
                        model.choose(candidate, conflict: conflict)
                    }
                }
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
