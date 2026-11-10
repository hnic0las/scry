import Foundation

/// Where a resolution comes from, as a role: each view maps it to a name,
/// a symbol and a colour.
enum Provenance: Equatable {
    /// A side as git produced it.
    case git
    /// Made by a format rule.
    case rule
    /// Written by the user.
    case human
}

extension CandidateInfo {
    /// The ID the core gives a conflict resolved by hand.
    static let editedID = "edited"

    /// The "closed" variants are made by a rule, an edit by the user; the
    /// others are git's sides as they are.
    var provenance: Provenance {
        if id == Self.editedID { return .human }
        if id.hasSuffix("-closed") { return .rule }
        return .git
    }

    var doesNotParse: Bool {
        if case .doesNotParse = parse { return true }
        return false
    }
}

extension ConflictInfo {
    /// Lines of a candidate shown in its card.
    static let previewLines = 3

    /// The chosen candidate, if any. An edit is not a candidate the core
    /// offers: it is shown like one, with its check.
    var chosenCandidate: CandidateInfo? {
        guard let chosen else { return nil }
        guard chosen == CandidateInfo.editedID else {
            return candidates.first { $0.id == chosen }
        }
        let lines = editText.split(separator: "\n", omittingEmptySubsequences: false).dropLast()
        return CandidateInfo(
            id: CandidateInfo.editedID,
            title: "Edited by you",
            subtitle: "Written by hand",
            preview: lines.prefix(Self.previewLines).map(String.init),
            truncated: lines.count > Self.previewLines,
            parse: editedParse ?? .notChecked,
            suggested: false
        )
    }

    /// Its index, from 0.
    var index: Int { Int(number) - 1 }
}
