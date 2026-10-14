import SwiftUI

/// Ends the session only after a press held for a second, so a stray click can't end it.
/// A bar of pixel blocks fills while holding; letting go early does nothing.
struct HoldToEndButton: View {
    var title = "HOLD TO END"
    let action: () -> Void

    @State private var progress: CGFloat = 0
    @State private var holding = false
    @Environment(\.isEnabled) private var isEnabled

    var body: some View {
        ZStack {
            PixelBox(border: Pixel.red)
            BlockFill(progress: progress)
                .fill(Pixel.red)
                .padding(Pixel.unit * 2)
            PixelText(holding ? "KEEP HOLDING" : title)
        }
        .frame(height: Pixel.unit * 14)
        .opacity(isEnabled ? 1 : 0.45)
        .contentShape(Rectangle())
        .onLongPressGesture(minimumDuration: 1, maximumDistance: 30) {
            progress = 0
            holding = false
            action()
        } onPressingChanged: { pressing in
            holding = pressing
            withAnimation(pressing ? .linear(duration: 1) : .easeOut(duration: 0.15)) {
                progress = pressing ? 1 : 0
            }
        }
        .accessibilityElement(children: .ignore)
        .accessibilityAddTraits(.isButton)
        .accessibilityLabel("Hold to end the session")
        .accessibilityAction { action() }
    }
}

/// Whole blocks from the left, so the bar fills in steps rather than smoothly.
private struct BlockFill: Shape {
    var progress: CGFloat
    var blocks = 16

    var animatableData: CGFloat {
        get { progress }
        set { progress = newValue }
    }

    func path(in rect: CGRect) -> Path {
        let filled = Int((progress * CGFloat(blocks)).rounded(.down))
        let width = rect.width / CGFloat(blocks)
        var path = Path()
        for i in 0..<filled {
            path.addRect(CGRect(x: rect.minX + CGFloat(i) * width, y: rect.minY, width: width - Pixel.unit, height: rect.height))
        }
        return path
    }
}
