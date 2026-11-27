import Foundation
import Testing
@testable import Scry

/// A change in memory: the files and their diffs, as the core would give
/// them, and how often each was asked.
final class FakeChanges: ChangesSource, @unchecked Sendable {
    var list: [ChangesFile]
    var diffs: [UInt32: FileDiffInfo] = [:]
    var grouped = true
    var title = "Not committed"
    var singleFile = false
    var hasHistory = true
    var historyFrom: String?
    var failure: ScryError?
    private(set) var listed = 0
    private(set) var asked: [(index: UInt32, allMarks: Bool)] = []

    init(files: [ChangesFile]) {
        list = files
    }

    func header() -> ChangesHeader {
        ChangesHeader(
            repository: "offline-sync", root: "/r", title: title, detail: "Against HEAD",
            sentence: "\(title), against HEAD", branch: "feature/offline-sync",
            oldLabel: "HEAD", newLabel: "Working copy", grouped: grouped, live: true, reduced: false,
            singleFile: singleFile, gitDir: "/r/.git", hasHistory: hasHistory, historyFrom: historyFrom
        )
    }

    private(set) var viewed: [(index: UInt32, viewed: Bool)] = []
    /// The blame each file gets, by its index; none refuses.
    var blames: [UInt32: FakeBlame] = [:]

    func blameSource(index: UInt32) throws -> BlameSource {
        guard let blame = blames[index] else { throw ScryError.Refused(message: "No history.") }
        return blame
    }

    func setViewed(index: UInt32, viewed: Bool) throws {
        if let failure { throw failure }
        self.viewed.append((index, viewed))
    }

    func files() throws -> [ChangesFile] {
        listed += 1
        if let failure { throw failure }
        return list
    }

    func fileDiff(index: UInt32, allMarks: Bool) throws -> FileDiffInfo {
        asked.append((index, allMarks))
        guard let diff = diffs[index] else {
            throw ScryError.Refused(message: "This file is no longer in the list.")
        }
        return diff
    }

    static func file(
        _ index: UInt32, _ path: String, _ state: ChangeState = .modified, group: ChangeGroup? = .notStaged,
        added: UInt32? = 1, removed: UInt32? = 1, oldPath: String? = nil, binary: Bool = false,
        partlyStaged: Bool = false, content: String = "1", viewed: ViewedState = .notViewed
    ) -> ChangesFile {
        let url = URL(fileURLWithPath: path)
        let directory = path.contains("/") ? String(path.dropLast(url.lastPathComponent.count)) : ""
        return ChangesFile(
            index: index, path: path, oldPath: oldPath, name: url.lastPathComponent, directory: directory,
            state: state, binary: binary, submodule: false, added: added, removed: removed, group: group,
            partlyStaged: partlyStaged, contentId: content, viewed: viewed
        )
    }

    /// A text diff of `lines` old lines, all kept, with three lines added
    /// before each of the lines in `addedAt`: the old side is the same
    /// whatever is added, as under a window on the working copy.
    static func added(lines: Int, addedAt: [Int]) -> FileDiffInfo {
        var rows: [DiffRow] = []
        var unified: [UnifiedDiffLine] = []
        var changes: [DiffChange] = []
        var new: UInt32 = 0
        for line in 0..<lines {
            if addedAt.contains(line) {
                changes.append(DiffChange(
                    firstRow: UInt32(rows.count), rowCount: 3, firstLine: UInt32(unified.count), added: 3, removed: 0
                ))
                for _ in 0..<3 {
                    new += 1
                    rows.append(DiffRow(kind: .added, old: nil, new: new, oldWords: [], newWords: []))
                    unified.append(UnifiedDiffLine(kind: .added, old: nil, new: new, words: []))
                }
            }
            new += 1
            rows.append(DiffRow(kind: .unchanged, old: UInt32(line + 1), new: new, oldWords: [], newWords: []))
            unified.append(UnifiedDiffLine(kind: .unchanged, old: UInt32(line + 1), new: new, words: []))
        }
        let line = DiffLine(text: "line", columns: 4, spans: [])
        return .text(diff: TextDiffInfo(
            oldLines: Array(repeating: line, count: lines), newLines: Array(repeating: line, count: Int(new)),
            rows: rows, unified: unified, changes: changes, longestLine: 4,
            added: UInt32(addedAt.count * 3), removed: 0,
            oldMissingFinalNewline: false, newMissingFinalNewline: false, large: false, language: "Swift"
        ))
    }

    /// A text diff of `lines` unchanged lines with a changed block before
    /// each of the lines in `changesAt`, each one old line replaced by two.
    static func text(lines: Int, changesAt: [Int], large: Bool = false) -> FileDiffInfo {
        var rows: [DiffRow] = []
        var unified: [UnifiedDiffLine] = []
        var changes: [DiffChange] = []
        var old: UInt32 = 0
        var new: UInt32 = 0
        for line in 0..<lines {
            if changesAt.contains(line) {
                changes.append(DiffChange(
                    firstRow: UInt32(rows.count), rowCount: 2, firstLine: UInt32(unified.count), added: 2, removed: 1
                ))
                old += 1
                new += 2
                rows.append(DiffRow(kind: .changed, old: old, new: new - 1, oldWords: [], newWords: []))
                rows.append(DiffRow(kind: .added, old: nil, new: new, oldWords: [], newWords: []))
                unified.append(UnifiedDiffLine(kind: .removed, old: old, new: nil, words: []))
                unified.append(UnifiedDiffLine(kind: .added, old: nil, new: new - 1, words: []))
                unified.append(UnifiedDiffLine(kind: .added, old: nil, new: new, words: []))
            }
            old += 1
            new += 1
            rows.append(DiffRow(kind: .unchanged, old: old, new: new, oldWords: [], newWords: []))
            unified.append(UnifiedDiffLine(kind: .unchanged, old: old, new: new, words: []))
        }
        let line = DiffLine(text: "line", columns: 4, spans: [])
        return .text(diff: TextDiffInfo(
            oldLines: Array(repeating: line, count: Int(old)), newLines: Array(repeating: line, count: Int(new)),
            rows: rows, unified: unified, changes: changes, longestLine: 4,
            added: UInt32(changesAt.count * 2), removed: UInt32(changesAt.count),
            oldMissingFinalNewline: false, newMissingFinalNewline: false, large: large, language: "Swift"
        ))
    }
}

@MainActor
struct ChangesViewModelTests {
    private func defaults() -> UserDefaults { .isolated }

    /// Three files in two groups, the first and the last with a text diff.
    private func sample() -> FakeChanges {
        let source = FakeChanges(files: [
            FakeChanges.file(0, "Sources/A.swift", group: .staged, added: 4, removed: 2),
            FakeChanges.file(1, "Sources/B.swift", added: 2, removed: 1),
            FakeChanges.file(2, "icon.png", .untracked, group: .untracked, added: nil, removed: nil, binary: true),
        ])
        source.diffs[0] = FakeChanges.text(lines: 10, changesAt: [2, 6])
        source.diffs[1] = FakeChanges.text(lines: 10, changesAt: [4])
        source.diffs[2] = .binary(oldSize: nil, newSize: 2048)
        return source
    }

    /// The window opens on its first file, with the files in the groups
    /// git status would show and totals that leave binary files out.
    @Test func opensOnTheFirstFileWithItsGroups() async {
        let source = sample()
        let viewModel = ChangesViewModel(source: source, defaults: defaults())
        #expect(viewModel.loadingFiles)

        await viewModel.load()
        #expect(!viewModel.loadingFiles)
        #expect(viewModel.sections.map(\.title) == ["Staged", "Not staged", "Untracked"])
        #expect(viewModel.sections.map(\.files.count) == [1, 1, 1])
        #expect(viewModel.selected?.path == "Sources/A.swift")
        #expect(viewModel.diff != nil)
        #expect(viewModel.listSummary == "3 files · +6 −3")
        #expect(viewModel.scopeDetail == "Against HEAD · branch feature/offline-sync")
        #expect(viewModel.filePosition == "1 of 3")
        #expect(viewModel.status == "2 changes · +4 −2")
        #expect(viewModel.changeCounter == "Change 1 of 2")
    }

    /// A range has no groups: its files are one untitled list. An empty
    /// change, or one that cannot be read, says so instead of a blank
    /// window.
    @Test func showsOneListWithoutGroupsAndSaysWhenThereIsNothing() async {
        let source = sample()
        source.grouped = false
        let viewModel = ChangesViewModel(source: source, defaults: defaults())
        await viewModel.load()
        #expect(viewModel.sections.map(\.title) == [nil])
        #expect(viewModel.sections.first?.files.count == 3)

        source.list = []
        await viewModel.load()
        #expect(viewModel.sections.isEmpty)
        #expect(viewModel.selected == nil)
        #expect(viewModel.content == .none)
        #expect(viewModel.emptyMessage == "Nothing to show. No file differs: not committed, against HEAD.")

        source.failure = .Git(message: "Fatal: bad revision.")
        await viewModel.load()
        #expect(viewModel.listError == "Fatal: bad revision.")
    }

    /// Reading the list again keeps the file being read selected, so a
    /// refresh does not throw the reader back to the first file; a file
    /// that left the list gives way to the first one.
    @Test func keepsTheSelectedFileAcrossAReload() async {
        let source = sample()
        let viewModel = ChangesViewModel(source: source, defaults: defaults())
        await viewModel.load()
        await viewModel.select(viewModel.files[1])

        // The list changed under it: B.swift is now first.
        source.list = [FakeChanges.file(0, "Sources/B.swift"), FakeChanges.file(1, "Sources/C.swift")]
        source.diffs = [0: FakeChanges.text(lines: 6, changesAt: [1]), 1: FakeChanges.text(lines: 6, changesAt: [3])]
        await viewModel.load()
        #expect(viewModel.selected?.path == "Sources/B.swift")
        #expect(viewModel.selected?.index == 0)

        source.list = [FakeChanges.file(0, "Sources/C.swift")]
        await viewModel.load()
        #expect(viewModel.selected?.path == "Sources/C.swift")
    }

    /// Next change walks the file's blocks; past the last one, a first
    /// press only says so and a second goes to the next file, so a key
    /// held a moment too long does not change the file. Going back from a
    /// file's first block lands on the previous file's last one.
    @Test func nextChangeCrossesToTheNextFileOnASecondPress() async {
        let viewModel = ChangesViewModel(source: sample(), defaults: defaults())
        await viewModel.load()
        #expect(!viewModel.canGoToPreviousChange)

        await viewModel.nextChange()
        #expect(viewModel.currentChange == 1)
        #expect(viewModel.atFileEdge == nil)

        await viewModel.nextChange()
        #expect(viewModel.selected?.path == "Sources/A.swift")
        #expect(viewModel.atFileEdge == .last)
        #expect(viewModel.status == "Last change · press again for the next file")

        await viewModel.nextChange()
        #expect(viewModel.selected?.path == "Sources/B.swift")
        #expect(viewModel.currentChange == 0)
        #expect(viewModel.atFileEdge == nil)

        await viewModel.previousChange()
        #expect(viewModel.atFileEdge == .first)
        await viewModel.previousChange()
        #expect(viewModel.selected?.path == "Sources/A.swift")
        #expect(viewModel.currentChange == 1)

        // Moving inside the file forgets the edge.
        await viewModel.nextChange()
        #expect(viewModel.atFileEdge == .last)
        await viewModel.previousChange()
        #expect(viewModel.atFileEdge == nil)
        #expect(viewModel.currentChange == 0)
    }

    /// The same block is at another row in each layout, since a changed
    /// row is two lines in unified: the scroll target, the tinted rows and
    /// the change map follow the layout, which is remembered.
    @Test func aBlockIsFoundInEitherLayout() async {
        let defaults = defaults()
        let viewModel = ChangesViewModel(source: sample(), defaults: defaults)
        await viewModel.load()
        #expect(viewModel.layout == .unified)
        await viewModel.nextChange()

        // Unified: 2 lines, a block of 3, 4 lines, then the second block.
        #expect(viewModel.scrollTarget == 9)
        #expect(viewModel.currentRows == 9..<12)
        #expect(viewModel.rowCount == 16)
        let unified = viewModel.changeMap
        #expect(unified.map(\.current) == [false, true])
        #expect(unified[0].start == 2.0 / 16)
        #expect(unified[0].length == 3.0 / 16)
        #expect(unified[0].removedShare == 1.0 / 3)

        let before = viewModel.scrollRequest
        viewModel.layout = .split
        #expect(viewModel.scrollRequest == before + 1)
        #expect(viewModel.scrollTarget == 8)
        #expect(viewModel.currentRows == 8..<10)
        #expect(viewModel.rowCount == 14)
        #expect(viewModel.changeMap[1].start == 8.0 / 14)

        let reopened = ChangesViewModel(source: sample(), defaults: defaults)
        #expect(reopened.layout == .split)

        viewModel.go(to: 0)
        #expect(viewModel.currentChange == 0)
        viewModel.go(to: 7)
        #expect(viewModel.currentChange == 0)
    }

    /// A file without rows shows what it is: a binary file its sizes, a
    /// file that cannot be read the reason. It is the last file here, so
    /// next change has nowhere to go.
    @Test func aFileWithoutRowsSaysWhatItIs() async {
        let source = sample()
        let viewModel = ChangesViewModel(source: source, defaults: defaults())
        await viewModel.load()
        await viewModel.select(viewModel.files[2])
        #expect(viewModel.content == .binary(old: nil, new: 2048))
        #expect(viewModel.status == "Binary file")
        #expect(viewModel.changeCounter == "")
        #expect(!viewModel.canGoToNextChange)
        #expect(ChangesViewModel.size(nil) == "nothing")

        source.diffs[2] = nil
        await viewModel.select(viewModel.files[2])
        #expect(viewModel.content == .failed("This file is no longer in the list."))
    }

    /// A large file opens without colours and word marks and says so;
    /// Turn On for This File asks the core for them, on the same block.
    @Test func turnsTheMarksOnForALargeFile() async {
        let source = sample()
        source.diffs[0] = FakeChanges.text(lines: 10, changesAt: [2, 6], large: true)
        let viewModel = ChangesViewModel(source: source, defaults: defaults())
        await viewModel.load()
        #expect(viewModel.notice?.hasPrefix("Large file (14 lines)") == true)
        await viewModel.nextChange()

        await viewModel.turnOnMarks()
        #expect(source.asked.last?.allMarks == true)
        #expect(source.asked.last?.index == 0)
        #expect(viewModel.currentChange == 1)
    }

    /// The line under the compare bar says what the colours alone would
    /// not: a new file, a deleted one, a file only renamed.
    @Test func noticesNewDeletedAndRenamedFiles() async {
        let source = FakeChanges(files: [
            FakeChanges.file(0, "New.swift", .untracked, group: .untracked),
            FakeChanges.file(1, "Gone.swift", .deleted),
            FakeChanges.file(2, "Sources/RetryPolicy.swift", .renamed, oldPath: "Sources/Backoff.swift"),
            FakeChanges.file(3, "Plain.swift"),
        ])
        source.diffs = [
            0: FakeChanges.text(lines: 3, changesAt: [0]), 1: FakeChanges.text(lines: 3, changesAt: [0]),
            2: FakeChanges.text(lines: 3, changesAt: []), 3: FakeChanges.text(lines: 3, changesAt: [1]),
        ]
        let viewModel = ChangesViewModel(source: source, defaults: defaults())
        await viewModel.load()
        #expect(viewModel.notice == "New file: every line is added.")
        await viewModel.nextFile()
        #expect(viewModel.notice == "Deleted: every line is removed.")
        await viewModel.nextFile()
        #expect(viewModel.notice == "Renamed from Sources/Backoff.swift. The content is the same.")
        #expect(viewModel.status == "No change in the content")
        #expect(viewModel.changeCounter == "No change")
        await viewModel.nextFile()
        #expect(viewModel.notice == nil)
        #expect(!viewModel.canGoToNextFile)
        await viewModel.nextFile()
        #expect(viewModel.selected?.path == "Plain.swift")
    }

    /// A row's badge is a letter, never colour alone, and its second line
    /// gives the folder and what is unusual: the old name of a renamed
    /// file (its whole path when it moved), binary, partly staged.
    @Test func describesAFileInItsRow() {
        let renamed = FakeChanges.file(0, "Sources/RetryPolicy.swift", .renamed, oldPath: "Sources/Backoff.swift")
        #expect(ChangesViewModel.badge(renamed) == "R")
        #expect(ChangesViewModel.detail(renamed) == "Sources/ · was Backoff.swift")

        let moved = FakeChanges.file(0, "Sources/Sync/Engine.swift", .renamed, oldPath: "Sources/Engine.swift")
        #expect(ChangesViewModel.detail(moved) == "Sources/Sync/ · was Sources/Engine.swift")

        let binary = FakeChanges.file(0, "icon.png", .modified, binary: true)
        #expect(ChangesViewModel.badge(binary) == "B")
        #expect(ChangesViewModel.stateName(binary) == "Binary")
        #expect(ChangesViewModel.detail(binary) == "binary")

        // "partly staged" is a tag of its own in the row.
        let partly = FakeChanges.file(0, "Sources/a.swift", partlyStaged: true)
        #expect(ChangesViewModel.detail(partly) == "Sources/")
        let conflicted = FakeChanges.file(0, "a.swift", .conflicted)
        #expect(ChangesViewModel.badge(conflicted) == "!")
        #expect(ChangesViewModel.detail(conflicted) == "conflicted")
        let states: [(ChangeState, String)] = [
            (.added, "A"), (.modified, "M"), (.deleted, "D"), (.untracked, "U"), (.typeChanged, "T"),
        ]
        for (state, letter) in states {
            #expect(ChangesViewModel.badge(FakeChanges.file(0, "a", state)) == letter)
        }
    }

    /// A conflicted file opens in the merge editor, by its path in the
    /// repository.
    @Test func opensAConflictedFileInTheMergeEditor() async {
        let source = FakeChanges(files: [FakeChanges.file(0, "Sources/A.swift", .conflicted)])
        source.diffs[0] = .conflicted
        var opened: [String] = []
        let viewModel = ChangesViewModel(source: source, defaults: defaults(), openConflicted: { opened.append($0) })
        await viewModel.load()
        #expect(viewModel.content == .conflicted)
        viewModel.openInMergeEditor()
        #expect(opened == ["Sources/A.swift"])
    }

    /// A file with no block to stop on is passed at the first press of
    /// next or previous change, in both directions.
    @Test func nextChangePassesAFileWithoutChanges() async {
        let source = FakeChanges(files: [
            FakeChanges.file(0, "A.swift"),
            FakeChanges.file(1, "icon.png", added: nil, removed: nil, binary: true),
            FakeChanges.file(2, "C.swift"),
        ])
        source.diffs[0] = FakeChanges.text(lines: 10, changesAt: [2, 6])
        source.diffs[1] = .binary(oldSize: 10, newSize: 20)
        source.diffs[2] = FakeChanges.text(lines: 10, changesAt: [4])
        let viewModel = ChangesViewModel(source: source, defaults: defaults())
        await viewModel.load()
        await viewModel.select(viewModel.files[1])
        #expect(viewModel.canGoToNextChange)
        #expect(viewModel.canGoToPreviousChange)

        await viewModel.nextChange()
        #expect(viewModel.selected?.path == "C.swift")

        await viewModel.select(viewModel.files[1])
        await viewModel.previousChange()
        #expect(viewModel.selected?.path == "A.swift")
        // Arriving from below: on the file's last block.
        #expect(viewModel.currentChange == 1)

        // The last file of the list, without changes: nowhere to go.
        source.list = [FakeChanges.file(1, "icon.png", added: nil, removed: nil, binary: true)]
        await viewModel.load()
        #expect(!viewModel.canGoToNextChange)
        #expect(!viewModel.canGoToPreviousChange)
        await viewModel.nextChange()
        #expect(viewModel.selected?.path == "icon.png")
    }

    /// A group's title hides its files and shows them again; a file
    /// reached from the keyboard opens its group.
    @Test func collapsesAGroup() async {
        let viewModel = ChangesViewModel(source: sample(), defaults: defaults())
        await viewModel.load()
        #expect(viewModel.sections.map(\.collapsed) == [false, false, false])
        #expect(viewModel.sections.map(\.group) == [.staged, .notStaged, .untracked])

        viewModel.toggle(.notStaged)
        #expect(viewModel.sections.map(\.collapsed) == [false, true, false])
        // The files are still counted, and the selected one still shown.
        #expect(viewModel.sections[1].files.count == 1)
        #expect(viewModel.selected?.path == "Sources/A.swift")

        await viewModel.nextFile()
        #expect(viewModel.selected?.path == "Sources/B.swift")
        #expect(viewModel.sections.map(\.collapsed) == [false, false, false])

        viewModel.toggle(.untracked)
        viewModel.toggle(.untracked)
        #expect(viewModel.collapsed.isEmpty)
    }

    /// Under git difftool the window knows its place among git's files:
    /// Next File closes it for the next one, Stop for none.
    @Test func leavesGitDifftoolsQueue() async {
        var left: [Bool] = []
        let viewModel = ChangesViewModel(
            source: sample(), defaults: defaults(),
            queue: ChangesViewModel.Queue(place: 2, total: 13), leaveQueue: { left.append($0) }
        )
        #expect(viewModel.queueLabel == "File 2 of 13")
        #expect(viewModel.queue?.isLast == false)
        viewModel.nextInQueue()
        viewModel.stopQueue()
        #expect(left == [false, true])

        #expect(ChangesViewModel.Queue(place: 13, total: 13).isLast)
        #expect(ChangesViewModel(source: sample(), defaults: defaults()).queueLabel == nil)
    }

    /// While ⌘ is held the window names the shortcuts that work now.
    @Test func hintsTheShortcutsThatWorkNow() async {
        let viewModel = ChangesViewModel(source: sample(), defaults: defaults())
        await viewModel.load()
        #expect(viewModel.toolbarHints == [.nextChange, .changesLayout, .blame, .refreshChanges])
        #expect(viewModel.hintStrip == [.nextFile, .viewed, .openRepository, .keyboardShortcuts])

        await viewModel.select(viewModel.files[2])
        // The last file has no text here: nothing to blame.
        #expect(viewModel.toolbarHints == [.previousChange, .changesLayout, .refreshChanges])
        #expect(viewModel.hintStrip == [.previousFile, .viewed, .openRepository, .keyboardShortcuts])
    }

    // MARK: Live refresh

    /// After a change on disk the list is read again: the selected file
    /// stays selected, a file that changed and one that arrived get a dot
    /// until they are shown, and nothing happens when nothing differs.
    @Test func refreshKeepsTheSelectionAndMarksWhatArrived() async {
        let source = sample()
        let moment = Date(timeIntervalSince1970: 1000)
        let viewModel = ChangesViewModel(source: source, defaults: defaults(), now: { moment })
        await viewModel.load()
        let scrolls = viewModel.scrollRequest

        await viewModel.refresh()
        #expect(viewModel.updatedAt == nil)
        #expect(viewModel.arrived.isEmpty)

        source.list[1] = FakeChanges.file(1, "Sources/B.swift", added: 5, removed: 1, content: "2")
        source.list.append(FakeChanges.file(3, "new.md", .untracked, group: .untracked, added: 3, removed: 0))
        source.diffs[3] = FakeChanges.text(lines: 3, changesAt: [])
        await viewModel.refresh()
        #expect(viewModel.files.count == 4)
        #expect(viewModel.selected?.path == "Sources/A.swift")
        #expect(viewModel.arrived == ["Sources/B.swift": .changed, "new.md": .new])
        #expect(viewModel.updatedAt == moment)
        // The open file did not change: it is not read again, nor moved.
        #expect(source.asked.count == 1)
        #expect(viewModel.scrollRequest == scrolls)
        #expect(viewModel.placeRequest == 0)
        #expect(ChangesViewModel.detail(viewModel.files[3], arrival: .new) == "new")

        // A file that arrived and changes again is still new.
        source.list[3] = FakeChanges.file(3, "new.md", .untracked, group: .untracked, added: 4, removed: 0, content: "2")
        await viewModel.refresh()
        #expect(viewModel.arrived["new.md"] == .new)

        await viewModel.select(viewModel.files[1])
        #expect(viewModel.arrived == ["new.md": .new])

        // Read Again starts from what is there now.
        await viewModel.load()
        #expect(viewModel.arrived.isEmpty)
    }

    /// When the open file changes on disk it is read again in place: no
    /// loading state, no jump to a block, and the line at the top of the
    /// view stays at the top though lines arrived above it.
    @Test func refreshReadsTheOpenFileAgainWithoutMovingIt() async {
        let source = sample()
        source.diffs[0] = FakeChanges.added(lines: 20, addedAt: [6, 15])
        let viewModel = ChangesViewModel(source: source, defaults: defaults())
        await viewModel.load()
        await viewModel.nextChange()
        #expect(viewModel.currentChange == 1)
        let scrolls = viewModel.scrollRequest
        // The last added line of the first block, half scrolled away.
        viewModel.topRow = 8.5

        source.list[0] = FakeChanges.file(0, "Sources/A.swift", group: .staged, added: 6, removed: 3, content: "2")
        source.diffs[0] = FakeChanges.added(lines: 20, addedAt: [2, 6, 15])
        await viewModel.refresh()
        #expect(viewModel.selected?.contentId == "2")
        #expect(viewModel.changeCount == 3)
        #expect(viewModel.currentChange == 1)
        #expect(viewModel.scrollRequest == scrolls)
        // Three rows arrived above: the same line is three rows lower.
        #expect(viewModel.placeRequest == 1)
        #expect(viewModel.placeTarget == 11.5)

        // A change under the view moves nothing.
        viewModel.topRow = 1
        source.list[0] = FakeChanges.file(0, "Sources/A.swift", group: .staged, added: 4, removed: 2, content: "3")
        source.diffs[0] = FakeChanges.added(lines: 20, addedAt: [2, 6])
        await viewModel.refresh()
        #expect(viewModel.placeRequest == 1)
        // The block the user was on is gone: the last one left.
        #expect(viewModel.currentChange == 1)
    }

    /// A file that no longer differs leaves the list: the one that took
    /// its place is shown, or nothing when the list is empty.
    @Test func refreshLeavesAFileThatNoLongerDiffers() async {
        let source = sample()
        let viewModel = ChangesViewModel(source: source, defaults: defaults())
        await viewModel.load()
        await viewModel.select(viewModel.files[1])

        source.list = [source.list[0], FakeChanges.file(1, "icon.png", .untracked, group: .untracked, binary: true)]
        source.diffs[1] = .binary(oldSize: nil, newSize: 2048)
        await viewModel.refresh()
        #expect(viewModel.selected?.path == "icon.png")
        #expect(viewModel.content == .binary(old: nil, new: 2048))

        source.list = []
        await viewModel.refresh()
        #expect(viewModel.selected == nil)
        #expect(viewModel.content == .none)

        // A list that cannot be read leaves what is shown.
        source.failure = .Failed(message: "git stopped")
        await viewModel.refresh()
        #expect(viewModel.listError == nil)
    }

    /// The top row is followed by its old line, in either layout; a view
    /// in a file with no old side, or past its end, does not move.
    @Test func placesTheTopRowByItsOldLine() {
        guard case .text(let before) = FakeChanges.added(lines: 10, addedAt: [6]),
              case .text(let after) = FakeChanges.added(lines: 10, addedAt: [2, 6])
        else { return }
        #expect(ChangesViewModel.place(of: 0, from: before, to: after, layout: .unified) == 0)
        #expect(ChangesViewModel.place(of: 4.25, from: before, to: after, layout: .unified) == 7.25)
        // On an added line: the old line under it is followed.
        #expect(ChangesViewModel.place(of: 7, from: before, to: after, layout: .split) == 10)
        #expect(ChangesViewModel.place(of: 7, from: after, to: before, layout: .split) == 4)
        // Lines the view was on are gone: the top goes to what follows.
        #expect(ChangesViewModel.place(of: 2, from: after, to: before, layout: .unified) == 0)
        #expect(ChangesViewModel.place(of: 99, from: before, to: after, layout: .unified) == nil)
        #expect(ChangesViewModel.place(of: -1, from: before, to: after, layout: .unified) == nil)
    }

    /// The window listens to the repository only while it shows the
    /// working copy or the index, and refreshes at each change.
    @Test func watchRefreshesAtEachChange() async {
        let source = sample()
        let (stream, changes) = AsyncStream.makeStream(of: Void.self)
        let viewModel = ChangesViewModel(source: source, defaults: defaults(), watch: { _ in stream })
        await viewModel.load()
        source.list[1] = FakeChanges.file(1, "Sources/B.swift", content: "2")
        changes.yield()
        changes.finish()
        await viewModel.watch()
        #expect(source.listed == 2)
        #expect(viewModel.arrived == ["Sources/B.swift": .changed])
    }

    // MARK: Viewed

    /// A file is marked viewed from its box or from the key, which then
    /// goes to the next file that is not viewed, from the top after the
    /// last one; on a viewed file the key removes the mark.
    @Test func marksFilesViewed() async {
        let source = sample()
        let viewModel = ChangesViewModel(source: source, defaults: defaults())
        await viewModel.load()
        #expect(viewModel.canMarkViewed)
        #expect(viewModel.viewedSummary == "0 of 3 viewed")

        viewModel.toggleViewed(viewModel.files[1])
        #expect(viewModel.files[1].viewed == .viewed)
        #expect(source.viewed.map(\.index) == [1])
        #expect(viewModel.viewedSummary == "1 of 3 viewed")

        // From the first file, past the viewed one.
        await viewModel.markViewedAndGoOn()
        #expect(viewModel.files[0].viewed == .viewed)
        #expect(viewModel.selected?.path == "icon.png")
        #expect(viewModel.viewedShare == 2.0 / 3.0)

        // The last one left: the selection stays.
        await viewModel.markViewedAndGoOn()
        #expect(viewModel.viewedCount == 3)
        #expect(viewModel.selected?.path == "icon.png")
        #expect(viewModel.selected?.viewed == .viewed)

        await viewModel.markViewedAndGoOn()
        #expect(viewModel.selected?.viewed == .notViewed)
        #expect(source.viewed.last?.viewed == false)

        // From the last file, the next one not viewed is before it.
        viewModel.toggleViewed(viewModel.files[0])
        await viewModel.markViewedAndGoOn()
        #expect(viewModel.selected?.path == "Sources/A.swift")
    }

    /// A file changed since it was viewed says so in its row; a mark that
    /// cannot be written is reported and changes nothing.
    @Test func saysWhatChangedSinceViewedAndWhatFailed() async {
        let source = FakeChanges(files: [
            FakeChanges.file(0, "Sources/A.swift", viewed: .changedSince),
            FakeChanges.file(1, "Sources/B.swift", viewed: .viewed),
        ])
        source.diffs[0] = FakeChanges.text(lines: 4, changesAt: [1])
        let viewModel = ChangesViewModel(source: source, defaults: defaults())
        await viewModel.load()
        #expect(ChangesViewModel.detail(viewModel.files[0]) == "changed since viewed")
        #expect(viewModel.viewedCount == 1)

        source.failure = .Failed(message: "The disk is full.")
        viewModel.toggleViewed(viewModel.files[0])
        #expect(viewModel.files[0].viewed == .changedSince)
        #expect(viewModel.status == "The disk is full.")
        source.failure = nil
        viewModel.toggleViewed(viewModel.files[0])
        #expect(viewModel.files[0].viewed == .viewed)
        #expect(viewModel.status == "1 change · +2 −1")
    }
}

struct RepositoryWatcherTests {
    /// Anything in the working folder matters; in the git folder only the
    /// index, HEAD and the references, never a lock file or Scry's notes.
    @Test func tellsWhatCanChangeADiff() {
        let matters = { RepositoryWatcher.matters($0, root: "/r", gitDir: "/r/.git") }
        #expect(matters("/r/src/a.swift"))
        #expect(matters("/r/.git/index"))
        #expect(matters("/r/.git/HEAD"))
        #expect(matters("/r/.git/MERGE_HEAD"))
        #expect(matters("/r/.git/refs/heads/main"))
        #expect(matters("/r/.git/packed-refs"))
        #expect(!matters("/r/.git/index.lock"))
        #expect(!matters("/r/.git/scry/viewed.json"))
        #expect(!matters("/r/.git/objects/ab/cdef"))
        #expect(!matters("/other/a.swift"))
        #expect(!matters("/r2/a.swift"))
        // A linked worktree's git folder is elsewhere.
        #expect(RepositoryWatcher.matters("/main/.git/worktrees/w/index", root: "/w", gitDir: "/main/.git/worktrees/w"))
        #expect(RepositoryWatcher.matters("/w/a.swift", root: "/w", gitDir: "/main/.git/worktrees/w"))
    }

    /// A file written in the folder is told, once the writes settle.
    @Test func tellsAWriteInTheFolder() async throws {
        let folder = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: folder.appendingPathComponent(".git"), withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: folder) }
        let changes = RepositoryWatcher.changes(root: folder.path, gitDir: folder.appendingPathComponent(".git").path)
        let told = await withTaskGroup(of: Bool.self) { group in
            group.addTask {
                for await _ in changes { return true }
                return false
            }
            group.addTask {
                // The stream starts a moment after it is made.
                for _ in 0..<40 {
                    try? await Task.sleep(for: .milliseconds(250))
                    try? Data("x".utf8).write(to: folder.appendingPathComponent("a.txt"))
                }
                return false
            }
            let first = await group.next() ?? false
            group.cancelAll()
            return first
        }
        #expect(told)
    }
}

@MainActor
struct DiffMetricsTests {
    /// Unified has two number columns and one code column as wide as the
    /// file area; split has one number column per side and half the width
    /// each, so a line can scroll further sideways there.
    @Test func columnsFollowTheLayout() {
        let unified = DiffMetrics(layout: .unified, width: 1000, digits: 3)
        let split = DiffMetrics(layout: .split, width: 1000, digits: 3)
        #expect(unified.gutter == 2 * unified.number + DiffMetrics.sign)
        #expect(split.gutter == split.number + DiffMetrics.sign)
        #expect(unified.code == 1000 - unified.gutter)
        #expect(split.pane == 499.5)
        #expect(split.code == 499.5 - split.gutter)
        #expect(unified.maximumScroll(columns: 10) == 0)
        #expect(split.maximumScroll(columns: 200) > unified.maximumScroll(columns: 200))
        // More digits widen the gutter.
        #expect(DiffMetrics(layout: .unified, width: 1000, digits: 6).number > unified.number)
        #expect(DiffMetrics(layout: .split, width: -5, digits: 3).code == 0)
    }
}
