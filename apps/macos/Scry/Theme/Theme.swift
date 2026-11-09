import AppKit
import SwiftUI

/// Colour roles and metrics from the design tokens in
/// `scry-ui/prototype/tokens`. Views ask for a role, never a hex value;
/// the roles themselves are generated into `Colors.swift`.
enum Theme {
    struct RGBA: Sendable {
        let red: Double
        let green: Double
        let blue: Double
        let alpha: Double

        init(_ red: Double, _ green: Double, _ blue: Double, _ alpha: Double) {
            self.red = red
            self.green = green
            self.blue = blue
            self.alpha = alpha
        }

        var nsColor: NSColor {
            NSColor(srgbRed: red, green: green, blue: blue, alpha: alpha)
        }
    }

    /// User default for the colour-blind-safe palette (Settings › Appearance).
    static let colourBlindPaletteKey = "colourBlindSafePalette"

    /// A colour resolved when drawn, for the current appearance and palette.
    static func color(light: RGBA, dark: RGBA, colourBlindLight: RGBA, colourBlindDark: RGBA) -> Color {
        Color(nsColor: NSColor(name: nil) { appearance in
            let isDark = appearance.bestMatch(from: [.aqua, .darkAqua]) == .darkAqua
            let colourBlind = UserDefaults.standard.bool(forKey: colourBlindPaletteKey)
            switch (isDark, colourBlind) {
            case (false, false): return light.nsColor
            case (true, false): return dark.nsColor
            case (false, true): return colourBlindLight.nsColor
            case (true, true): return colourBlindDark.nsColor
            }
        })
    }
}

/// Metrics in points, from `tokens/layout.json` and `tokens/typography.json`.
enum Metrics {
    static let rowHeight: CGFloat = 20
    static let lineNumberWidth: CGFloat = 38
    static let lineNumberPadding: CGFloat = 6
    static let markWidth: CGFloat = 14
    static let lensHeight: CGFloat = 26
    static let connectorWidth: CGFloat = 22
    static let inspectorWidth: CGFloat = 320
    static let paneHeaderRule: CGFloat = 3
    static let statusBarHeight: CGFloat = 26
    static let provenanceStrip: CGFloat = 3
    static let blockTintOpacity = 0.07
    /// Changed words in a conflict side: the side's colour at 24 %.
    static let changedWordOpacity = 0.24
    static let badgeHeight: CGFloat = 18
    static let cardCornerRadius: CGFloat = 8
    static let previewLines = 3

    static let code = Font.system(size: 12, design: .monospaced)
    static let lineNumber = Font.system(size: 11, design: .monospaced)
    static let codeSmall = Font.system(size: 10.5, design: .monospaced)
    static let headline = Font.system(size: 13, weight: .semibold)
    static let callout = Font.system(size: 12)
    static let caption = Font.system(size: 11)
    static let captionBold = Font.system(size: 11, weight: .semibold)
}
