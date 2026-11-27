import Foundation
import Observation

/// What a blame reads: the core's `FileBlame`, or a test's own.
protocol BlameSource: Sendable {
    /// Reads the blame, until git is done or `stop` is called. Blocks.
    func read()
    func stop()
    /// What was found after the first `commits` commits and `lines`
    /// groups of lines, which the caller already has.
    func progress(commits: UInt32, lines: UInt32) -> BlameProgress
}

extension FileBlame: BlameSource {}

/// Blame is shown or not, for one window: every file and every point of
/// a history the window shows follows it. Not remembered between windows,
/// since blame costs time on each file.
@MainActor
@Observable
final class BlameSwitch {
    var isOn = false
}

/// What the gutter shows beside one line of the new side.
enum BlameCell: Equatable {
    /// Not read yet: a grey bar.
    case unknown
    /// In no commit yet. `first` on the first line of a run of lines.
    case notCommitted(first: Bool)
    case commit(BlameCommitInfo, first: Bool)
}

/// S17, one file's blame while it is read: who last wrote each line of
/// the new side, known little by little.
@MainActor
@Observable
final class BlameReading {
    private static let unknown: Int32 = -2
    private static let notCommitted: Int32 = -1

    private(set) var commits: [BlameCommitInfo] = []
    /// For each line of the new side, its commit's place among `commits`,
    /// or `unknown`, or `notCommitted`.
    private(set) var lines: [Int32]
    /// How many lines are known.
    private(set) var known = 0
    private(set) var done = false
    /// Why git gave no blame.
    private(set) var failure: String?
    /// The first line of the group under the pointer.
    var hovered: Int?

    @ObservationIgnored private let source: BlameSource
    /// The groups of lines taken from the source so far.
    @ObservationIgnored private var taken: UInt32 = 0
    /// What to wait between two looks at what git found.
    @ObservationIgnored private let pause: @Sendable () async -> Void

    /// `lineCount` is how many lines the new side has.
    init(
        source: BlameSource, lineCount: Int,
        pause: @escaping @Sendable () async -> Void = { try? await Task.sleep(for: .milliseconds(120)) }
    ) {
        self.source = source
        self.pause = pause
        lines = Array(repeating: Self.unknown, count: lineCount)
    }

    /// Reads the blame off the main thread and takes what git finds as
    /// it comes, until git is done or the task is cancelled.
    func run() async {
        let source = source
        let reading = Task.detached { source.read() }
        while !done {
            if Task.isCancelled {
                source.stop()
                return
            }
            take()
            if !done { await pause() }
        }
        await reading.value
    }

    /// Stops git: the file is no longer shown.
    func stop() {
        source.stop()
    }

    /// Takes what the source found since the last time.
    func take() {
        let progress = source.progress(commits: UInt32(commits.count), lines: taken)
        commits.append(contentsOf: progress.commits)
        taken += UInt32(progress.lines.count)
        for found in progress.lines {
            let value = found.commit.map(Int32.init) ?? Self.notCommitted
            let first = Int(found.first) - 1
            for line in first..<(first + Int(found.count)) where lines.indices.contains(line) {
                if lines[line] == Self.unknown { known += 1 }
                lines[line] = value
            }
        }
        if progress.done {
            if progress.neverCommitted {
                lines = Array(repeating: Self.notCommitted, count: lines.count)
                known = lines.count
            }
            failure = progress.failure
            done = true
        }
    }

    /// The gutter's cell for `line`, from 1, of the new side.
    func cell(line: UInt32) -> BlameCell {
        let index = Int(line) - 1
        guard lines.indices.contains(index) else { return .unknown }
        let value = lines[index]
        if value == Self.unknown { return .unknown }
        let first = index == 0 || lines[index - 1] != value
        if value == Self.notCommitted { return .notCommitted(first: first) }
        guard commits.indices.contains(Int(value)) else { return .unknown }
        return .commit(commits[Int(value)], first: first)
    }

    /// The first line, from 1, of the group `line` is in: consecutive
    /// lines of one commit.
    func groupStart(line: UInt32) -> Int {
        var index = Int(line) - 1
        guard lines.indices.contains(index) else { return Int(line) }
        let value = lines[index]
        while index > 0, lines[index - 1] == value { index -= 1 }
        return index + 1
    }

    /// "Blame: reading history… 46 %", while it is read.
    var status: String? {
        if let failure { return "Blame: \(failure)" }
        guard !done else { return nil }
        let share = lines.isEmpty ? 0 : known * 100 / lines.count
        return "Blame: reading history… \(share) %"
    }

    // MARK: Words

    /// "You", or the author's name.
    nonisolated static func author(_ commit: BlameCommitInfo) -> String {
        commit.mine ? "You" : commit.author
    }

    /// The author in the compact gutter: "You", or initials ("PS").
    nonisolated static func initials(_ commit: BlameCommitInfo) -> String {
        if commit.mine { return "You" }
        let words = commit.author.split(whereSeparator: { $0 == " " || $0 == "-" || $0 == "." || $0 == "_" })
        let letters = words.compactMap(\.first).filter(\.isLetter)
        guard let first = letters.first else { return String(commit.author.prefix(2)) }
        let last = letters.count > 1 ? String(letters[letters.count - 1]) : ""
        return (String(first) + last).uppercased()
    }

    /// The tooltip: the subject, then "351410c · You · 20 Nov 2026, 16:05".
    nonisolated static func help(_ commit: BlameCommitInfo, timeZone: TimeZone = .current) -> String {
        let date = Date(timeIntervalSince1970: TimeInterval(commit.time))
        var style = Date.FormatStyle(date: .abbreviated, time: .shortened)
        style.timeZone = timeZone
        return """
        \(commit.subject)
        \(commit.shortId) · \(author(commit)) · \(date.formatted(style))
        Click to see what this commit did to the file.
        """
    }
}
