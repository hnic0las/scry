import Foundation
import Observation

/// S4: settles a conflicted file the merge editor cannot, or one a view
/// settles better: a file deleted on one side (keep or delete), a binary
/// file (take a version), an Xcode project (keep both sides' additions)
/// or a string catalog (merged key by key, a choice per value both sides
/// changed).
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

    /// A string catalog value both sides changed.
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
    private(set) var picks: [FileVersion?] = []
    /// Also remove the list entries of objects no longer defined, which
    /// git kept without a conflict.
    var removeMissing = true

    @ObservationIgnored private let source: OverviewSource
    @ObservationIgnored private let done: () -> Void
    @ObservationIgnored private let openInEditor: (OverviewFile) -> Void

    /// `done` runs once the file is settled; `openInEditor` hands a text
    /// file to the merge editor instead.
    init(
        file: OverviewFile,
        source: OverviewSource,
        done: @escaping () -> Void,
        openInEditor: @escaping (OverviewFile) -> Void
    ) {
        self.file = file
        self.source = source
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

    var oursLabel: String { project?.oursLabel ?? catalog?.oursLabel ?? binary?.ours.label ?? "Yours" }
    var theirsLabel: String { project?.theirsLabel ?? catalog?.theirsLabel ?? binary?.theirs.label ?? "Theirs" }

    /// Every value both sides changed has a pick.
    var canWriteCatalog: Bool { catalog != nil && picks.allSatisfy { $0 != nil } }

    /// A project file or catalog can also go to the merge editor.
    var canOpenInEditor: Bool { file.openable }

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

    func writeCatalog() {
        guard canWriteCatalog else { return }
        run(.resolveCatalog(path: file.path, picks: picks.compactMap { $0 }))
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
            case .binary: binary = try source.binary(path: file.path)
            case .projectFile: project = try source.project(path: file.path)
            case .stringCatalog:
                let catalog = try source.catalog(path: file.path)
                self.catalog = catalog
                picks = Array(repeating: nil, count: catalog.conflicts.count)
            }
        } catch {
            self.error = "\(error)"
        }
    }

    private func run(_ action: OverviewAction, then next: (() -> Void)? = nil) {
        do {
            try source.apply(action: action)
            error = nil
            (next ?? done)()
        } catch {
            self.error = "\(error)"
        }
    }
}

/// One sheet per file: its identity is the view model's own.
extension SpecialFileViewModel: Identifiable {}
