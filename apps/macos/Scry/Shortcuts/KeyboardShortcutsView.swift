import SwiftUI

/// The Keyboard Shortcuts sheet (⌘/, Help ▸ Keyboard Shortcuts).
struct KeyboardShortcutsView: View {
    @Bindable var viewModel: KeyboardShortcutsViewModel
    let close: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack {
                Text("Keyboard Shortcuts")
                    .font(Metrics.headline)
                Spacer()
                TextField("Search", text: $viewModel.query)
                    .textFieldStyle(.roundedBorder)
                    .frame(width: 220)
            }
            .padding(16)
            Divider()
            ScrollView {
                VStack(alignment: .leading, spacing: 18) {
                    ForEach(viewModel.groups) { group in
                        ShortcutGroupView(title: group.title, rows: group.rows)
                    }
                    if viewModel.isEmpty {
                        Text("No shortcut matches.")
                            .foregroundStyle(Theme.Surfaces.textSecondary)
                            .frame(maxWidth: .infinity)
                            .padding(.top, 40)
                    }
                }
                .padding(16)
            }
            Divider()
            HStack {
                Text(viewModel.footer)
                    .font(Metrics.caption)
                    .foregroundStyle(Theme.Surfaces.textSecondary)
                Spacer()
                Button("Done", action: close)
                    .keyboardShortcut(.cancelAction)
            }
            .padding(16)
        }
        .frame(width: 760, height: 560)
        .background(Theme.Surfaces.window)
    }
}

/// One section of the sheet: each action with its keys.
struct ShortcutGroupView: View {
    let title: String
    let rows: [KeyboardShortcutsViewModel.Row]

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text(title)
                .font(Metrics.captionBold)
                .foregroundStyle(Theme.Surfaces.textSecondary)
            ForEach(rows) { row in
                HStack(alignment: .firstTextBaseline) {
                    VStack(alignment: .leading, spacing: 1) {
                        Text(row.title)
                            .font(Metrics.callout)
                        if let note = row.note {
                            Text(note)
                                .font(Metrics.caption)
                                .foregroundStyle(Theme.Surfaces.textSecondary)
                        }
                    }
                    Spacer()
                    KeyCap(keys: row.keys)
                }
                .padding(.vertical, 2)
            }
        }
    }
}

/// Keys as a small key cap: "⌥⌘1".
struct KeyCap: View {
    let keys: String

    var body: some View {
        Text(keys)
            .font(Metrics.callout.monospacedDigit())
            .padding(.horizontal, 6)
            .padding(.vertical, 2)
            .background(
                RoundedRectangle(cornerRadius: 5)
                    .fill(Theme.Surfaces.chrome)
            )
            .overlay(
                RoundedRectangle(cornerRadius: 5)
                    .strokeBorder(Theme.Surfaces.textSecondary.opacity(0.3))
            )
    }
}

#Preview("Keyboard Shortcuts, editor") {
    KeyboardShortcutsView(viewModel: KeyboardShortcutsViewModel(current: .editor)) {}
}

#Preview("Keyboard Shortcuts, searching") {
    let viewModel = KeyboardShortcutsViewModel(current: .overview)
    viewModel.query = "take"
    return KeyboardShortcutsView(viewModel: viewModel) {}
}
