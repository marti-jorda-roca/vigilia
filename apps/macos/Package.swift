// swift-tools-version:6.0
// 6.0 rather than 5.9 so `swift test` can use Swift Testing, which ships with the Command Line
// Tools (XCTest does not). The code still builds in the Swift 5 language mode.

import PackageDescription

let package = Package(
    name: "Vigilia",
    platforms: [
        .macOS(.v13),  // SMAppService
        .iOS(.v16),  // the protocol library only, for a future iOS client
    ],
    products: [
        .library(name: "VigiliaProtocol", targets: ["VigiliaProtocol"]),
    ],
    targets: [
        .target(name: "VigiliaProtocol"),
        .testTarget(name: "VigiliaProtocolTests", dependencies: ["VigiliaProtocol"]),
    ],
    swiftLanguageModes: [.v5]
)
