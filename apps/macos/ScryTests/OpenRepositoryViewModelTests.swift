import Foundation
import Testing
@testable import Scry

@MainActor
struct OpenRepositoryViewModelTests {
    /// The recent repositories keep the latest first, once each, and no
    /// more than eight: the list stays one glance long.
    @Test func keepsTheLatestRepositoriesFirst() {
        let recents = RecentRepositories(defaults: .isolated)
        #expect(recents.paths.isEmpty)
        for index in 1...10 {
            recents.add("/r/\(index)")
        }
        recents.add("/r/5")
        #expect(recents.paths.count == RecentRepositories.limit)
        #expect(recents.paths.prefix(3) == ["/r/5", "/r/10", "/r/9"])
        #expect(recents.paths.filter { $0 == "/r/5" }.count == 1)

        recents.remove("/r/10")
        #expect(recents.paths.prefix(2) == ["/r/5", "/r/9"])
    }

    /// Each repository says what waits in it once read, "Checking…" until
    /// then: conflicts first in urgency, then what is not committed.
    @Test func saysWhatWaitsInEachRepository() async {
        let recents = RecentRepositories(defaults: .isolated)
        for path in ["/r/gone", "/r/clean", "/r/dirty", "/r/one", "/r/merge", "/r/rebase"] {
            recents.add(path)
        }
        let viewModel = OpenRepositoryViewModel(recents: recents, waiting: { path in
            switch path {
            case "/r/rebase": .resolving(headline: "Rebasing feature onto main", unresolved: 3)
            case "/r/merge": .resolving(headline: "Merging feature into main", unresolved: 0)
            case "/r/one": .notCommitted(files: 1)
            case "/r/dirty": .notCommitted(files: 12)
            case "/r/clean": .nothing
            default: .missing
            }
        }) { _ in }
        #expect(viewModel.rows.map(\.status).allSatisfy { $0 == "Checking…" })
        #expect(viewModel.selection == "/r/rebase")

        await viewModel.check()
        #expect(viewModel.rows.map(\.status) == [
            "Rebasing feature onto main · 3 files unresolved",
            "Merging feature into main",
            "1 file not committed",
            "12 files not committed",
            "Nothing waiting",
            "Folder not found",
        ])
        #expect(viewModel.rows.map(\.isResolving) == [true, true, false, false, false, false])
        #expect(viewModel.rows[0].name == "rebase")
        #expect(viewModel.rows.last?.isMissing == true)
    }

    /// Return opens the selected repository; one whose folder is gone
    /// does not open, and can be removed from the list.
    @Test func opensTheSelectedRepositoryAndRemovesAMissingOne() async {
        let recents = RecentRepositories(defaults: .isolated)
        recents.add("/r/gone")
        recents.add("/r/here")
        var opened: [String] = []
        let viewModel = OpenRepositoryViewModel(
            recents: recents, waiting: { $0 == "/r/gone" ? .missing : .nothing }
        ) { opened.append($0) }
        await viewModel.check()

        viewModel.openSelected()
        #expect(opened == ["/r/here"])

        // The arrows move the selection and stop at the ends.
        #expect(viewModel.canOpenSelected)
        viewModel.moveSelection(by: -1)
        #expect(viewModel.selection == "/r/here")
        viewModel.moveSelection(by: 1)
        viewModel.moveSelection(by: 1)
        #expect(viewModel.selection == "/r/gone")
        #expect(!viewModel.canOpenSelected)
        viewModel.moveSelection(by: -1)

        let gone = viewModel.rows[1]
        viewModel.open(gone)
        #expect(opened == ["/r/here"])
        #expect(viewModel.selection == "/r/gone")

        viewModel.remove(gone)
        #expect(viewModel.rows.map(\.path) == ["/r/here"])
        #expect(viewModel.selection == "/r/here")
        #expect(recents.paths == ["/r/here"])
    }

    /// The path is shown from the home folder, as the Finder would.
    @Test func showsPathsFromTheHomeFolder() {
        let row = OpenRepositoryViewModel.Row(path: NSHomeDirectory() + "/code/scry")
        #expect(row.folder == "~/code/scry")
        #expect(OpenRepositoryViewModel.Row(path: "/opt/repo").folder == "/opt/repo")
    }
}
