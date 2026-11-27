import SwiftUI

/// A small repository with changes not committed, built in a temporary
/// directory with the user's git, for the Changes window's previews.
@MainActor
enum PreviewChanges {
    static let old = """
    import Foundation

    /// Sends queued items to the server.
    final class SyncEngine {
        private let client: SyncClient
        private var outbox: [Item] = []

        func upload(_ item: Item) async throws {
            try await client.send(item, timeout: .seconds(30))
        }

        func flush() async {
            let batch = outbox.prefix(20)
            for item in batch {
                try? await upload(item)
            }
        }

        func cancelAll() {
            outbox.removeAll()
        }

        var pendingCount: Int { outbox.count }
    }

    """

    static let new = """
    import Foundation

    /// Sends queued items to the server.
    final class SyncEngine {
        private let client: SyncClient
        private var outbox = SyncQueue()
        private let monitor = NetworkMonitor()

        func upload(_ item: Item) async throws {
            guard monitor.isOnline else {
                outbox.enqueue(item)
                return
            }
            try await retry(times: 3, backoff: .exponential) {
                try await client.send(item, timeout: .seconds(30))
            }
        }

        func flush() async {
            let batch = outbox.next(20)
            for item in batch {
                try? await upload(item)
            }
        }

        var pendingCount: Int { outbox.count }

        /// Sends everything that waited while the device was offline, however long the queue has grown since the last flush.
        func retryFailed() async {
            guard monitor.isOnline else { return }
            await flush()
        }
    }

    """

    /// The window on the sample repository, in `layout`.
    static func window(
        _ layout: ChangesViewModel.Layout, select path: String = "Sources/Sync/SyncEngine.swift", viewed: Bool = false,
        selection: ClosedRange<Int>? = nil, explanations: [LineExplanation]? = nil
    ) -> some View {
        let viewModel = repository().map { ChangesViewModel(source: $0, defaults: .preview) }
        viewModel?.layout = layout
        viewModel?.loadNow(select: path)
        if let explanations {
            viewModel?.showNow(
                selection: selection,
                explanations: LineExplanations(available: true, watching: true, items: explanations),
                open: explanations.first?.id
            )
        }
        if viewed, let viewModel {
            // Three files viewed, one of them changed since.
            let names = ["OldCache.swift", "SyncClient.swift", "offline.md"]
            for file in viewModel.files where names.contains(file.name) { viewModel.toggleViewed(file) }
            let changed = URL(fileURLWithPath: viewModel.header.root).appending(path: "Sources/Sync/SyncClient.swift")
            try? "// Changed since it was viewed.\n".write(to: changed, atomically: true, encoding: .utf8)
            viewModel.loadNow(select: path)
        }
        return Group {
            if let viewModel {
                ChangesView(viewModel: viewModel, loads: false)
            } else {
                Text("git could not build the sample repository")
            }
        }
    }

    static func repository() -> ChangeSet? {
        let root = FileManager.default.temporaryDirectory.appending(path: "ScryPreview-\(UUID().uuidString)")
        let write: (String, String) -> Bool = { path, text in
            let url = root.appending(path: path)
            return (try? FileManager.default.createDirectory(
                at: url.deletingLastPathComponent(), withIntermediateDirectories: true
            )) != nil && (try? text.write(to: url, atomically: true, encoding: .utf8)) != nil
        }
        guard write("Sources/Sync/SyncEngine.swift", old),
              write("Sources/Sync/SyncClient.swift", "protocol SyncClient {\n    func send(_ item: Item) async throws\n}\n"),
              write("Sources/Legacy/OldCache.swift", "final class OldCache {\n    var items: [Item] = []\n}\n"),
              write("Sources/Sync/Backoff.swift", "enum Backoff {\n    case fixed\n    case exponential\n}\n"),
              git(root, "init", "--quiet"),
              git(root, "add", "."),
              git(root, "-c", "user.name=Scry", "-c", "user.email=scry@example.com", "-c", "commit.gpgsign=false",
                  "commit", "--quiet", "-m", "base"),
              write("Sources/Sync/SyncClient.swift", "protocol SyncClient {\n    func send(_ item: Item, timeout: Duration) async throws\n}\n"),
              git(root, "add", "Sources/Sync/SyncClient.swift"),
              git(root, "mv", "Sources/Sync/Backoff.swift", "Sources/Sync/RetryPolicy.swift"),
              write("Sources/Sync/SyncEngine.swift", new),
              git(root, "rm", "--quiet", "--cached", "Sources/Legacy/OldCache.swift"),
              (try? FileManager.default.removeItem(at: root.appending(path: "Sources/Legacy/OldCache.swift"))) != nil,
              write("Sources/Sync/SyncQueue.swift", "struct SyncQueue {\n    private var items: [Item] = []\n    var count: Int { items.count }\n}\n"),
              write("docs/offline.md", "# Offline\n\nUploads wait in a queue.\n")
        else { return nil }
        try? Data([0x89, 0x50, 0x4E, 0x47, 0, 1, 2, 3]).write(to: root.appending(path: "icon-sync.png"))
        return try? ChangeSet.open(path: root.path, staged: false, revisions: nil, paths: [])
    }

    /// The history of a file renamed once, with a change not committed.
    static func history(
        _ layout: ChangesViewModel.Layout = .unified, select place: Int = 1, pinned: Bool = false, blame: Bool = false
    ) -> some View {
        let shown = BlameSwitch()
        shown.isOn = blame
        let viewModel = historyRepository().map { HistoryViewModel(source: $0, defaults: .preview, blame: shown) }
        viewModel?.loadNow(select: place, pinned: pinned)
        viewModel?.file?.layout = layout
        return Group {
            if let viewModel {
                HistoryView(viewModel: viewModel, loads: false)
            } else {
                Text("git could not build the sample repository")
            }
        }
    }

    static func historyRepository() -> FileHistory? {
        let root = FileManager.default.temporaryDirectory.appending(path: "ScryPreview-\(UUID().uuidString)")
        let manager = "Sources/Sync/SyncManager.swift"
        let engine = "Sources/Sync/SyncEngine.swift"
        let write: (String, String) -> Bool = { path, text in
            let url = root.appending(path: path)
            return (try? FileManager.default.createDirectory(
                at: url.deletingLastPathComponent(), withIntermediateDirectories: true
            )) != nil && (try? text.write(to: url, atomically: true, encoding: .utf8)) != nil
        }
        let commit: (String, String) -> Bool = { author, message in
            git(root, "add", ".") && git(
                root, "-c", "user.name=\(author)", "-c", "user.email=scry@example.com", "-c", "commit.gpgsign=false",
                "commit", "--quiet", "-m", message
            )
        }
        let first = old.replacingOccurrences(of: "SyncEngine", with: "SyncManager")
            .replacingOccurrences(of: "    func cancelAll() {\n        outbox.removeAll()\n    }\n\n", with: "")
        let renamed = old.replacingOccurrences(of: "prefix(20)", with: "prefix(10)")
        let queued = new.replacingOccurrences(of: "    private let monitor = NetworkMonitor()\n", with: "")
            .replacingOccurrences(of: "            guard monitor.isOnline else { return }\n", with: "")
            .replacingOccurrences(of: "        guard monitor.isOnline else {\n            outbox.enqueue(item)\n            return\n        }\n", with: "")
        guard write(manager, first),
              git(root, "init", "--quiet"),
              git(root, "config", "user.name", "Alex Martin"),
              commit("Sam Lee", "Add the sync manager"),
              write(manager, first.replacingOccurrences(of: "prefix(20)", with: "prefix(10)")),
              commit("Sam Lee", "Send smaller batches"),
              git(root, "mv", manager, engine),
              write(engine, renamed),
              commit("Alex Martin", "Rename SyncManager to SyncEngine, add cancelAll"),
              write(engine, old),
              commit("Sam Lee", "Back to batches of 20"),
              write(engine, queued),
              commit("Alex Martin", "Retry uploads with a backoff, queue in SyncQueue"),
              write(engine, new)
        else { return nil }
        return try? FileHistory.open(path: root.path, file: engine, from: nil)
    }

    private static func git(_ root: URL, _ arguments: String...) -> Bool {
        let process = Process()
        process.executableURL = URL(fileURLWithPath: "/usr/bin/env")
        process.arguments = ["git", "-C", root.path] + arguments
        process.standardOutput = FileHandle.nullDevice
        process.standardError = FileHandle.nullDevice
        guard (try? process.run()) != nil else { return false }
        process.waitUntilExit()
        return process.terminationStatus == 0
    }
}

#Preview("Changes, unified", traits: .fixedLayout(width: 1300, height: 760)) {
    PreviewChanges.window(.unified)
}

#Preview("Changes, split", traits: .fixedLayout(width: 1300, height: 760)) {
    PreviewChanges.window(.split)
}

#Preview("Changes, split, dark", traits: .fixedLayout(width: 1300, height: 760)) {
    PreviewChanges.window(.split)
        .preferredColorScheme(.dark)
}

#Preview("Changes, a binary file", traits: .fixedLayout(width: 1300, height: 760)) {
    PreviewChanges.window(.unified, select: "icon-sync.png")
}

#Preview("Changes, files viewed", traits: .fixedLayout(width: 1300, height: 760)) {
    PreviewChanges.window(.unified, viewed: true)
}

extension PreviewChanges {
    static func explained(_ id: String, _ lines: ClosedRange<UInt32>, _ text: String?, minutes: Double, stale: Bool = false)
        -> LineExplanation {
        let at = UInt64((Date().timeIntervalSince1970 - minutes * 60) * 1000)
        return LineExplanation(
            id: id, first: lines.lowerBound, last: lines.upperBound, stale: stale,
            askedAt: text == nil ? at : nil,
            answer: text.map { Explanation(text: $0, by: "Claude Code", at: at, failed: false) }
        )
    }

    static let uploadText = "These lines make upload work offline. Without a network, the item goes to the outbox and the function returns without sending; otherwise the send runs inside a retry with exponential backoff, with a 30-second timeout per attempt. One thing to check: an item queued here is only sent when flush() runs, and nothing in this change calls flush() when the network comes back."
    static let flushText = "flush now takes its batch from the queue with next(20), which removes the items it returns, where prefix(20) left them in the outbox."
}

#Preview("Explain, a selection", traits: .fixedLayout(width: 1300, height: 760)) {
    PreviewChanges.window(.unified, selection: 10...18, explanations: [])
}

#Preview("Explain, answered and folded", traits: .fixedLayout(width: 1300, height: 760)) {
    PreviewChanges.window(.unified, selection: 10...18, explanations: [
        PreviewChanges.explained("L1", 10...18, PreviewChanges.uploadText, minutes: 2),
        PreviewChanges.explained("L2", 21...26, PreviewChanges.flushText, minutes: 9),
    ])
}

#Preview("Explain, waiting and stale, split, dark", traits: .fixedLayout(width: 1300, height: 760)) {
    PreviewChanges.window(.split, selection: 10...13, explanations: [
        PreviewChanges.explained("L1", 10...13, nil, minutes: 0.1),
        PreviewChanges.explained("L2", 21...26, PreviewChanges.flushText, minutes: 9, stale: true),
    ])
    .preferredColorScheme(.dark)
}

#Preview("History, each change", traits: .fixedLayout(width: 1300, height: 760)) {
    PreviewChanges.history()
}

#Preview("History, pinned, split, dark", traits: .fixedLayout(width: 1300, height: 760)) {
    PreviewChanges.history(.split, select: 3, pinned: true)
        .preferredColorScheme(.dark)
}

#Preview("Blame, unified", traits: .fixedLayout(width: 1300, height: 760)) {
    PreviewChanges.history(select: 0, blame: true)
}

#Preview("Blame, split, dark", traits: .fixedLayout(width: 1300, height: 760)) {
    PreviewChanges.history(.split, select: 0, blame: true)
        .preferredColorScheme(.dark)
}
