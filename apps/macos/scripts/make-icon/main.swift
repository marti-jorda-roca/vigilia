// Draws Resources/AppIcon.iconset from the pixel lantern in Sources/Vigilia/Lantern.swift.
// Built and run by scripts/make-icon.sh; the argument is the .iconset directory to fill.
import AppKit

/// `Lantern.swift` sizes `LanternView` with `Pixel.unit`; the icon doesn't use the view, so
/// a stand-in saves compiling the rest of the app's views.
enum Pixel {
    static let unit: CGFloat = 1
}

/// The lit lantern, centred on a black rounded square that follows the macOS icon grid: an
/// 824-point body inside 1024 with a 185-point corner radius. Each lantern pixel is a whole
/// number of image pixels, so the edges stay sharp at every size.
func icon(size: Int) -> Data {
    let rep = NSBitmapImageRep(
        bitmapDataPlanes: nil, pixelsWide: size, pixelsHigh: size, bitsPerSample: 8, samplesPerPixel: 4,
        hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)!
    let context = NSGraphicsContext(bitmapImageRep: rep)!
    let cg = context.cgContext
    let side = CGFloat(size)
    cg.clear(CGRect(x: 0, y: 0, width: side, height: side))

    let body = side * 824 / 1024
    let inset = (side - body) / 2
    let square = CGRect(x: inset, y: inset, width: body, height: body)
    let radius = body * 185 / 824
    cg.addPath(CGPath(roundedRect: square, cornerWidth: radius, cornerHeight: radius, transform: nil))
    cg.setFillColor(CGColor(gray: 0, alpha: 1))
    cg.fillPath()

    // The lantern takes about two thirds of the square's height.
    let scale = max(1, Int((body * 0.66 / CGFloat(Lantern.height)).rounded(.down)))
    let width = Lantern.width * scale
    let height = Lantern.height * scale
    let left = (size - width) / 2
    let top = (size - height) / 2
    for pixel in Lantern.pixels(state: .lit, frame: CGColor(gray: 1, alpha: 1)) {
        cg.setFillColor(pixel.color)
        // CoreGraphics counts y from the bottom; the art counts rows from the top.
        let y = size - top - (pixel.y + 1) * scale
        cg.fill(CGRect(x: left + pixel.x * scale, y: y, width: scale, height: scale))
    }
    context.flushGraphics()
    return rep.representation(using: .png, properties: [:])!
}

let iconset = URL(fileURLWithPath: CommandLine.arguments[1])
try FileManager.default.createDirectory(at: iconset, withIntermediateDirectories: true)
for points in [16, 32, 128, 256, 512] {
    try icon(size: points).write(to: iconset.appendingPathComponent("icon_\(points)x\(points).png"))
    try icon(size: points * 2).write(to: iconset.appendingPathComponent("icon_\(points)x\(points)@2x.png"))
}
