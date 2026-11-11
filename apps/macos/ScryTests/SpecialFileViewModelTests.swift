import Testing
@testable import Scry

@MainActor
struct SpecialFileViewModelTests {
    private func sheet(_ kind: SpecialKind, _ source: FakeOverview, done: @escaping () -> Void = {}) -> SpecialFileViewModel {
        SpecialFileViewModel(
            file: file("Sources/X", .unresolved, openable: kind == .projectFile || kind == .stringCatalog, special: kind),
            source: source,
            done: done,
            openInEditor: { _ in }
        )
    }

    /// A file deleted on one side says who deleted it and who changed it,
    /// then keeping or deleting goes through the core and closes.
    @Test func deletedFileSaysWhoAndIsKeptOrDeleted() {
        let source = FakeOverview(files: [])
        var closed = 0
        let viewModel = sheet(.deletedOnOneSide, source) { closed += 1 }

        #expect(viewModel.explanation == "main · rebase target deleted it; Your commit a1b2c3d · Retry changed it (12 lines). Keep the changed file, or delete it.")
        #expect(viewModel.changesTitle == "What Your commit a1b2c3d · Retry changed, lost if you delete the file")
        #expect(viewModel.changeLines.map(\.kind) == [.hunk, .context, .removed, .added])
        #expect(viewModel.changeLines.map(\.text) == ["@@ -1,2 +1,2 @@", "keep", "old", "new"])
        viewModel.keepFile()
        viewModel.deleteFile()
        #expect(source.actions == [.keepFile(path: "Sources/X"), .deleteFile(path: "Sources/X")])
        #expect(closed == 2)
    }

    /// A binary file shows both versions with their size; a side without
    /// the file cannot be taken.
    @Test func binaryTakesOnlyAnExistingVersion() {
        let source = FakeOverview(files: [])
        let viewModel = sheet(.binary, source)

        #expect(viewModel.versionCards.map(\.side) == ["Yours", "Theirs"])
        #expect(viewModel.versionCards[0].size == "2 KB")
        #expect(viewModel.versionCards[1].size == "No file on this side")
        viewModel.take(.theirs)
        #expect(source.actions.isEmpty)
        viewModel.take(.ours)
        #expect(source.actions == [.takeVersion(path: "Sources/X", version: .ours)])
    }

    /// A project file settles the safe conflicts and opens the editor on
    /// the others, with their reason; references git kept to deleted
    /// objects are named, and removed unless the user unticks it.
    @Test func projectSettlesWhatIsSafeAndLeavesTheRest() {
        let source = FakeOverview(files: [])
        source.projectInfo = ProjectInfo(
            oursLabel: "main", theirsLabel: "feature",
            conflicts: [projectConflict(["A"], ["B"], reason: nil), projectConflict([], [], reason: "MARKETING_VERSION: 1.4 on one side, 2.0 on the other")],
            resolvable: 1, missing: ["A2000002 ForecastView.swift"], missingRemovable: true
        )
        var edited = 0
        let viewModel = SpecialFileViewModel(
            file: file("P", .unresolved, special: .projectFile), source: source, done: {}, openInEditor: { _ in edited += 1 }
        )
        #expect(viewModel.keepBothTitle == "Settle 1 of 2 Conflicts")
        #expect(viewModel.explanation == "1 of 2 conflicts settle by keeping both sides' changes; the merge editor opens on the 1 left.")
        #expect(viewModel.projectRows[1].reason == "MARKETING_VERSION: 1.4 on one side, 2.0 on the other")
        #expect(viewModel.projectWarning?.hasPrefix("Git merged references to objects that no longer exist: A2000002 ForecastView.swift.") == true)
        #expect(viewModel.canRemoveMissing)

        viewModel.removeMissing = false
        viewModel.keepBoth()
        #expect(source.actions == [.keepBothAdditions(path: "P", removeMissing: false)])
        #expect(edited == 1)

        source.projectInfo = ProjectInfo(oursLabel: "main", theirsLabel: "feature", conflicts: [projectConflict([], [], reason: "x")], resolvable: 0, missing: [], missingRemovable: false)
        let refused = sheet(.projectFile, source)
        #expect(!refused.canKeepBoth)
        refused.keepBoth()
        #expect(source.actions.count == 1)
    }

    /// A catalog is written only once every value both sides changed has
    /// a pick, and the picks go in order; a removed value says so.
    @Test func catalogWritesOnceEveryValueIsChosen() {
        let source = FakeOverview(files: [])
        let viewModel = sheet(.stringCatalog, source)

        #expect(viewModel.explanation == "3 changes merged by key. 2 values were changed on both sides: choose each one.")
        #expect(viewModel.catalogRows[1].ours == "(removed)")
        #expect(viewModel.catalogRows[1].base == "—")
        viewModel.choose(.theirs, for: 0)
        viewModel.writeCatalog()
        #expect(source.actions.isEmpty)
        viewModel.choose(.ours, for: 1)
        #expect(viewModel.catalogRows.map(\.pick) == [.theirs, .ours])
        viewModel.writeCatalog()
        #expect(source.actions == [.resolveCatalog(path: "Sources/X", picks: [.theirs, .ours])])
    }

    /// From the overview, a special file opens its sheet instead of the
    /// editor; once settled, the sheet closes and the list is read again.
    @Test func overviewOpensTheSheetAndRefreshes() {
        var edited: [String] = []
        let source = FakeOverview(files: [file("Old.m", .unresolved, openable: false, special: .deletedOnOneSide), file("A.swift", .unresolved)])
        let overview = OverviewViewModel(source: source) { edited.append($0.path) }
        #expect(overview.rows.map(\.canOpen) == [true, true])
        #expect(overview.rows.first?.special == .deletedOnOneSide)

        overview.open("Old.m")
        let special = try? #require(overview.special)
        #expect(edited.isEmpty)
        let reads = source.reads
        special?.keepFile()
        #expect(overview.special == nil)
        #expect(source.reads > reads)
        #expect(overview.rows.first?.state == .staged)

        overview.open("A.swift")
        #expect(edited == ["A.swift"])
    }
}
