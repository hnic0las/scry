import Foundation
import Observation

/// The repositories opened lately, most recent first, kept in the user's
/// defaults.
struct RecentRepositories {
    static let key = "recentRepositories"
    static let limit = 8

    let defaults: UserDefaults

    var paths: [String] {
        defaults.stringArray(forKey: Self.key) ?? []
    }

    /// Puts `path` first, and drops the oldest past the limit.
    func add(_ path: String) {
        var paths = paths.filter { $0 != path }
        paths.insert(path, at: 0)
        defaults.set(Array(paths.prefix(Self.limit)), forKey: Self.key)
    }

    func remove(_ path: String) {
        defaults.set(paths.filter { $0 != path }, forKey: Self.key)
    }
}

/// S19, the window Scry opens on when it has nothing to show: open a
/// repository, or one of the recent ones, each with what waits in it.
@MainActor
@Observable
final class OpenRepositoryViewModel {
    struct Row: Identifiable, Equatable {
        let path: String
        /// What waits in it; `nil` until it is read.
        var waiting: RepositoryWaiting?
        var id: String { path }

        /// The folder's name.
        var name: String { URL(fileURLWithPath: path).lastPathComponent }

        /// The path, with the home folder as `~`.
        var folder: String {
            let home = NSHomeDirectory()
            return path.hasPrefix(home + "/") ? "~" + path.dropFirst(home.count) : path
        }

        /// "Rebase in progress · 3 files unresolved", "12 files not
        /// committed", "Nothing waiting".
        var status: String {
            switch waiting {
            case nil:
                return "Checking…"
            case .resolving(let headline, let unresolved):
                let files = unresolved == 1 ? "1 file unresolved" : "\(unresolved) files unresolved"
                return unresolved == 0 ? headline : "\(headline) · \(files)"
            case .notCommitted(let files):
                return files == 1 ? "1 file not committed" : "\(files) files not committed"
            case .nothing:
                return "Nothing waiting"
            case .missing:
                return "Folder not found"
            }
        }

        /// Conflicts wait: the row shows the unresolved glyph and colour.
        var isResolving: Bool {
            if case .resolving = waiting { return true }
            return false
        }

        var isMissing: Bool { waiting == .missing }
    }

    private(set) var rows: [Row]
    var selection: String?

    @ObservationIgnored private let recents: RecentRepositories
    @ObservationIgnored private let waiting: @Sendable (String) -> RepositoryWaiting
    @ObservationIgnored private let open: (String) -> Void

    /// `waiting` reads what waits in a repository; `open` opens one.
    init(
        recents: RecentRepositories,
        waiting: @escaping @Sendable (String) -> RepositoryWaiting = { repositoryWaiting(path: $0) },
        open: @escaping (String) -> Void
    ) {
        self.recents = recents
        self.waiting = waiting
        self.open = open
        rows = recents.paths.map { Row(path: $0) }
        selection = rows.first?.path
    }

    /// Reads what waits in each repository, one at a time and off the
    /// main thread, so the window shows at once.
    func check() async {
        let waiting = waiting
        for path in rows.map(\.path) {
            let found = await Task.detached { waiting(path) }.value
            if let index = rows.firstIndex(where: { $0.path == path }) {
                rows[index].waiting = found
            }
        }
    }

    /// Opens the selected repository; a folder that is gone stays listed
    /// until removed.
    func openSelected() {
        guard let row = rows.first(where: { $0.path == selection }), !row.isMissing else { return }
        open(row.path)
    }

    /// Whether Return opens a repository: one is selected and its folder
    /// is there.
    var canOpenSelected: Bool {
        rows.contains { $0.path == selection && !$0.isMissing }
    }

    /// Moves the selection up (-1) or down (1), stopping at the ends.
    func moveSelection(by step: Int) {
        guard let index = rows.firstIndex(where: { $0.path == selection }) else {
            selection = rows.first?.path
            return
        }
        let target = min(max(index + step, 0), rows.count - 1)
        selection = rows[target].path
    }

    func open(_ row: Row) {
        selection = row.path
        openSelected()
    }

    func remove(_ row: Row) {
        recents.remove(row.path)
        rows.removeAll { $0.path == row.path }
        if selection == row.path { selection = rows.first?.path }
    }
}
