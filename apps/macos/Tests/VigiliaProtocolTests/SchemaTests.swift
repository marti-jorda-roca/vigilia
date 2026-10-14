import Foundation
import Testing

@testable import VigiliaProtocol

/// Property names and enum values in `protocol.schema.json` must match the Swift types, so a
/// field renamed or added on the Rust side fails here rather than in the app.
struct SchemaTests {
    let defs: [String: Any]

    init() throws {
        defs = try #require(try Fixtures.schema()["$defs"] as? [String: Any])
    }

    private func properties(_ name: String) -> Set<String> {
        let def = defs[name] as? [String: Any]
        return Set((def?["properties"] as? [String: Any] ?? [:]).keys)
    }

    /// Values of a string enum, written either as `enum` or as a `oneOf` of `enum`/`const`.
    private func values(_ def: Any?) -> Set<String> {
        guard let def = def as? [String: Any] else { return [] }
        var out = Set(def["enum"] as? [String] ?? [])
        if let value = def["const"] as? String { out.insert(value) }
        for branch in def["oneOf"] as? [Any] ?? [] {
            out.formUnion(values(branch))
        }
        return out
    }

    private func keys<K: CodingKey & CaseIterable>(_: K.Type) -> Set<String> {
        Set(K.allCases.map(\.stringValue))
    }

    @Test func structFields() {
        #expect(properties("Session") == keys(Session.CodingKeys.self))
        #expect(properties("Capabilities") == keys(Capabilities.CodingKeys.self))
        #expect(properties("HelloResult") == keys(HelloResult.CodingKeys.self))
        #expect(properties("SessionResult") == keys(SessionResult.CodingKeys.self))
        #expect(properties("RuleSettings") == keys(RuleSettings.CodingKeys.self))
        #expect(properties("Settings") == keys(Settings.CodingKeys.self))
        #expect(properties("ConfigResult") == keys(ConfigResult.CodingKeys.self))
        #expect(properties("PowerSnapshot") == keys(PowerSnapshot.CodingKeys.self))
        #expect(properties("RulesPatch") == keys(RulesPatch.CodingKeys.self))
    }

    @Test func enumValues() {
        func known<E: RawRepresentable & CaseIterable>(_: E.Type) -> Set<String> where E.RawValue == String {
            Set(E.allCases.map(\.rawValue))
        }
        #expect(values(defs["SessionState"]) == known(SessionState.self).subtracting(["unknown"]))
        // The Rust `ErrorCode` has its own `unknown` fallback, so it is in the schema too.
        #expect(values(defs["ErrorCode"]) == known(ErrorCode.self))
        #expect(values(defs["EventKind"]) == known(EventKind.self))
        #expect(values(defs["ThermalLevel"]) == known(ThermalLevel.self))
        #expect(values(defs["LidState"]) == known(LidState.self))
    }

    @Test func eventTags() {
        let branches = (defs["Event"] as? [String: Any])?["oneOf"] as? [[String: Any]] ?? []
        let tags = branches.compactMap { branch in
            ((branch["properties"] as? [String: Any])?["event"] as? [String: Any])?["const"] as? String
        }
        #expect(Set(tags) == Set(EventKind.allCases.map(\.rawValue)))
    }

    @Test func methods() {
        let branches = (defs["Request"] as? [String: Any])?["oneOf"] as? [[String: Any]] ?? []
        let methods = branches.compactMap { branch in
            ((branch["properties"] as? [String: Any])?["method"] as? [String: Any])?["const"] as? String
        }
        #expect(Set(methods).subtracting(["debug.simulate_lid"]) == Set(Call.methods))
    }

    @Test func requestParams() {
        #expect(properties("SessionStartParams") == ["duration", "lid_closed_awake"])
        #expect(properties("SessionExtendParams") == ["duration"])
        #expect(properties("SubscribeParams") == ["filter"])
        #expect(properties("ConfigSetParams") == ["rules"])
    }
}
