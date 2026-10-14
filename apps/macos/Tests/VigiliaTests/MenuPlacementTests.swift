import CoreGraphics
import Testing

@testable import Vigilia

/// The menu must open right under the status item, not wherever the window system puts it.
struct MenuPlacementTests {
    // A 1512x982 screen; AppKit's origin is bottom-left, so the menu bar is at the top.
    let screen = CGRect(x: 0, y: 0, width: 1512, height: 982)
    let size = CGSize(width: 288, height: 300)

    private func button(x: CGFloat) -> CGRect {
        CGRect(x: x, y: 958, width: 70, height: 24)
    }

    @Test func hangsFromTheStatusItem() {
        let frame = MenuPlacement.frame(size: size, below: button(x: 900), on: screen)
        #expect(frame.minX == 900)
        #expect(frame.maxY == 958 - MenuPlacement.gap)
        #expect(frame.size == size)
    }

    @Test func staysOnScreenAtTheRightEdge() {
        let frame = MenuPlacement.frame(size: size, below: button(x: 1440), on: screen)
        #expect(frame.maxX == screen.maxX - MenuPlacement.margin)
        #expect(frame.maxY == 958 - MenuPlacement.gap)
    }

    @Test func staysOnScreenAtTheLeftEdge() {
        let frame = MenuPlacement.frame(size: size, below: button(x: 2), on: screen)
        #expect(frame.minX == MenuPlacement.margin)
    }

    @Test func worksOnASecondScreen() {
        // A screen to the right of the main one, taller and offset.
        let second = CGRect(x: 1512, y: -200, width: 2560, height: 1440)
        let anchor = CGRect(x: 3500, y: 1216, width: 70, height: 24)
        let frame = MenuPlacement.frame(size: size, below: anchor, on: second)
        #expect(frame.minX == 3500)
        #expect(frame.maxY == 1216 - MenuPlacement.gap)
    }
}
