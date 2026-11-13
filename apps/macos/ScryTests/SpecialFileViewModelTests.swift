import Testing
@testable import Scry

@MainActor
struct SpecialFileViewModelTests {
    private func sheet(_ kind: SpecialKind, _ source: FakeOverview, done: @escaping () -> Void = {}) -> SpecialFileViewModel {
        SpecialFileViewModel(
            file: file("Sources/X", .unresolved, openable: ![.deletedOnOneSide, .binary, .notMerged, .unusual].contains(kind), special: kind),
            source: source,
            done: done,
            openInEditor: { _ in }
        )
    }

    /// A file deleted on one side says who deleted it and who changed it,
    /// then keeping or deleting goes through the core and closes.
    @Test func deletedFileSaysWhoAndIsKeptOrDeleted() async {
        let source = FakeOverview(files: [])
        var closed = 0
        let viewModel = sheet(.deletedOnOneSide, source) { closed += 1 }

        #expect(viewModel.explanation == "main · rebase target deleted it; Your commit a1b2c3d · Retry changed it (12 lines). Keep the changed file, or delete it.")
        #expect(viewModel.changesTitle == "What Your commit a1b2c3d · Retry changed, lost if you delete the file")
        #expect(viewModel.changeLines.map(\.kind) == [.hunk, .context, .removed, .added])
        #expect(viewModel.changeLines.map(\.text) == ["@@ -1,2 +1,2", "keep", "old", "new"])
        viewModel.keepFile()
        await viewModel.idle()
        viewModel.deleteFile()
        await viewModel.idle()
        #expect(source.actions == [.keepFile(path: "Sources/X"), .deleteFile(path: "Sources/X")])
        #expect(closed == 2)
    }

    /// A binary file shows both versions with their size; a side without
    /// the file cannot be taken.
    @Test func binaryTakesOnlyAnExistingVersion() async {
        let source = FakeOverview(files: [])
        let viewModel = sheet(.binary, source)

        #expect(viewModel.versionCards.map(\.side) == ["Yours", "Theirs"])
        #expect(viewModel.versionCards[0].size == "2 KB")
        #expect(viewModel.versionCards[1].size == "No file on this side")
        viewModel.take(.theirs)
        await viewModel.idle()
        #expect(source.actions.isEmpty)
        viewModel.take(.ours)
        await viewModel.idle()
        #expect(source.actions == [.takeVersion(path: "Sources/X", version: .ours)])
    }

    /// A project file settles the safe conflicts and opens the editor on
    /// the others, with their reason; references git kept to deleted
    /// objects are named, and removed unless the user unticks it.
    @Test func projectSettlesWhatIsSafeAndLeavesTheRest() async {
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
        await viewModel.idle()
        #expect(source.actions == [.keepBothAdditions(path: "P", removeMissing: false)])
        #expect(edited == 1)

        source.projectInfo = ProjectInfo(oursLabel: "main", theirsLabel: "feature", conflicts: [projectConflict([], [], reason: "x")], resolvable: 0, missing: [], missingRemovable: false)
        let refused = sheet(.projectFile, source)
        #expect(!refused.canKeepBoth)
        refused.keepBoth()
        await refused.idle()
        #expect(source.actions.count == 1)
    }

    /// A catalog is written only once every value both sides changed has
    /// a pick, and the picks go in order; a removed value says so.
    @Test func catalogWritesOnceEveryValueIsChosen() async {
        let source = FakeOverview(files: [])
        let viewModel = sheet(.stringCatalog, source)

        #expect(viewModel.explanation == "3 changes merged by key. 2 values were changed on both sides: choose each one.")
        #expect(viewModel.catalogRows[1].ours == "(removed)")
        #expect(viewModel.catalogRows[1].base == "—")
        viewModel.choose(.theirs, for: 0)
        viewModel.writeChoices()
        await viewModel.idle()
        #expect(source.actions.isEmpty)
        viewModel.choose(.ours, for: 1)
        #expect(viewModel.catalogRows.map(\.pick) == [.theirs, .ours])
        viewModel.writeChoices()
        await viewModel.idle()
        #expect(source.actions == [.resolveCatalog(path: "Sources/X", picks: [.theirs, .ours])])
    }

    /// A Package.resolved lists what merged by itself, from which side,
    /// and is written once each package moved on both sides has a pick.
    @Test func packagesMergeByPackageWithAChoiceLeft() async {
        let source = FakeOverview(files: [])
        let viewModel = sheet(.packageResolved, source)

        #expect(viewModel.explanation == "2 packages taken from the side that moved them. 1 package was moved on both sides: choose each version.")
        #expect(viewModel.pinRows.map(\.change) == ["7.9.0 → 7.10.0", "added 4.4.1"])
        #expect(viewModel.pinRows.map(\.side) == [.theirs, .ours])
        #expect(viewModel.originHashNote != nil)
        #expect(viewModel.writeTitle == "Write Package.resolved")
        #expect(!viewModel.canWriteChoices)
        viewModel.chooseAll(.theirs)
        viewModel.writeChoices()
        await viewModel.idle()
        #expect(source.actions == [.resolvePackages(path: "Sources/X", picks: [.theirs])])
    }

    /// A lockfile is never edited by hand: taking a side keeps the sheet
    /// open on the command to run; a conflicted manifest is named first,
    /// and go.sum can keep both sides' lines.
    @Test func lockfileTakesASideThenShowsTheCommand() async {
        let source = FakeOverview(files: [])
        var closed = 0
        let viewModel = sheet(.lockfile, source) { closed += 1 }

        #expect(!viewModel.canOpenInEditor)
        #expect(viewModel.command == "cd web && npm install")
        #expect(viewModel.manifestWarning == nil)
        #expect(viewModel.lockfileRows.first?.theirs == "—")
        #expect(!viewModel.canMergeLines)
        viewModel.takeLockfile(.theirs)
        await viewModel.idle()
        #expect(source.actions == [.takeLockfile(path: "Sources/X", version: .theirs)])
        #expect(viewModel.taken == .theirs)
        #expect(closed == 0)
        #expect(viewModel.explanation.hasPrefix("The lockfile of feature is written. Run the command below"))
        viewModel.finish()
        #expect(closed == 1)

        source.lockfileInfo = LockfileInfo(
            oursLabel: "main", theirsLabel: "feature", tool: "Go", manifest: "go.mod", manifestConflicted: true,
            command: "go mod tidy", mergesByLines: true, changes: []
        )
        let sum = sheet(.lockfile, source)
        #expect(sum.manifestWarning == "go.mod has conflicts too: resolve it first, since Go writes the lockfile from it.")
        sum.mergeLines()
        await sum.idle()
        #expect(source.actions.last == .mergeLockfileLines(path: "Sources/X"))
    }

    /// A file git did not merge (`-merge`) is settled like a binary: take
    /// one version whole, never staged as it was left.
    @Test func notMergedTakesOneVersion() async {
        let source = FakeOverview(files: [])
        let viewModel = sheet(.notMerged, source)

        #expect(viewModel.subtitle == "Not merged by git")
        #expect(viewModel.explanation.hasPrefix(".gitattributes tells git not to merge this file"))
        #expect(!viewModel.canOpenInEditor)
        viewModel.take(.ours)
        await viewModel.idle()
        #expect(source.actions == [.takeVersion(path: "Sources/X", version: .ours)])
    }

    /// An unusual conflict says what git left and what marking it
    /// resolved stages, then marks it resolved as it is in the folder.
    @Test func unusualIsMarkedResolvedAsInTheFolder() async {
        let source = FakeOverview(files: [])
        var closed = 0
        let viewModel = sheet(.unusual, source) { closed += 1 }

        #expect(viewModel.explanation == "A submodule both sides moved to different commits. Check out the commit you want.")
        #expect(viewModel.folderNote == "It is in the folder: marking it resolved stages it as it is there.")
        viewModel.markResolved()
        await viewModel.idle()
        #expect(source.actions == [.markResolved(path: "Sources/X")])
        #expect(closed == 1)

        source.unusualInfo = UnusualInfo(what: "Both sides renamed it.", advice: "Mark it resolved.", inFolder: false)
        #expect(sheet(.unusual, source).folderNote == "It is not in the folder: marking it resolved stages its removal.")
    }

    /// From the overview, a special file opens its sheet instead of the
    /// editor; once settled, the sheet closes and the list is read again.
    @Test func overviewOpensTheSheetAndRefreshes() async {
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
        await special?.idle()
        #expect(overview.special == nil)
        #expect(source.reads > reads)
        #expect(overview.rows.first?.state == .staged)

        overview.open("A.swift")
        #expect(edited == ["A.swift"])
    }
}
