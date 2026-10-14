import AppKit
import SwiftUI

/// Where the menu goes: right under the status item, like a regular menu, kept on screen.
enum MenuPlacement {
    /// Space between the menu bar and the menu.
    static let gap: CGFloat = 4
    /// Closest the menu gets to a screen edge.
    static let margin: CGFloat = 8

    /// The menu's frame in screen coordinates. `anchor` is the status item button's frame;
    /// `screen` is the frame of the screen it is on. The left edges line up, unless that would
    /// push the menu off the screen.
    static func frame(size: CGSize, below anchor: CGRect, on screen: CGRect) -> CGRect {
        var x = anchor.minX
        x = min(x, screen.maxX - margin - size.width)
        x = max(x, screen.minX + margin)
        let y = anchor.minY - gap - size.height
        return CGRect(origin: CGPoint(x: x, y: y), size: size).integral
    }
}

/// A borderless panel for the menu. It can take keyboard focus (for the timeout field) without
/// activating the app, so the app you were using stays in front.
final class MenuPanel: NSPanel {
    init() {
        super.init(
            contentRect: .zero, styleMask: [.borderless, .nonactivatingPanel], backing: .buffered,
            defer: true)
        isFloatingPanel = true
        level = .popUpMenu
        backgroundColor = .clear
        isOpaque = false
        hasShadow = true
        hidesOnDeactivate = false
        isReleasedWhenClosed = false
        collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary, .transient]
        appearance = NSAppearance(named: .darkAqua)
    }

    override var canBecomeKey: Bool { true }
}
