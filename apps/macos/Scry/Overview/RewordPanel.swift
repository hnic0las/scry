import AppKit
import SwiftUI

/// A `reword` git already committed, with no files left: the message
/// takes the file list's place, with what the commit changes below so the
/// user reads it while writing (SPEC: Reword during `git rebase -i`).
struct RewordPanel: View {
    let title: String
    @Binding var message: String
    let note: OverviewViewModel.MessageNote
    /// The message differs from the commit's own.
    let edited: Bool
    let files: [OverviewViewModel.ChangedFileRow]
    let selection: String?
    let diff: [OverviewViewModel.DiffRow]
    /// "You · 3 days ago · replayed without conflicts".
    let commitDetail: String
    let select: (String?) -> Void
    let reset: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack(alignment: .firstTextBaseline) {
                Text(title)
                    .font(Metrics.headline)
                    .foregroundStyle(Theme.Surfaces.textSecondary)
                Spacer()
                RevertButton(edited: edited, reset: reset)
            }
            CommitMessageEditor(message: $message)
                .frame(maxWidth: 880, alignment: .leading)
                .frame(height: 132)
            MessageNoteView(note: note)
            HStack(alignment: .firstTextBaseline) {
                Text("In this commit")
                    .font(Metrics.captionBold)
                Spacer()
                Text(commitDetail)
                    .font(Metrics.caption)
                    .foregroundStyle(Theme.Surfaces.textSecondary)
            }
            .padding(.top, 8)
            HStack(spacing: 0) {
                ChangedFileList(files: files, selection: selection, select: select)
                    .frame(width: 280)
                Divider()
                CommitDiffView(lines: diff)
            }
            .background(Theme.Surfaces.pane)
            .clipShape(RoundedRectangle(cornerRadius: 8))
            .overlay(RoundedRectangle(cornerRadius: 8).strokeBorder(Theme.Surfaces.hairline))
        }
        .padding(.horizontal, 24)
        .padding(.vertical, 18)
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        .background(Theme.Surfaces.window)
    }
}

/// A `reword` that stopped on conflicts: the message editor docked under
/// the file table, used when the user continues.
struct RewordDock: View {
    let title: String
    @Binding var message: String
    /// "Original message", or "Subject: 89 of 72".
    let status: OverviewViewModel.MessageNote
    let edited: Bool
    let reset: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack(alignment: .firstTextBaseline, spacing: 8) {
                Image(systemName: "pencil")
                    .foregroundStyle(Theme.Interaction.accent)
                Text(title)
                    .font(Metrics.callout)
                    .foregroundStyle(Theme.Surfaces.textSecondary)
                Spacer()
                Text(status.text)
                    .font(Metrics.callout.weight(status.warning ? .semibold : .regular))
                    .foregroundStyle(status.warning ? Theme.Severity.warning : Theme.Surfaces.textSecondary)
                RevertButton(edited: edited, reset: reset)
            }
            CommitMessageEditor(message: $message)
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 10)
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)
        .background(Theme.Surfaces.chrome)
    }
}

private struct RevertButton: View {
    let edited: Bool
    let reset: () -> Void

    var body: some View {
        Button("Revert to Original", action: reset)
            .disabled(!edited)
            .help("Put back the message the commit had before the rebase")
    }
}

/// The remark under the editor: "Edited.", or a warning.
struct MessageNoteView: View {
    let note: OverviewViewModel.MessageNote

    var body: some View {
        Label {
            Text(note.text)
        } icon: {
            if note.warning {
                Image(systemName: "exclamationmark.triangle")
            }
        }
        .font(Metrics.callout)
        .foregroundStyle(note.warning ? Theme.Severity.warning : Theme.Surfaces.textSecondary)
    }
}

/// The reworded commit's files, one always selected; ↑ and ↓ move the
/// selection when the list has focus.
struct ChangedFileList: View {
    let files: [OverviewViewModel.ChangedFileRow]
    let selection: String?
    let select: (String?) -> Void

    var body: some View {
        List(selection: Binding(get: { selection }, set: { if $0 != nil { select($0) } })) {
            ForEach(files) { file in
                HStack(alignment: .firstTextBaseline) {
                    VStack(alignment: .leading, spacing: 1) {
                        Text(file.name)
                            .font(Metrics.code.weight(.semibold))
                        Text(file.directory)
                            .font(Metrics.codeSmall)
                            .foregroundStyle(Theme.Surfaces.textSecondary)
                    }
                    Spacer(minLength: 4)
                    if let added = file.added {
                        Text(added).foregroundStyle(Theme.Diff.addedMark)
                    }
                    if let removed = file.removed {
                        Text(removed).foregroundStyle(Theme.Diff.removedMark)
                    }
                }
                .font(Metrics.code)
                .padding(.vertical, 3)
                .tag(file.path)
                .accessibilityElement(children: .combine)
            }
        }
        .listStyle(.sidebar)
        .scrollContentBackground(.hidden)
    }
}

/// One file's diff in the commit: unified, read only, scrolling on its
/// own, with the `added` and `removed` roles so the colour-blind palette
/// applies.
struct CommitDiffView: View {
    let lines: [OverviewViewModel.DiffRow]
    /// The visible width, so tinted lines reach the right edge.
    @State private var width: CGFloat = 0

    var body: some View {
        ScrollView([.vertical, .horizontal]) {
            LazyVStack(alignment: .leading, spacing: 0) {
                ForEach(lines) { line in
                    HStack(spacing: 0) {
                        Text(line.number)
                            .font(Metrics.lineNumber)
                            .foregroundStyle(Theme.Surfaces.textTertiary)
                            .frame(width: 44, alignment: .trailing)
                        Text(mark(line.kind))
                            .foregroundStyle(markColor(line.kind))
                            .frame(width: 22)
                        Text(line.text)
                            .foregroundStyle(line.kind == .hunk ? Theme.Surfaces.textSecondary : Theme.Surfaces.text)
                            .fixedSize()
                        Spacer(minLength: 12)
                    }
                    .font(Metrics.code)
                    .frame(height: Metrics.rowHeight)
                    .frame(minWidth: width, alignment: .leading)
                    .background(background(line.kind))
                }
            }
            .padding(.vertical, 4)
        }
        .onGeometryChange(for: CGFloat.self) { $0.size.width } action: { width = $0 }
        .overlay {
            if lines.isEmpty {
                Text("No changes to show.")
                    .foregroundStyle(Theme.Surfaces.textSecondary)
            }
        }
    }

    private func mark(_ kind: DiffLineKind) -> String {
        switch kind {
        case .added: "+"
        case .removed: "−"
        case .hunk, .context: ""
        }
    }

    private func markColor(_ kind: DiffLineKind) -> Color {
        switch kind {
        case .added: Theme.Diff.addedMark
        case .removed: Theme.Diff.removedMark
        case .hunk, .context: Theme.Surfaces.textTertiary
        }
    }

    private func background(_ kind: DiffLineKind) -> Color {
        switch kind {
        case .added: Theme.Diff.added
        case .removed: Theme.Diff.removed
        case .hunk: Theme.Diff.context
        case .context: Color.clear
        }
    }
}

/// A commit message editor: SF Mono 12 on 20 pt lines, "Subject" and
/// "Body" in a gutter, a dashed guide
/// at column 72, the subject's length at its right, and the characters
/// past 72 tinted. Advice only: the text is never wrapped or cut.
struct CommitMessageEditor: View {
    @Binding var message: String
    @State private var focused = false

    var body: some View {
        MessageTextEditor(message: $message, focusChanged: { focused = $0 })
            .padding(1)
            .background(RoundedRectangle(cornerRadius: 6).fill(Theme.Surfaces.pane))
            .overlay(
                RoundedRectangle(cornerRadius: 6)
                    .strokeBorder(focused ? Theme.Interaction.accent : Theme.Surfaces.hairline, lineWidth: focused ? 2 : 1)
            )
            .clipShape(RoundedRectangle(cornerRadius: 6))
    }
}

private struct MessageTextEditor: NSViewRepresentable {
    @Binding var message: String
    let focusChanged: (Bool) -> Void

    func makeCoordinator() -> Coordinator { Coordinator(message: $message) }

    func makeNSView(context: Context) -> NSScrollView {
        let textView = MessageTextView()
        textView.delegate = context.coordinator
        textView.isRichText = false
        textView.allowsUndo = true
        textView.isAutomaticQuoteSubstitutionEnabled = false
        textView.isAutomaticDashSubstitutionEnabled = false
        textView.isAutomaticTextReplacementEnabled = false
        textView.isAutomaticSpellingCorrectionEnabled = false
        textView.drawsBackground = false
        textView.textContainerInset = NSSize(width: MessageTextView.gutter, height: 10)
        textView.textContainer?.lineFragmentPadding = 0
        textView.textContainer?.widthTracksTextView = true
        textView.isVerticallyResizable = true
        textView.isHorizontallyResizable = false
        textView.minSize = .zero
        textView.maxSize = NSSize(width: CGFloat.greatestFiniteMagnitude, height: .greatestFiniteMagnitude)
        textView.textContainer?.containerSize = NSSize(width: 0, height: CGFloat.greatestFiniteMagnitude)
        textView.autoresizingMask = [.width, .height]
        textView.setAccessibilityLabel("Commit message")
        textView.string = message
        textView.restyle()

        let scrollView = NSScrollView()
        scrollView.documentView = textView
        scrollView.hasVerticalScroller = true
        scrollView.autohidesScrollers = true
        scrollView.drawsBackground = false
        scrollView.borderType = .noBorder
        textView.focusChanged = focusChanged
        return scrollView
    }

    func updateNSView(_ scrollView: NSScrollView, context: Context) {
        guard let textView = scrollView.documentView as? MessageTextView else { return }
        if textView.string != message {
            textView.string = message
            textView.restyle()
        }
    }

    final class Coordinator: NSObject, NSTextViewDelegate {
        let message: Binding<String>

        init(message: Binding<String>) {
            self.message = message
        }

        func textDidChange(_ notification: Notification) {
            guard let textView = notification.object as? MessageTextView else { return }
            textView.restyle()
            message.wrappedValue = textView.string
        }
    }
}

/// The text view under `CommitMessageEditor`, drawing its gutter and
/// guides over the text.
final class MessageTextView: NSTextView {
    static let gutter: CGFloat = 64
    static let lineHeight: CGFloat = 20

    var focusChanged: ((Bool) -> Void)?

    private let regular = NSFont.monospacedSystemFont(ofSize: 12, weight: .regular)
    private let semibold = NSFont.monospacedSystemFont(ofSize: 12, weight: .semibold)
    private var characterWidth: CGFloat { Metrics.codeCharacterWidth }

    private var subject: Substring { string.prefix { $0 != "\n" } }

    override func becomeFirstResponder() -> Bool {
        let became = super.becomeFirstResponder()
        if became { focusChanged?(true) }
        return became
    }

    override func resignFirstResponder() -> Bool {
        let resigned = super.resignFirstResponder()
        if resigned { focusChanged?(false) }
        return resigned
    }

    /// Fonts and tints from the text: the subject semibold, its
    /// characters past the limit tinted and underlined.
    func restyle() {
        guard let storage = textStorage else { return }
        let paragraph = NSMutableParagraphStyle()
        paragraph.minimumLineHeight = Self.lineHeight
        paragraph.maximumLineHeight = Self.lineHeight
        let base: [NSAttributedString.Key: Any] = [
            .font: regular,
            .foregroundColor: NSColor(Theme.Surfaces.text),
            .paragraphStyle: paragraph,
        ]
        typingAttributes = base
        let text = string
        let all = NSRange(text.startIndex..., in: text)
        storage.beginEditing()
        storage.setAttributes(base, range: all)
        let subject = self.subject
        storage.addAttribute(.font, value: semibold, range: NSRange(subject.startIndex..<subject.endIndex, in: text))
        if subject.count > OverviewViewModel.subjectLimit {
            let start = subject.index(subject.startIndex, offsetBy: OverviewViewModel.subjectLimit)
            storage.addAttributes([
                .backgroundColor: NSColor(Theme.KeyboardHintsAndCommitMessage.subjectOverflowTint),
                .underlineStyle: NSUnderlineStyle.single.rawValue,
                .underlineColor: NSColor(Theme.Severity.warning),
            ], range: NSRange(start..<subject.endIndex, in: text))
        }
        storage.endEditing()
        // What the user types next: semibold on the subject line.
        typingAttributes[.font] = selectedRange().location <= subject.utf16.count ? semibold : regular
        needsDisplay = true
    }

    override func setSelectedRanges(
        _ ranges: [NSValue], affinity: NSSelectionAffinity, stillSelecting: Bool
    ) {
        super.setSelectedRanges(ranges, affinity: affinity, stillSelecting: stillSelecting)
        typingAttributes[.font] = selectedRange().location <= subject.utf16.count ? semibold : regular
    }

    /// At least as tall as the scroll view, so the guide at column 72
    /// runs to the bottom.
    override func setFrameSize(_ newSize: NSSize) {
        let visible = enclosingScrollView?.contentSize.height ?? 0
        super.setFrameSize(NSSize(width: newSize.width, height: max(newSize.height, visible)))
    }

    override func draw(_ dirtyRect: NSRect) {
        super.draw(dirtyRect)
        // NSTextView leaves the clip on its text container: the gutter and
        // the count are outside it.
        NSGraphicsContext.saveGraphicsState()
        defer { NSGraphicsContext.restoreGraphicsState() }
        NSBezierPath(rect: dirtyRect).setClip()
        let origin = textContainerOrigin
        let guide = NSColor(Theme.KeyboardHintsAndCommitMessage.messageGuide)

        // Column 72, over the full height: the subject's limit and the
        // body's wrap guide.
        let limitX = (origin.x + CGFloat(OverviewViewModel.subjectLimit) * characterWidth).rounded() + 0.5
        let limit = NSBezierPath()
        limit.move(to: NSPoint(x: limitX, y: bounds.minY))
        limit.line(to: NSPoint(x: limitX, y: bounds.maxY))
        limit.setLineDash([3, 3], count: 2, phase: 0)
        guide.setStroke()
        limit.stroke()

        let subjectRect = lineRect(at: 0)
        drawLabel("Subject", in: subjectRect)
        if let body = bodyLine() {
            drawLabel("Body", in: lineRect(at: body))
        }

        let count = subject.count
        if string.isEmpty {
            let placeholder = NSAttributedString(string: "Commit message", attributes: [
                .font: semibold,
                .foregroundColor: NSColor(Theme.Surfaces.textTertiary),
            ])
            placeholder.draw(at: NSPoint(x: origin.x, y: subjectRect.minY + 2))
        } else {
            let tooLong = count > OverviewViewModel.subjectLimit
            let label = NSAttributedString(string: "\(count)", attributes: [
                .font: NSFont.monospacedDigitSystemFont(ofSize: 11, weight: tooLong ? .semibold : .regular),
                .foregroundColor: NSColor(tooLong ? Theme.Severity.warning : Theme.Surfaces.textTertiary),
            ])
            let size = label.size()
            label.draw(at: NSPoint(x: bounds.maxX - size.width - 12, y: subjectRect.midY - size.height / 2))
        }
    }

    /// The first body line: line 3, after the blank line, or line 2 when
    /// the user typed there.
    private func bodyLine() -> Int? {
        let lines = string.split(separator: "\n", omittingEmptySubsequences: false)
        if lines.count > 1 && !lines[1].trimmingCharacters(in: .whitespaces).isEmpty { return 1 }
        if lines.count > 2 { return 2 }
        return nil
    }

    /// The rectangle of the line `index` (0 for the subject), in the
    /// view's coordinates, the first fragment when it wraps.
    private func lineRect(at index: Int) -> NSRect {
        let origin = textContainerOrigin
        let fallback = NSRect(
            x: origin.x, y: origin.y + CGFloat(index) * Self.lineHeight,
            width: bounds.width, height: Self.lineHeight
        )
        guard let layoutManager, let textContainer else { return fallback }
        var offset = 0
        var current = 0
        for character in string.utf16 {
            if current == index { break }
            offset += 1
            if character == 10 { current += 1 }
        }
        guard current == index else { return fallback }
        layoutManager.ensureLayout(for: textContainer)
        let rect: NSRect
        if offset >= (string as NSString).length {
            rect = layoutManager.extraLineFragmentRect
            if rect.isEmpty && offset == 0 { return fallback }
        } else {
            let glyph = layoutManager.glyphIndexForCharacter(at: offset)
            rect = layoutManager.lineFragmentRect(forGlyphAt: glyph, effectiveRange: nil)
        }
        return rect.offsetBy(dx: origin.x, dy: origin.y)
    }

    private func drawLabel(_ text: String, in line: NSRect) {
        let label = NSAttributedString(string: text, attributes: [
            .font: NSFont.systemFont(ofSize: 11),
            .foregroundColor: NSColor(Theme.Surfaces.textTertiary),
        ])
        let size = label.size()
        label.draw(at: NSPoint(x: Self.gutter - size.width - 10, y: line.midY - size.height / 2))
    }
}
