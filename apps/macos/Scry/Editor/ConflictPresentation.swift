import Foundation

extension CandidateInfo {
    var doesNotParse: Bool {
        if case .doesNotParse = parse { return true }
        return false
    }
}

extension ConflictInfo {
    /// Its index, from 0.
    var index: Int { Int(number) - 1 }
}
