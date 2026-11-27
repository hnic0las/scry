import CoreServices
import Foundation

/// Tells when a repository changed on disk: its working folder, its index
/// or what HEAD names. What the Changes window listens to while it shows
/// the working copy or the index.
enum RepositoryWatcher {
    /// How long the system gathers writes before telling: an agent writing
    /// ten files makes one refresh, not ten.
    static let settle: TimeInterval = 0.4

    /// Whether a change at `path` can change what the window shows.
    ///
    /// In the working folder, anything. In the git folder, only the index,
    /// HEAD and the references: git and Scry write much else there (lock
    /// files, objects, Scry's own notes) that changes no diff.
    nonisolated static func matters(_ path: String, root: String, gitDir: String) -> Bool {
        if let inside = relative(path, to: gitDir) {
            if inside.hasSuffix(".lock") { return false }
            return inside == "index" || inside == "packed-refs" || inside.hasSuffix("HEAD")
                || inside.hasPrefix("refs/")
        }
        return relative(path, to: root) != nil
    }

    /// `path` from `folder`, when it is inside.
    private nonisolated static func relative(_ path: String, to folder: String) -> String? {
        guard !folder.isEmpty, path.hasPrefix(folder) else { return nil }
        let rest = path.dropFirst(folder.count)
        guard rest.hasPrefix("/") else { return nil }
        return String(rest.dropFirst())
    }

    /// The path the system reports events under: links followed, as
    /// `/private/var` for `/var`.
    nonisolated static func real(_ path: String) -> String {
        guard let resolved = realpath(path, nil) else { return path }
        defer { free(resolved) }
        return String(cString: resolved)
    }

    /// One element each time something that matters changed in the
    /// repository at `root`, whose git folder is `gitDir` (inside `root`
    /// or not), once the writes have settled. Ends when no one listens.
    static func changes(root: String, gitDir: String) -> AsyncStream<Void> {
        // Two changes while a refresh runs make one more refresh.
        AsyncStream(bufferingPolicy: .bufferingNewest(1)) { continuation in
            let (root, gitDir) = (real(root), real(gitDir))
            guard !root.isEmpty else { return continuation.finish() }
            let listener = Listener(root: root, gitDir: gitDir) { continuation.yield() }
            var context = FSEventStreamContext(
                version: 0, info: Unmanaged.passRetained(listener).toOpaque(), retain: nil, release: nil,
                copyDescription: nil
            )
            let folders = gitDir.isEmpty || gitDir.hasPrefix(root + "/") ? [root] : [root, gitDir]
            let flags = kFSEventStreamCreateFlagFileEvents | kFSEventStreamCreateFlagUseCFTypes
            guard let stream = FSEventStreamCreate(
                nil,
                { _, info, _, paths, _, _ in
                    guard let info else { return }
                    let listener = Unmanaged<Listener>.fromOpaque(info).takeUnretainedValue()
                    let paths = unsafeBitCast(paths, to: NSArray.self) as? [String] ?? []
                    listener.changed(paths)
                },
                &context, folders as CFArray, FSEventStreamEventId(kFSEventStreamEventIdSinceNow), settle,
                FSEventStreamCreateFlags(flags)
            ) else {
                Unmanaged.passUnretained(listener).release()
                return continuation.finish()
            }
            FSEventStreamSetDispatchQueue(stream, DispatchQueue(label: "scry.repository-watcher"))
            FSEventStreamStart(stream)
            let running = Running(stream: stream, listener: listener)
            continuation.onTermination = { _ in running.stop() }
        }
    }

    /// What the system calls with the paths that changed.
    private final class Listener: Sendable {
        let root: String
        let gitDir: String
        let tell: @Sendable () -> Void

        init(root: String, gitDir: String, tell: @escaping @Sendable () -> Void) {
            self.root = root
            self.gitDir = gitDir
            self.tell = tell
        }

        func changed(_ paths: [String]) {
            if paths.contains(where: { RepositoryWatcher.matters($0, root: root, gitDir: gitDir) }) {
                tell()
            }
        }
    }

    /// A started stream, to stop once.
    private final class Running: @unchecked Sendable {
        private let stream: FSEventStreamRef
        private let listener: Listener

        init(stream: FSEventStreamRef, listener: Listener) {
            self.stream = stream
            self.listener = listener
        }

        func stop() {
            FSEventStreamStop(stream)
            FSEventStreamInvalidate(stream)
            FSEventStreamRelease(stream)
            // The reference the stream's context held.
            Unmanaged.passUnretained(listener).release()
        }
    }
}
