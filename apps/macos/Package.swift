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
        .executable(name: "Vigilia", targets: ["Vigilia"]),
    ],
    targets: [
        .target(name: "VigiliaProtocol"),
        // The menu bar app. Bundling (Info.plist, the helper binary, signing) is done by
        // scripts/build-app.sh; `swift run Vigilia` works without a bundle for development.
        .executableTarget(name: "Vigilia", dependencies: ["VigiliaProtocol"]),
        .testTarget(name: "VigiliaProtocolTests", dependencies: ["VigiliaProtocol"]),
        .testTarget(name: "VigiliaTests", dependencies: ["Vigilia"]),
    ],
    swiftLanguageModes: [.v5]
)
