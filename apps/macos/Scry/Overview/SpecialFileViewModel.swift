import Foundation
import Observation

/// S4: settles a conflicted file the merge editor cannot, or one a view
/// settles better: a file deleted on one side (keep or delete), a binary
/// file (take a version), an Xcode project (keep both sides' additions),
/// a string catalog or a `Package.resolved` (merged key by key or package
/// by package, a choice per value both sides changed), or a lockfile
/// (take a side, then let the package manager write it again).
@MainActor
@Observable
final class SpecialFileViewModel {
    /// A version of a binary file, as a card shows it.
    struct VersionCard: Identifiable, Equatable {
        let version: FileVersion
        /// "Yours" or "Theirs".
        let side: String
        let label: String
        /// "12 KB", or "No file on this side".
        let size: String
        /// The content, for an image preview.
        let data: Data?
        let available: Bool
        var id: String { side }
    }

    /// A line of the changes a deletion would throw away.
    struct DiffLine: Identifiable, Equatable {
        enum Kind: Equatable {
            case hunk
            case added
            case removed
            case context
        }

        let id: Int
        let kind: Kind
        /// The line without its `+`, `-` or space.
        let text: String
    }

    /// A conflict of an Xcode project file.
    struct ProjectRow: Identifiable, Equatable {
        let id: Int
        let oursAdds: [String]
        let theirsAdds: [String]
        let oursRemoves: [String]
        let theirsRemoves: [String]
        /// "MARKETING_VERSION 1.3 → 1.4".
        let oursChanges: [String]
        let theirsChanges: [String]
        let keepsBoth: Bool
        /// Why it stays for the merge editor.
        let reason: String?
    }

    /// A Swift package one side alone moved: "kingfisher 7.9.0 → 7.10.0".
    struct PinRow: Identifiable, Equatable {
        let id: Int
        let identity: String
        let change: String
        /// The side it was taken from.
        let side: FileVersion
    }

    /// A package of a lockfile: its version in each.
    struct PackageRow: Identifiable, Equatable {
        let id: Int
        let name: String
        let base: String
        let ours: String
        let theirs: String
        /// Both sides moved it, differently: the side taken keeps its
        /// version, unless the merged manifest rules it out.
        let bothChanged: Bool
    }

    /// A string catalog value, or a Swift package, both sides changed.
    struct CatalogRow: Identifiable, Equatable {
        let id: Int
        /// "sync.done", or the path when it is not a string.
        let key: String
        /// "fr › value".
        let detail: String
        let base: String
        let ours: String
        let theirs: String
        let pick: FileVersion?
    }

    let file: OverviewFile
    private(set) var error: String?

    private(set) var deletion: DeletionInfo?
    private(set) var binary: BinaryInfo?
    private(set) var project: ProjectInfo?
    private(set) var catalog: CatalogInfo?
    private(set) var packages: PackagesInfo?
    private(set) var lockfile: LockfileInfo?
    private(set) var unusual: UnusualInfo?
    /// The side of the lockfile written, waiting for the command.
    private(set) var taken: FileVersion?
    private(set) var picks: [FileVersion?] = []
    /// Also remove the list entries of objects no longer defined, which
    /// git kept without a conflict.
    var removeMissing = true

    @ObservationIgnored private let source: OverviewSource
    /// Git is writing or staging the file: the buttons wait for it.
    private(set) var isRunning = false

    @ObservationIgnored private let changed: (OverviewSnapshot) -> Void
    @ObservationIgnored private let done: () -> Void
    @ObservationIgnored private var work: Task<Void, Never>?
    @ObservationIgnored private let openInEditor: (OverviewFile) -> Void

    /// `changed` gets the overview as each action leaves it; `done` runs
    /// once the file is settled; `openInEditor` hands a text file to the
    /// merge editor instead.
    init(
        file: OverviewFile,
        source: OverviewSource,
        changed: @escaping (OverviewSnapshot) -> Void = { _ in },
        done: @escaping () -> Void,
        openInEditor: @escaping (OverviewFile) -> Void
    ) {
        self.file = file
        self.source = source
        self.changed = changed
        self.done = done
        self.openInEditor = openInEditor
        load()
    }

    var kind: SpecialKind { file.special ?? .binary }

    // MARK: Header

    var title: String { file.name }

    var subtitle: String {
        switch kind {
        case .deletedOnOneSide: "Deleted on one side, changed on the other"
        case .binary: "Binary file: no text to merge"
        case .projectFile: "Xcode project file"
        case .stringCatalog: "String catalog"
        case .packageResolved: "Swift package pins"
        case .lockfile: lockfile.map { "\($0.tool) lockfile, written by \($0.tool) from \($0.manifest)" } ?? "Lockfile"
        case .notMerged: "Not merged by git"
        case .unusual: file.kind
        }
    }

    /// What the user decides here, in a sentence.
    var explanation: String {
        switch kind {
        case .deletedOnOneSide:
            guard let deletion else { return "" }
            let lines = deletion.keptLines.map { " (\($0) \($0 == 1 ? "line" : "lines"))" } ?? ""
            return "\(deletion.deletedLabel) deleted it; \(deletion.keptLabel) changed it\(lines). Keep the changed file, or delete it."
        case .binary:
            return "Git cannot merge binary files. Take one version whole."
        case .notMerged:
            return ".gitattributes tells git not to merge this file, so it left your version without markers. Take one version whole."
        case .unusual:
            guard let unusual else { return "" }
            return "\(unusual.what) \(unusual.advice)"
        case .projectFile:
            guard let project else { return "" }
            let total = project.conflicts.count
            let settled = Int(project.resolvable)
            if settled == total {
                return "Every conflict settles by keeping both sides' additions and removals."
            }
            if settled == 0 {
                return "No conflict can keep both sides here: resolve this file in the merge editor."
            }
            return "\(settled) of \(total) conflicts settle by keeping both sides' changes; the merge editor opens on the \(total - settled) left."
        case .stringCatalog:
            guard let catalog else { return "" }
            let automatic = "\(catalog.automatic) \(catalog.automatic == 1 ? "change" : "changes") merged by key"
            let left = catalog.conflicts.count
            return left == 0
                ? "\(automatic); nothing left to choose."
                : "\(automatic). \(left) \(left == 1 ? "value was" : "values were") changed on both sides: choose each one."
        case .packageResolved:
            guard let packages else { return "" }
            let count = packages.automatic.count
            let automatic = count == 1 ? "1 package taken from the side that moved it" : "\(count) packages taken from the side that moved them"
            let left = packages.conflicts.count
            return left == 0
                ? "\(automatic); nothing left to choose."
                : "\(automatic). \(left) \(left == 1 ? "package was" : "packages were") moved on both sides: choose each version."
        case .lockfile:
            guard let lockfile else { return "" }
            if let taken {
                let side = taken == .ours ? lockfile.oursLabel : lockfile.theirsLabel
                return "The lockfile of \(side) is written. Run the command below, so \(lockfile.tool) adds the other side's changes from \(lockfile.manifest), then stage it from the overview."
            }
            let lines = lockfile.mergesByLines ? ", or keep both sides' lines" : ""
            return "\(lockfile.tool) writes this file: don't merge it by hand. Take one side\(lines), then run the command below to bring in the other side's changes."
        }
    }

    // MARK: Deleted on one side

    /// "What main · being merged changed".
    var changesTitle: String {
        deletion.map { "What \($0.keptLabel) changed, lost if you delete the file" } ?? ""
    }

    /// The changes the side that kept the file made since the base.
    var changeLines: [DiffLine] {
        guard let deletion else { return [] }
        return deletion.changes.split(separator: "\n", omittingEmptySubsequences: false)
            .dropLast(deletion.changes.hasSuffix("\n") ? 1 : 0)
            .enumerated()
            .map { index, line in
                let kind: DiffLine.Kind = line.hasPrefix("@@") ? .hunk
                    : line.hasPrefix("+") ? .added
                    : line.hasPrefix("-") ? .removed : .context
                return DiffLine(id: index, kind: kind, text: kind == .hunk ? String(line) : String(line.dropFirst()))
            }
    }

    // MARK: Binary

    var versionCards: [VersionCard] {
        guard let binary else { return [] }
        return [(FileVersion.ours, "Yours", binary.ours), (.theirs, "Theirs", binary.theirs)].map { version, side, info in
            VersionCard(
                version: version,
                side: side,
                label: info.label,
                size: info.size.map { ByteCountFormatter.string(fromByteCount: Int64($0), countStyle: .file) } ?? "No file on this side",
                data: info.data,
                available: info.size != nil
            )
        }
    }

    // MARK: Project file

    var projectRows: [ProjectRow] {
        project?.conflicts.enumerated().map { index, conflict in
            ProjectRow(
                id: index,
                oursAdds: conflict.oursAdds,
                theirsAdds: conflict.theirsAdds,
                oursRemoves: conflict.oursRemoves,
                theirsRemoves: conflict.theirsRemoves,
                oursChanges: conflict.oursChanges,
                theirsChanges: conflict.theirsChanges,
                keepsBoth: conflict.keepsBoth,
                reason: conflict.reason
            )
        } ?? []
    }

    /// Objects the project refers to without defining them, which git
    /// merged without a conflict: a file one side deleted, still listed
    /// by the other.
    var projectWarning: String? {
        guard let project, !project.missing.isEmpty else { return nil }
        let names = project.missing.joined(separator: ", ")
        return "Git merged references to objects that no longer exist: \(names). Xcode would show them as missing files."
    }

    /// Whether those references are only list entries, which can go.
    var canRemoveMissing: Bool { project?.missingRemovable ?? false }

    var canKeepBoth: Bool { (project?.resolvable ?? 0) > 0 }

    /// "Keep Both Sides", or "Settle 3 of 4 Conflicts" when some are left.
    var keepBothTitle: String {
        guard let project else { return "Keep Both Sides" }
        let total = project.conflicts.count
        return Int(project.resolvable) == total ? "Keep Both Sides" : "Settle \(project.resolvable) of \(total) Conflicts"
    }

    // MARK: String catalog

    var catalogRows: [CatalogRow] {
        guard let catalog else { return [] }
        return catalog.conflicts.enumerated().map { index, conflict in
            CatalogRow(
                id: index,
                key: conflict.key ?? conflict.detail,
                detail: conflict.key == nil ? "" : conflict.detail,
                base: conflict.base ?? "—",
                ours: conflict.ours ?? "(removed)",
                theirs: conflict.theirs ?? "(removed)",
                pick: picks.indices.contains(index) ? picks[index] : nil
            )
        }
    }

    // MARK: Package.resolved

    /// The packages one side alone moved, taken from it.
    var pinRows: [PinRow] {
        guard let packages else { return [] }
        return packages.automatic.enumerated().map { index, pin in
            let side: FileVersion = pin.ours != pin.base ? .ours : .theirs
            let now = (side == .ours ? pin.ours : pin.theirs) ?? "removed"
            let change = pin.base.map { "\($0) → \(now)" } ?? "added \(now)"
            return PinRow(id: index, identity: pin.identity, change: change, side: side)
        }
    }

    /// The packages both sides moved, one pick each.
    var packageRows: [CatalogRow] {
        guard let packages else { return [] }
        return packages.conflicts.enumerated().map { index, pin in
            CatalogRow(
                id: index,
                key: pin.identity,
                detail: "",
                base: pin.base ?? "—",
                ours: pin.ours ?? "(removed)",
                theirs: pin.theirs ?? "(removed)",
                pick: picks.indices.contains(index) ? picks[index] : nil
            )
        }
    }

    var originHashNote: String? {
        packages?.originHashStale == true
            ? "Both sides changed originHash, the fingerprint of the package dependencies: yours is kept, and Xcode updates it at the next package resolution."
            : nil
    }

    // MARK: Lockfile

    var lockfileRows: [PackageRow] {
        guard let lockfile else { return [] }
        return lockfile.changes.enumerated().map { index, change in
            PackageRow(
                id: index,
                name: change.name,
                base: change.base ?? "—",
                ours: change.ours ?? "—",
                theirs: change.theirs ?? "—",
                bothChanged: change.bothChanged
            )
        }
    }

    /// The manifest has conflicts too: the command writes the lockfile
    /// from it, so it comes first.
    var manifestWarning: String? {
        guard let lockfile, lockfile.manifestConflicted else { return nil }
        return "\(lockfile.manifest) has conflicts too: resolve it first, since \(lockfile.tool) writes the lockfile from it."
    }

    /// "cd ios && pod install".
    var command: String { lockfile?.command ?? "" }

    var canMergeLines: Bool { lockfile?.mergesByLines ?? false }

    // MARK: Unusual

    /// What marking it resolved stages.
    var folderNote: String? {
        guard let unusual else { return nil }
        return unusual.inFolder
            ? "It is in the folder: marking it resolved stages it as it is there."
            : "It is not in the folder: marking it resolved stages its removal."
    }

    var canMarkResolved: Bool { unusual != nil }

    // MARK: Labels

    var oursLabel: String {
        project?.oursLabel ?? catalog?.oursLabel ?? packages?.oursLabel ?? lockfile?.oursLabel ?? binary?.ours.label ?? "Yours"
    }

    var theirsLabel: String {
        project?.theirsLabel ?? catalog?.theirsLabel ?? packages?.theirsLabel ?? lockfile?.theirsLabel ?? binary?.theirs.label ?? "Theirs"
    }

    /// A catalog or `Package.resolved` with a pick for every value both
    /// sides changed.
    var canWriteChoices: Bool { (catalog != nil || packages != nil) && picks.allSatisfy { $0 != nil } }

    /// "Write Catalog" or "Write Package.resolved".
    var writeTitle: String { kind == .packageResolved ? "Write Package.resolved" : "Write Catalog" }

    /// A project file, catalog or `Package.resolved` can also go to the
    /// merge editor; a lockfile is not edited by hand.
    var canOpenInEditor: Bool { file.openable && kind != .lockfile }

    // MARK: Intents

    func keepFile() {
        run(.keepFile(path: file.path))
    }

    func deleteFile() {
        run(.deleteFile(path: file.path))
    }

    func take(_ version: FileVersion) {
        guard versionCards.first(where: { $0.version == version })?.available == true else { return }
        run(.takeVersion(path: file.path, version: version))
    }

    /// Settles the safe conflicts; when some are left, the merge editor
    /// opens on them.
    func keepBoth() {
        guard canKeepBoth, let project else { return }
        let left = Int(project.resolvable) < project.conflicts.count
        run(.keepBothAdditions(path: file.path, removeMissing: removeMissing && canRemoveMissing)) { [file, done, openInEditor] in
            if left { openInEditor(file) } else { done() }
        }
    }

    func choose(_ version: FileVersion, for row: Int) {
        guard picks.indices.contains(row) else { return }
        picks[row] = version
    }

    /// Takes one side for every value left.
    func chooseAll(_ version: FileVersion) {
        picks = picks.map { _ in version }
    }

    /// Writes the catalog or `Package.resolved` with the picks.
    func writeChoices() {
        guard canWriteChoices else { return }
        let chosen = picks.compactMap { $0 }
        run(kind == .packageResolved
            ? .resolvePackages(path: file.path, picks: chosen)
            : .resolveCatalog(path: file.path, picks: chosen))
    }

    /// Writes one side of the lockfile, unstaged; the sheet stays open on
    /// the command to run.
    func takeLockfile(_ version: FileVersion) {
        guard lockfile != nil else { return }
        run(.takeLockfile(path: file.path, version: version)) { [weak self] in
            self?.taken = version
        }
    }

    /// Keeps both sides' lines (`go.sum`).
    func mergeLines() {
        guard canMergeLines else { return }
        run(.mergeLockfileLines(path: file.path))
    }

    /// Stages the file as it is in the folder, or its removal.
    func markResolved() {
        guard canMarkResolved else { return }
        run(.markResolved(path: file.path))
    }

    /// Closes the sheet once the lockfile is taken.
    func finish() {
        done()
    }

    func openEditor() {
        guard canOpenInEditor else { return }
        openInEditor(file)
    }

    // MARK: Private

    private func load() {
        do {
            switch kind {
            case .deletedOnOneSide: deletion = try source.deletion(path: file.path)
            case .binary, .notMerged: binary = try source.binary(path: file.path)
            case .unusual: unusual = try source.unusual(path: file.path)
            case .projectFile: project = try source.project(path: file.path)
            case .stringCatalog:
                let catalog = try source.catalog(path: file.path)
                self.catalog = catalog
                picks = Array(repeating: nil, count: catalog.conflicts.count)
            case .packageResolved:
                let packages = try source.packages(path: file.path)
                self.packages = packages
                picks = Array(repeating: nil, count: packages.conflicts.count)
            case .lockfile: lockfile = try source.lockfile(path: file.path)
            }
        } catch {
            self.error = "\(error)"
        }
    }

    /// Runs the action off the main actor, one at a time; on success the
    /// overview gets the snapshot the core read after it.
    private func run(_ action: OverviewAction, then next: (() -> Void)? = nil) {
        guard !isRunning else { return }
        isRunning = true
        let source = source
        work = Task { [weak self] in
            let result = await Task.detached { Result { try source.apply(action: action) } }.value
            guard let self else { return }
            isRunning = false
            switch result {
            case .success(let snapshot):
                error = nil
                changed(snapshot)
                (next ?? done)()
            case .failure(let failure):
                error = "\(failure)"
            }
        }
    }

    /// Waits for the action in flight, if any.
    func idle() async {
        await work?.value
    }
}

/// One sheet per file: its identity is the view model's own.
extension SpecialFileViewModel: Identifiable {}
