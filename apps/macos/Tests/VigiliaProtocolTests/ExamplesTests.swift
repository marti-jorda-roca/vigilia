import Foundation
import Testing

@testable import VigiliaProtocol

/// Checks the Swift types against `crates/protocol/schema/examples.jsonl`, which the Rust tests
/// generate from the real types.
struct ExamplesTests {
    let lines: [Data]

    init() throws {
        lines = try Fixtures.exampleLines()
    }

    private func lines(where predicate: (NSDictionary) -> Bool) throws -> [Data] {
        try lines.filter { predicate(try Fixtures.object($0)) }
    }

    @Test func requestsEncodeLikeTheRustTypes() throws {
        let requests = try lines(where: { $0["method"] != nil })
        let calls: [Call] = [
            .hello,
            .sessionStart(duration: 7_200, lidClosedAwake: true),
            .sessionExtend(seconds: 1_800),
            .sessionStop,
            .sessionStatus,
            .subscribe([.sessionChanged]),
        ]
        // `debug.simulate_lid` (id 7) is admin tooling this client doesn't send.
        let configCalls: [Call] = [
            .configGet,
            .configSet(RulesPatch(batteryFloorPercent: 20, endAtCriticalHeat: false)),
        ]
        let expected = Array(zip(1..., calls)) + Array(zip(8..., configCalls))
        for (id, call) in expected {
            let wire = try #require(
                requests.first { (try? Fixtures.object($0)["id"] as? Int) == id },
                "no example request with id \(id)")
            let ours = try Request(id: UInt64(id), call: call).encodedLine()
            #expect(try Fixtures.object(ours) == Fixtures.object(wire), "\(call.method)")
        }
        #expect(
            Set(requests.map { try! Fixtures.object($0)["method"] as! String })
                .subtracting(["debug.simulate_lid"]) == Set(Call.methods))
    }

    @Test func responsesDecode() throws {
        let responses = try lines(where: { $0["ok"] != nil })
        #expect(responses.count == 6)

        let hello = try ServerMessage.result(HelloResult.self, from: responses[0])
        #expect(hello.protocolVersion == protocolVersion)
        #expect(hello.capabilities.keepAwakeLidClosed && hello.capabilities.powerEvents)
        #expect(hello.scopes == ["session.read"])

        let session = try #require(try ServerMessage.result(SessionResult.self, from: responses[1]).session)
        #expect(session.state == .active)
        #expect(session.endsAt?.timeIntervalSince(session.startedAt) == 7_200)
        #expect(!session.lidClosedAwake)

        #expect(try ServerMessage.result(SessionResult.self, from: responses[2]).session == nil)
        _ = try ServerMessage.result(Empty.self, from: responses[3])

        let config = try ServerMessage.result(ConfigResult.self, from: responses[4]).config
        #expect(config.rules == RuleSettings(batteryFloorPercent: 10, endAtCriticalHeat: true))

        #expect(throws: ErrorBody(code: .badRequest, message: "bad request")) {
            try ServerMessage.result(SessionResult.self, from: responses[5])
        }
    }

    @Test func eventsDecode() throws {
        let events = try lines(where: { $0["event"] != nil }).map { line -> Event in
            guard case .event(let event) = try ServerMessage.decode(line) else {
                Issue.record("not read as an event")
                return .unknown("")
            }
            return event
        }
        #expect(events.count == 7)
        for event in events {
            if case .unknown = event { Issue.record("an example event was not understood") }
        }
        guard case .sessionChanged(let session?) = events[0] else {
            Issue.record("expected a session")
            return
        }
        #expect(session.state == .active)
        #expect(events[1] == .sessionChanged(nil))
        #expect(events[2] == .lidChanged(.closed))
        #expect(events[3] == .powerChanged(PowerSnapshot(batteryPercent: 80, onAc: false, thermal: .nominal)))
        #expect(events[4] == .warning(kind: "rule", detail: "battery at 9% is below the 10% floor"))
        guard case .needsAttention = events[5] else {
            Issue.record("expected needs_attention")
            return
        }
        #expect(
            events[6] == .configChanged(Settings(rules: RuleSettings(batteryFloorPercent: 10, endAtCriticalHeat: true)))
        )
    }

    @Test func responsesAreToldApartFromEvents() throws {
        guard
            case .response(let id, _) = try ServerMessage.decode(
                Data(#"{"id":3,"ok":true,"result":{"session":null}}"#.utf8))
        else {
            Issue.record("expected a response")
            return
        }
        #expect(id == 3)
        guard
            case .response(let noID, _) = try ServerMessage.decode(
                Data(#"{"id":null,"ok":false,"error":{"code":"bad_request","message":"x"}}"#.utf8))
        else {
            Issue.record("expected a response")
            return
        }
        #expect(noID == nil)
    }

    // MARK: Values a newer service may send

    @Test func unknownEventsAreKept() throws {
        guard case .event(let event) = try ServerMessage.decode(Data(#"{"event":"agent_said","text":"hi"}"#.utf8))
        else {
            Issue.record("expected an event")
            return
        }
        #expect(event == .unknown("agent_said"))
        // A known tag with a payload this client can't read must not fail the connection.
        guard case .event(let odd) = try ServerMessage.decode(Data(#"{"event":"lid_changed","state":"ajar"}"#.utf8))
        else {
            Issue.record("expected an event")
            return
        }
        #expect(odd == .unknown("lid_changed"))
    }

    @Test func unknownErrorCodesAndStates() throws {
        let line = Data(#"{"id":1,"ok":false,"error":{"code":"rate_limited","message":"slow down"}}"#.utf8)
        #expect(throws: ErrorBody(code: .unknown, message: "slow down")) {
            try ServerMessage.result(Empty.self, from: line)
        }
        let session = try VigiliaJSON.decoder().decode(
            Session.self,
            from: Data(#"{"id":"s","started_at":"2026-10-12T00:00:00Z","ends_at":null,"state":"Paused"}"#.utf8))
        #expect(session.state == .unknown)
        #expect(session.endsAt == nil)
    }

    @Test func unknownCapabilitiesAreIgnoredAndMissingOnesDefault() throws {
        let caps = try JSONDecoder().decode(
            Capabilities.self, from: Data(#"{"keep_awake_idle":true,"mute":true}"#.utf8))
        #expect(caps.keepAwakeIdle)
        #expect(!caps.keepAwakeLidClosed)
    }

    @Test func timestampsWithNanoseconds() throws {
        // chrono writes `Utc::now()` with up to nine fractional digits.
        let plain = try #require(VigiliaDate.parse("2026-10-12T00:00:00Z"))
        let nanos = try #require(VigiliaDate.parse("2026-10-12T00:00:00.123456789Z"))
        let millis = try #require(VigiliaDate.parse("2026-10-12T00:00:00.5Z"))
        #expect(abs(nanos.timeIntervalSince(plain) - 0.123456789) < 1e-6)
        #expect(millis.timeIntervalSince(plain) == 0.5)
        #expect(VigiliaDate.parse("yesterday") == nil)
    }

    @Test func durationsParseLikeTheCli() {
        // The cases from crates/protocol/src/duration.rs.
        #expect(HumanDuration.parse("2h") == 7_200)
        #expect(HumanDuration.parse("30m") == 1_800)
        #expect(HumanDuration.parse("90s") == 90)
        #expect(HumanDuration.parse("1d2h30m") == 86_400 + 7_200 + 1_800)
        #expect(HumanDuration.parse(" 2H ") == 7_200)
        #expect(HumanDuration.parse("1h1h") == 7_200)
        for bad in ["", "h", "10", "2x", "2h 30m", "99999999999999999999d", "-1h", "1.5h"] {
            #expect(HumanDuration.parse(bad) == nil, "\(bad)")
        }
    }

    @Test func durationsMatchTheRustFormat() {
        #expect(HumanDuration.format(seconds: 0) == "0s")
        #expect(HumanDuration.format(seconds: 5_400) == "1h30m")
        #expect(HumanDuration.format(seconds: 90_061) == "1d1h1m1s")
    }
}
