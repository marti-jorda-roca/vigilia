import Foundation
import Testing

@testable import VigiliaProtocol

struct PathsTests {
    let home = URL(fileURLWithPath: "/Users/someone")

    @Test func defaultsToApplicationSupport() {
        #expect(VigiliaPaths.socket(environment: [:], home: home).path
            == "/Users/someone/Library/Application Support/vigilia/vigilia.sock")
    }

    @Test func vigiliaHomeOverridesEverything() {
        #expect(VigiliaPaths.socket(environment: ["VIGILIA_HOME": "/tmp/v"], home: home).path
            == "/tmp/v/vigilia.sock")
        // Empty is ignored, as in `crates/core/src/paths.rs`.
        #expect(VigiliaPaths.socket(environment: ["VIGILIA_HOME": ""], home: home).path
            == "/Users/someone/Library/Application Support/vigilia/vigilia.sock")
    }

    @Test func launchAgent() {
        #expect(VigiliaPaths.launchAgentPlist(home: home).path
            == "/Users/someone/Library/LaunchAgents/com.vigilia.service.plist")
    }
}
