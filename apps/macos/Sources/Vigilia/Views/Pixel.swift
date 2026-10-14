import SwiftUI

/// The pixel-art look of `docs/assets/vigilia-banner.svg`: white pixels on black, with the
/// lantern's warm colours as accents. Sizes are multiples of `Pixel.unit` so edges stay crisp.
enum Pixel {
    /// One art pixel, in points.
    static let unit: CGFloat = 2

    static let background = Color.black
    static let ink = Color.white
    static let dim = Color.white.opacity(0.55)
    static let faint = Color.white.opacity(0.25)
    static let yellow = Color(red: 1, green: 0.824, blue: 0.247)  // #ffd23f
    static let orange = Color(red: 1, green: 0.549, blue: 0.102)  // #ff8c1a
    static let red = Color(red: 0.902, green: 0.231, blue: 0.122)  // #e63b1f

    /// Small explanatory text stays in a system font so it reads easily.
    static let body = Font.system(size: 11, design: .monospaced)
}

// MARK: - Text

private struct PixelInkKey: EnvironmentKey {
    static let defaultValue = Pixel.ink
}

extension EnvironmentValues {
    /// The colour `PixelText` draws in unless given one; buttons set it for their state.
    var pixelInk: Color {
        get { self[PixelInkKey.self] }
        set { self[PixelInkKey.self] = newValue }
    }
}

/// Text drawn in `PixelFont`. `scale` is points per font pixel.
struct PixelText: View {
    let text: String
    var scale: CGFloat = Pixel.unit
    var color: Color?

    @Environment(\.pixelInk) private var ink
    @Environment(\.isEnabled) private var isEnabled

    init(_ text: String, scale: CGFloat = Pixel.unit, color: Color? = nil) {
        self.text = text
        self.scale = scale
        self.color = color
    }

    var body: some View {
        let layout = PixelFont.layout(text)
        let fill = (color ?? ink).opacity(isEnabled ? 1 : 0.4)
        Canvas { context, _ in
            var path = Path()
            for p in layout.pixels {
                path.addRect(CGRect(x: p.x * scale, y: p.y * scale, width: scale, height: scale))
            }
            context.fill(path, with: .color(fill))
        }
        .frame(width: CGFloat(layout.width) * scale, height: CGFloat(PixelFont.height) * scale)
        .accessibilityElement()
        .accessibilityLabel(text.capitalized)
    }
}

// MARK: - Boxes

/// A rectangle with its corners cut by one step, the way pixel art rounds corners.
struct NotchedRect: Shape {
    var step: CGFloat = Pixel.unit

    func path(in r: CGRect) -> Path {
        let s = min(step, r.width / 2, r.height / 2)
        var p = Path()
        p.move(to: CGPoint(x: r.minX + s, y: r.minY))
        p.addLine(to: CGPoint(x: r.maxX - s, y: r.minY))
        p.addLine(to: CGPoint(x: r.maxX - s, y: r.minY + s))
        p.addLine(to: CGPoint(x: r.maxX, y: r.minY + s))
        p.addLine(to: CGPoint(x: r.maxX, y: r.maxY - s))
        p.addLine(to: CGPoint(x: r.maxX - s, y: r.maxY - s))
        p.addLine(to: CGPoint(x: r.maxX - s, y: r.maxY))
        p.addLine(to: CGPoint(x: r.minX + s, y: r.maxY))
        p.addLine(to: CGPoint(x: r.minX + s, y: r.maxY - s))
        p.addLine(to: CGPoint(x: r.minX, y: r.maxY - s))
        p.addLine(to: CGPoint(x: r.minX, y: r.minY + s))
        p.addLine(to: CGPoint(x: r.minX + s, y: r.minY + s))
        p.closeSubpath()
        return p
    }
}

/// A one-pixel border with notched corners around a filled box.
struct PixelBox: View {
    var border: Color = Pixel.ink
    var fill: Color = Pixel.background

    var body: some View {
        ZStack {
            NotchedRect().fill(border)
            NotchedRect().fill(fill).padding(Pixel.unit)
        }
    }
}

/// The dotted line under headings in the banner.
struct PixelRule: View {
    var color: Color = Pixel.orange

    var body: some View {
        Canvas { context, size in
            var path = Path()
            var x: CGFloat = 0
            while x + Pixel.unit <= size.width {
                path.addRect(CGRect(x: x, y: 0, width: Pixel.unit, height: Pixel.unit))
                x += Pixel.unit * 2
            }
            context.fill(path, with: .color(color))
        }
        .frame(height: Pixel.unit)
        .accessibilityHidden(true)
    }
}

/// A few fixed stars, as in the banner's night sky.
struct PixelStars: View {
    private static let stars: [(CGFloat, CGFloat)] = [
        (0.06, 0.2), (0.21, 0.75), (0.37, 0.12), (0.52, 0.6), (0.66, 0.25), (0.81, 0.8), (0.93, 0.35),
    ]

    var body: some View {
        Canvas { context, size in
            for (x, y) in Self.stars {
                let rect = CGRect(x: (x * size.width).rounded(), y: (y * size.height).rounded(), width: Pixel.unit, height: Pixel.unit)
                context.fill(Path(rect), with: .color(Pixel.faint))
            }
        }
        .accessibilityHidden(true)
    }
}

/// A section heading: orange pixel text over a dotted rule.
struct PixelHeading: View {
    let title: String

    var body: some View {
        VStack(alignment: .leading, spacing: Pixel.unit * 2) {
            PixelText(title, color: Pixel.orange)
            PixelRule()
        }
        .accessibilityElement(children: .combine)
        .accessibilityAddTraits(.isHeader)
    }
}

// MARK: - Controls

/// Buttons as pixel boxes. Pressing inverts them, like a selected item in an old game menu.
struct PixelButtonStyle: ButtonStyle {
    enum Kind { case normal, primary, plain }
    var kind: Kind = .normal

    func makeBody(configuration: Configuration) -> some View {
        PixelButton(configuration: configuration, kind: kind)
    }

    private struct PixelButton: View {
        let configuration: Configuration
        let kind: Kind
        @Environment(\.isEnabled) private var isEnabled

        var body: some View {
            let pressed = configuration.isPressed
            let (border, fill, ink): (Color, Color, Color) = {
                switch kind {
                case .primary:
                    return (Pixel.orange, pressed ? Pixel.yellow : Pixel.orange, Pixel.background)
                case .normal:
                    return (Pixel.ink, pressed ? Pixel.ink : Pixel.background, pressed ? Pixel.background : Pixel.ink)
                case .plain:
                    return (.clear, .clear, pressed ? Pixel.orange : Pixel.ink)
                }
            }()
            configuration.label
                .environment(\.pixelInk, ink)
                .padding(.horizontal, kind == .plain ? 0 : Pixel.unit * 4)
                .padding(.vertical, kind == .plain ? Pixel.unit : Pixel.unit * 3)
                .background(PixelBox(border: border, fill: fill))
                .opacity(isEnabled ? 1 : 0.45)
                .contentShape(Rectangle())
        }
    }
}

/// A pixel checkbox: a box that fills orange when on.
struct PixelToggleStyle: ToggleStyle {
    func makeBody(configuration: Configuration) -> some View {
        PixelToggle(configuration: configuration)
    }

    private struct PixelToggle: View {
        let configuration: Configuration
        @Environment(\.isEnabled) private var isEnabled

        var body: some View {
            Button {
                configuration.isOn.toggle()
            } label: {
                HStack(spacing: Pixel.unit * 4) {
                    ZStack {
                        PixelBox()
                        if configuration.isOn {
                            Rectangle().fill(Pixel.orange).padding(Pixel.unit * 2)
                        }
                    }
                    .frame(width: Pixel.unit * 7, height: Pixel.unit * 7)
                    configuration.label
                }
                .opacity(isEnabled ? 1 : 0.45)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .accessibilityValue(configuration.isOn ? "on" : "off")
        }
    }
}

extension View {
    /// The menu and Settings are always drawn dark, like the banner.
    func pixelSurface() -> some View {
        self
            .font(Pixel.body)
            .foregroundColor(Pixel.ink)
            .tint(Pixel.orange)
            .environment(\.colorScheme, .dark)
    }
}
