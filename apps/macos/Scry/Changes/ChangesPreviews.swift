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
        _ layout: ChangesViewModel.Layout, select path: String = "Sources/Sync/SyncEngine.swift", viewed: Bool = false
    ) -> some View {
        let viewModel = repository().map { ChangesViewModel(source: $0, defaults: .preview) }
        viewModel?.layout = layout
        viewModel?.loadNow(select: path)
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
