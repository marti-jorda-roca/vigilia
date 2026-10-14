import AppKit
import SwiftUI

/// The pixel lantern from the banner, used as the menu bar icon and in the menu.
enum Lantern {
    enum State: Equatable {
        /// No session: an empty lantern.
        case unlit
        /// A session is keeping the Mac awake.
        case lit
        /// A warning or a failed undo: a red flame.
        case alert
    }

    /// One character per pixel: `F` frame, `W` wick, `Y` `O` `D` the flame (yellow, orange,
    /// deep red), `r` glow around the glass.
    static let art = [
        "......F......",
        ".....F.F.....",
        "......F......",
        "....FFFFF....",
        "...FFFFFFF...",
        "...F.....F...",
        "...F..Y..F...",
        "r..F.YYY.F..r",
        "...F.YOY.F...",
        "r..F.OOO.F..r",
        "...F.ODO.F...",
        "...F..W..F...",
        "...FFFFFFF...",
        "..FFFFFFFFF..",
    ]

    static let width = art[0].count
    static let height = art.count

    /// The colour of each pixel for `state`, with `frame` for the metal. Unlit, only the frame
    /// and the wick are drawn; lit, the flame and its glow; on alert, a red flame.
    static func pixels(state: State, frame: CGColor) -> [(x: Int, y: Int, color: CGColor)] {
        let yellow = CGColor(red: 1, green: 0.824, blue: 0.247, alpha: 1)
        let orange = CGColor(red: 1, green: 0.549, blue: 0.102, alpha: 1)
        let red = CGColor(red: 0.902, green: 0.231, blue: 0.122, alpha: 1)

        var out: [(Int, Int, CGColor)] = []
        for (y, row) in art.enumerated() {
            for (x, cell) in row.enumerated() {
                let color: CGColor?
                switch (cell, state) {
                case ("F", _), ("W", _):
                    color = frame
                case ("Y", .lit):
                    color = yellow
                case ("O", .lit), ("r", .lit):
                    color = orange
                case ("D", .lit), ("Y", .alert), ("O", .alert), ("D", .alert):
                    color = red
                default:
                    color = nil
                }
                if let color { out.append((x, y, color)) }
            }
        }
        return out
    }

    /// The menu bar image, one point per pixel. The unlit lantern is a template image, so the
    /// system tints it; the coloured ones draw their frame in `frame` (black or white to match
    /// the menu bar).
    static func statusImage(state: State, darkMenuBar: Bool) -> NSImage {
        let frame = state == .unlit || !darkMenuBar ? CGColor(gray: 0, alpha: 1) : CGColor(gray: 1, alpha: 1)
        let pixels = pixels(state: state, frame: frame)
        let image = NSImage(size: NSSize(width: width, height: height), flipped: true) { _ in
            guard let context = NSGraphicsContext.current?.cgContext else { return false }
            for p in pixels {
                context.setFillColor(p.color)
                context.fill(CGRect(x: p.x, y: p.y, width: 1, height: 1))
            }
            return true
        }
        image.isTemplate = state == .unlit
        image.accessibilityDescription = switch state {
        case .unlit: "Vigilia"
        case .lit: "Vigilia: keeping awake"
        case .alert: "Vigilia needs attention"
        }
        return image
    }
}

/// The lantern at a larger pixel size, for the menu header.
struct LanternView: View {
    let state: Lantern.State
    var scale: CGFloat = Pixel.unit

    var body: some View {
        let pixels = Lantern.pixels(state: state, frame: CGColor(gray: 1, alpha: 1))
        Canvas { context, _ in
            for p in pixels {
                let rect = CGRect(x: CGFloat(p.x) * scale, y: CGFloat(p.y) * scale, width: scale, height: scale)
                context.fill(Path(rect), with: .color(Color(cgColor: p.color)))
            }
        }
        .frame(width: CGFloat(Lantern.width) * scale, height: CGFloat(Lantern.height) * scale)
        .accessibilityHidden(true)
    }
}
