import Foundation
import Testing

/// Files from the Rust side of the repo, which defines the protocol.
enum Fixtures {
    static let repoRoot = URL(fileURLWithPath: #filePath)
        .deletingLastPathComponent()  // VigiliaProtocolTests
        .deletingLastPathComponent()  // Tests
        .deletingLastPathComponent()  // macos
        .deletingLastPathComponent()  // apps
        .deletingLastPathComponent()

    static let schemaDir = repoRoot.appendingPathComponent("crates/protocol/schema")

    /// Every line of `examples.jsonl`.
    static func exampleLines() throws -> [Data] {
        let text = try String(
            contentsOf: schemaDir.appendingPathComponent("examples.jsonl"), encoding: .utf8)
        return text.split(separator: "\n").map { Data($0.utf8) }
    }

    static func schema() throws -> [String: Any] {
        let data = try Data(contentsOf: schemaDir.appendingPathComponent("protocol.schema.json"))
        return try #require(try JSONSerialization.jsonObject(with: data) as? [String: Any])
    }

    static func object(_ line: Data) throws -> NSDictionary {
        try #require(try JSONSerialization.jsonObject(with: line) as? NSDictionary)
    }
}
