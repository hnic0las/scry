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

/// A merge stopped in a throwaway repository, its file then resolved
/// outside Scry, as an agent would: the first conflict takes the feature's
/// side, the second gets text of its own, with the agent's record. Built
/// with `git`, the user's configuration left out of it.
@MainActor
enum ReviewSample {
    static let base = lines(timeout: "    let timeout: Duration = .seconds(10)\n", cancel: "")
    static let ours = lines(timeout: "    let timeout: Duration = .seconds(30)\n", cancel: "        queue.cancelAll()\n")
    static let theirs = lines(timeout: "    let retries = 3\n", cancel: "        queue.suspend()\n")
    static let resolved = lines(
        timeout: "    let retries = 3\n",
        cancel: agentText
    )
    /// The second conflict as the agent wrote it: no candidate's text.
    static let agentText = "        queue.suspend()\n        queue.cancelAll()\n        paused = true\n"
    static let reason = "Keeps both: the feature's suspend() first so no upload starts while main's cancelAll() runs, then records the pause."

    /// The sample file with the two places the sides change.
    private static func lines(timeout: String, cancel: String) -> String {
        "import Foundation\n\nfinal class Uploader {\n\(timeout)    let name = \"uploads\"\n    let limit = 4\n    let queue = Queue()\n    var paused = false\n}\n\nfunc cancel() {\n\(cancel)}\n"
    }

    /// The resolved file, from a new repository; `nil` when git is missing.
    static func file() -> String? {
        let directory = FileManager.default.temporaryDirectory.appending(path: "ScryReview-\(UUID().uuidString)")
        let file = directory.appending(path: "Uploader.swift")
        let git = { (arguments: [String]) -> String? in
            let process = Process()
            process.executableURL = URL(fileURLWithPath: "/usr/bin/git")
            process.arguments = ["-C", directory.path] + arguments
            process.environment = [
                "GIT_CONFIG_GLOBAL": "/dev/null", "GIT_CONFIG_NOSYSTEM": "1",
                "GIT_AUTHOR_NAME": "Scry", "GIT_AUTHOR_EMAIL": "preview@example.com",
                "GIT_COMMITTER_NAME": "Scry", "GIT_COMMITTER_EMAIL": "preview@example.com",
                "HOME": directory.path, "PATH": "/usr/bin:/bin",
            ]
            let output = Pipe()
            process.standardOutput = output
            process.standardError = Pipe()
            guard (try? process.run()) != nil else { return nil }
            let data = output.fileHandleForReading.readDataToEndOfFile()
            process.waitUntilExit()
            return String(data: data, encoding: .utf8)?.trimmingCharacters(in: .whitespacesAndNewlines)
        }
        let write = { (text: String) in try? text.write(to: file, atomically: true, encoding: .utf8) }
        guard (try? FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)) != nil,
              git(["init", "--quiet", "--initial-branch=main"]) != nil
        else { return nil }
        write(base)
        _ = git(["add", "."]); _ = git(["commit", "--quiet", "-m", "Add the uploader"])
        _ = git(["switch", "--quiet", "-c", "feature"])
        write(theirs)
        _ = git(["commit", "--quiet", "-am", "Pause the queue"])
        _ = git(["switch", "--quiet", "main"])
        write(ours)
        _ = git(["commit", "--quiet", "-am", "Cancel every upload"])
        _ = git(["merge", "--quiet", "feature"])
        // The agent's record for the second conflict, where the MCP
        // server keeps it.
        guard let id = (try? MergeModel(path: file.path))?.conflicts.last?.id,
              let gitDirectory = git(["rev-parse", "--absolute-git-dir"]),
              let head = git(["rev-parse", "HEAD"]),
              let merging = git(["rev-parse", "MERGE_HEAD"])
        else { return nil }
        write(resolved)
        let record: [String: Any] = [
            id: [
                "conflict": id, "by": "claude-code 2.1", "pick": NSNull(),
                "text": agentText,
                "reason": reason, "dropped": "nothing", "at": 1_763_300_000,
            ] as [String: Any],
        ]
        let folder = URL(fileURLWithPath: gitDirectory).appending(path: "scry/review/\(head)-\(merging)")
        guard (try? FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)) != nil,
              let data = try? JSONSerialization.data(withJSONObject: record),
              (try? data.write(to: folder.appending(path: "records-Uploader.swift.json"))) != nil
        else { return nil }
        return file.path
    }

    /// The editor on the resolved file, as the app opens it: for review.
    static func editor(_ setUp: (MergeEditorViewModel) -> Void = { _ in }) -> MergeEditorViewModel? {
        guard let path = file(), let model = try? MergeModel(path: path) else { return nil }
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
