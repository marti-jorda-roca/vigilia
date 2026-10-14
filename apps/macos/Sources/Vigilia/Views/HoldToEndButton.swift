import SwiftUI

/// Ends the session only after a press held for a second, so a stray click can't end it.
/// A ring fills while holding; letting go early does nothing.
struct HoldToEndButton: View {
    var title = "Hold to end"
    let action: () -> Void

    @State private var progress: CGFloat = 0
    @State private var holding = false

    var body: some View {
        HStack(spacing: 8) {
            ZStack {
                Circle().stroke(Color.secondary.opacity(0.3), lineWidth: 2.5)
                Circle()
                    .trim(from: 0, to: progress)
                    .stroke(Color.red, style: StrokeStyle(lineWidth: 2.5, lineCap: .round))
                    .rotationEffect(.degrees(-90))
            }
            .frame(width: 16, height: 16)
            Text(holding ? "Keep holding…" : title)
        }
        .frame(maxWidth: .infinity)
        .padding(.vertical, 7)
        .background(RoundedRectangle(cornerRadius: 7).fill(Color.red.opacity(holding ? 0.2 : 0.1)))
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
        .accessibilityLabel(title)
        .accessibilityAction { action() }
    }
}
