// The Vigilia wire protocol, as defined by `crates/protocol` (the source of truth).
//
// One JSON object per line, in both directions. Requests carry an `id` that the response echoes;
// subscribers also receive events, told apart from responses by their `event` key. Within a
// protocol version the service only adds things, so every enum here has a fallback for values a
// newer service may send.

import Foundation

/// The protocol version this client speaks.
public let protocolVersion = 1

// MARK: - Requests

/// Durations travel as strings such as `"1h30m"`, written and read like the Rust
/// `HumanDuration` does: one or more `<number><unit>` groups, units `d`, `h`, `m`, `s`.
public enum HumanDuration {
    private static let units: [Character: Int] = ["d": 86_400, "h": 3_600, "m": 60, "s": 1]

    /// Seconds in `text`, or `nil` when it isn't a duration. Surrounding whitespace and the
    /// case of the units don't matter; `"2h 30m"` (a space inside) is rejected, as in the CLI.
    public static func parse(_ text: String) -> Int? {
        let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return nil }
        var total = 0
        var number = ""
        for c in trimmed {
            if c.isASCII, c.isNumber {
                number.append(c)
                continue
            }
            guard let unit = units[Character(c.lowercased())], let n = Int(number) else { return nil }
            let (part, overflowA) = n.multipliedReportingOverflow(by: unit)
            let (sum, overflowB) = total.addingReportingOverflow(part)
            guard !overflowA, !overflowB else { return nil }
            total = sum
            number = ""
        }
        return number.isEmpty ? total : nil
    }

    public static func format(seconds: Int) -> String {
        var rest = max(seconds, 0)
        if rest == 0 { return "0s" }
        var out = ""
        for (unit, secs) in [("d", 86_400), ("h", 3_600), ("m", 60), ("s", 1)] where rest >= secs {
            out += "\(rest / secs)\(unit)"
            rest %= secs
        }
        return out
    }
}

public enum EventKind: String, Codable, CaseIterable, Sendable {
    case sessionChanged = "session_changed"
    case lidChanged = "lid_changed"
    case powerChanged = "power_changed"
    case warning
    case needsAttention = "needs_attention"
    case configChanged = "config_changed"
}

/// A change to the `[rules]` settings: only the fields that are set change.
public struct RulesPatch: Encodable, Equatable, Sendable {
    public var batteryFloorPercent: Int?
    public var endAtCriticalHeat: Bool?

    public init(batteryFloorPercent: Int? = nil, endAtCriticalHeat: Bool? = nil) {
        self.batteryFloorPercent = batteryFloorPercent
        self.endAtCriticalHeat = endAtCriticalHeat
    }

    enum CodingKeys: String, CodingKey, CaseIterable {
        case batteryFloorPercent = "battery_floor_percent"
        case endAtCriticalHeat = "end_at_critical_heat"
    }
}

/// A method and its parameters. `debug.simulate_lid` is left out: it is admin-only tooling.
public enum Call: Equatable, Sendable {
    case hello
    /// `duration` in seconds; `nil` runs until `session.stop`.
    case sessionStart(duration: Int?, lidClosedAwake: Bool)
    case sessionExtend(seconds: Int)
    case sessionStop
    case sessionStatus
    /// Empty means every kind.
    case subscribe([EventKind])
    case configGet
    case configSet(RulesPatch)

    public var method: String {
        switch self {
        case .hello: "hello"
        case .sessionStart: "session.start"
        case .sessionExtend: "session.extend"
        case .sessionStop: "session.stop"
        case .sessionStatus: "session.status"
        case .subscribe: "subscribe"
        case .configGet: "config.get"
        case .configSet: "config.set"
        }
    }

    /// Every method this client can call.
    public static let methods = [
        "hello", "session.start", "session.extend", "session.stop", "session.status",
        "subscribe", "config.get", "config.set",
    ]
}

public struct Request: Encodable, Sendable {
    public let id: UInt64
    public let call: Call

    public init(id: UInt64, call: Call) {
        self.id = id
        self.call = call
    }

    /// The request as one line of the wire format, newline included.
    public func encodedLine() throws -> Data {
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.sortedKeys, .withoutEscapingSlashes]
        var line = try encoder.encode(self)
        line.append(0x0A)
        return line
    }

    private enum Keys: String, CodingKey {
        case id, v, method, params
        case duration, lidClosedAwake = "lid_closed_awake", filter, rules
    }

    public func encode(to encoder: Encoder) throws {
        var c = encoder.container(keyedBy: Keys.self)
        try c.encode(id, forKey: .id)
        try c.encode(protocolVersion, forKey: .v)
        try c.encode(call.method, forKey: .method)
        var params = c.nestedContainer(keyedBy: Keys.self, forKey: .params)
        switch call {
        case .hello, .sessionStop, .sessionStatus, .configGet:
            break
        case let .sessionStart(duration, lidClosedAwake):
            if let duration {
                try params.encode(HumanDuration.format(seconds: duration), forKey: .duration)
            }
            try params.encode(lidClosedAwake, forKey: .lidClosedAwake)
        case let .sessionExtend(seconds):
            try params.encode(HumanDuration.format(seconds: seconds), forKey: .duration)
        case let .subscribe(filter):
            try params.encode(filter, forKey: .filter)
        case let .configSet(patch):
            try params.encode(patch, forKey: .rules)
        }
    }
}

// MARK: - Values

public enum SessionState: String, Decodable, CaseIterable, Sendable {
    case active = "Active"
    /// Undo steps are running.
    case ending = "Ending"
    case ended = "Ended"
    /// An undo step failed. `session.stop` retries it.
    case needsAttention = "NeedsAttention"
    /// A state added by a newer service.
    case unknown

    public init(from decoder: Decoder) throws {
        let raw = try decoder.singleValueContainer().decode(String.self)
        self = SessionState(rawValue: raw) ?? .unknown
    }
}

public struct Session: Decodable, Equatable, Sendable {
    public var id: String
    public var startedAt: Date
    /// `nil` when the session runs until `session.stop`.
    public var endsAt: Date?
    public var state: SessionState
    public var lidClosedAwake: Bool

    public init(
        id: String, startedAt: Date, endsAt: Date?, state: SessionState,
        lidClosedAwake: Bool = false
    ) {
        self.id = id
        self.startedAt = startedAt
        self.endsAt = endsAt
        self.state = state
        self.lidClosedAwake = lidClosedAwake
    }

    enum CodingKeys: String, CodingKey, CaseIterable {
        case id
        case startedAt = "started_at"
        case endsAt = "ends_at"
        case state
        case lidClosedAwake = "lid_closed_awake"
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        id = try c.decode(String.self, forKey: .id)
        startedAt = try c.decode(Date.self, forKey: .startedAt)
        endsAt = try c.decodeIfPresent(Date.self, forKey: .endsAt)
        state = try c.decode(SessionState.self, forKey: .state)
        lidClosedAwake = try c.decodeIfPresent(Bool.self, forKey: .lidClosedAwake) ?? false
    }
}

/// What this OS and machine support. Missing flags are false, unknown ones are ignored.
public struct Capabilities: Codable, Equatable, Sendable {
    public var keepAwakeIdle = false
    public var keepAwakeLidClosed = false
    public var lidEvents = false
    public var powerEvents = false
    public var backstop = false
    public var socketActivation = false

    public init() {}

    enum CodingKeys: String, CodingKey, CaseIterable {
        case keepAwakeIdle = "keep_awake_idle"
        case keepAwakeLidClosed = "keep_awake_lid_closed"
        case lidEvents = "lid_events"
        case powerEvents = "power_events"
        case backstop
        case socketActivation = "socket_activation"
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        func flag(_ key: CodingKeys) throws -> Bool {
            try c.decodeIfPresent(Bool.self, forKey: key) ?? false
        }
        keepAwakeIdle = try flag(.keepAwakeIdle)
        keepAwakeLidClosed = try flag(.keepAwakeLidClosed)
        lidEvents = try flag(.lidEvents)
        powerEvents = try flag(.powerEvents)
        backstop = try flag(.backstop)
        socketActivation = try flag(.socketActivation)
    }
}

public struct HelloResult: Decodable, Equatable, Sendable {
    public var protocolVersion: Int
    public var serverVersion: String
    public var capabilities: Capabilities
    /// The scopes this connection holds, such as `session.read`.
    public var scopes: [String]

    enum CodingKeys: String, CodingKey, CaseIterable {
        case protocolVersion = "protocol_version"
        case serverVersion = "server_version"
        case capabilities
        case scopes
    }
}

/// Result of every `session.*` method.
public struct SessionResult: Decodable, Equatable, Sendable {
    public var session: Session?

    enum CodingKeys: String, CodingKey, CaseIterable {
        case session
    }
}

public struct Empty: Decodable, Equatable, Sendable {}

public struct RuleSettings: Codable, Equatable, Sendable {
    public var batteryFloorPercent: Int
    public var endAtCriticalHeat: Bool

    public init(batteryFloorPercent: Int, endAtCriticalHeat: Bool) {
        self.batteryFloorPercent = batteryFloorPercent
        self.endAtCriticalHeat = endAtCriticalHeat
    }

    enum CodingKeys: String, CodingKey, CaseIterable {
        case batteryFloorPercent = "battery_floor_percent"
        case endAtCriticalHeat = "end_at_critical_heat"
    }
}

/// The settings returned by `config.get` and `config.set`.
public struct Settings: Codable, Equatable, Sendable {
    public var rules: RuleSettings

    public init(rules: RuleSettings) {
        self.rules = rules
    }

    enum CodingKeys: String, CodingKey, CaseIterable {
        case rules
    }
}

public struct ConfigResult: Decodable, Equatable, Sendable {
    public var config: Settings

    enum CodingKeys: String, CodingKey, CaseIterable {
        case config
    }
}

public enum LidState: String, Decodable, CaseIterable, Sendable {
    case open, closed
}

public enum ThermalLevel: String, Decodable, CaseIterable, Sendable {
    case nominal, fair, serious, critical
}

public struct PowerSnapshot: Decodable, Equatable, Sendable {
    /// `nil` on a machine without a battery.
    public var batteryPercent: Int?
    public var onAc: Bool
    public var thermal: ThermalLevel

    enum CodingKeys: String, CodingKey, CaseIterable {
        case batteryPercent = "battery_percent"
        case onAc = "on_ac"
        case thermal
    }
}

// MARK: - Events

public enum Event: Equatable, Sendable {
    /// The session started, was extended, or ended. `nil` when there is none.
    case sessionChanged(Session?)
    case lidChanged(LidState)
    case powerChanged(PowerSnapshot)
    /// Something to show that doesn't stop the session.
    case warning(kind: String, detail: String)
    /// An undo step failed; the machine may not sleep normally until `session.stop` succeeds.
    case needsAttention(detail: String)
    case configChanged(Settings)
    /// An event added by a newer service, or one this client could not read.
    case unknown(String)
}

extension Event: Decodable {
    private enum Keys: String, CodingKey {
        case event, session, state, power, kind, detail, config
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: Keys.self)
        let tag = try c.decode(String.self, forKey: .event)
        switch EventKind(rawValue: tag) {
        case .sessionChanged:
            self = .sessionChanged(try c.decodeIfPresent(Session.self, forKey: .session))
        case .lidChanged:
            self = .lidChanged(try c.decode(LidState.self, forKey: .state))
        case .powerChanged:
            self = .powerChanged(try c.decode(PowerSnapshot.self, forKey: .power))
        case .warning:
            self = .warning(
                kind: try c.decode(String.self, forKey: .kind),
                detail: try c.decode(String.self, forKey: .detail))
        case .needsAttention:
            self = .needsAttention(detail: try c.decode(String.self, forKey: .detail))
        case .configChanged:
            self = .configChanged(try c.decode(Settings.self, forKey: .config))
        case nil:
            self = .unknown(tag)
        }
    }
}

// MARK: - Responses

public enum ErrorCode: String, Decodable, CaseIterable, Sendable {
    case badRequest = "bad_request"
    case unsupportedVersion = "unsupported_version"
    case forbidden
    case noSession = "no_session"
    case sessionActive = "session_active"
    case unsupported
    case `internal`
    /// A code added by a newer service. Treat it like `internal`.
    case unknown

    public init(from decoder: Decoder) throws {
        let raw = try decoder.singleValueContainer().decode(String.self)
        self = ErrorCode(rawValue: raw) ?? .unknown
    }
}

/// An error the service answered with.
public struct ErrorBody: Decodable, Equatable, Error, LocalizedError, Sendable {
    public var code: ErrorCode
    public var message: String

    public init(code: ErrorCode, message: String) {
        self.code = code
        self.message = message
    }

    public var errorDescription: String? { message }
}

/// The reply did not have the expected shape.
public struct ProtocolError: Error, Equatable, LocalizedError, Sendable {
    public var message: String

    public init(_ message: String) {
        self.message = message
    }

    public var errorDescription: String? { message }
}

/// One line from the service.
public enum ServerMessage: Sendable {
    case event(Event)
    /// The raw line is kept so the caller can decode the result type it expects for the method
    /// it called. `id` is `nil` when the request was too malformed to have one.
    case response(id: UInt64?, line: Data)

    private struct Probe: Decodable {
        var event: String?
        var id: UInt64?
    }

    private struct Envelope<T: Decodable>: Decodable {
        var ok: Bool
        var result: T?
        var error: ErrorBody?
    }

    public static func decode(_ line: Data) throws -> ServerMessage {
        let decoder = VigiliaJSON.decoder()
        let probe = try decoder.decode(Probe.self, from: line)
        guard let tag = probe.event else {
            return .response(id: probe.id, line: line)
        }
        // An event this client can't read must not break the connection.
        return .event((try? decoder.decode(Event.self, from: line)) ?? .unknown(tag))
    }

    /// The `result` of a response line, or the `ErrorBody` it carries.
    public static func result<T: Decodable>(_: T.Type, from line: Data) throws -> T {
        let envelope: Envelope<T>
        do {
            envelope = try VigiliaJSON.decoder().decode(Envelope<T>.self, from: line)
        } catch {
            throw ProtocolError("unexpected result shape: \(error)")
        }
        guard envelope.ok else {
            throw envelope.error ?? ErrorBody(code: .internal, message: "error response without a body")
        }
        guard let result = envelope.result else {
            throw ProtocolError("response without a result")
        }
        return result
    }
}

// MARK: - JSON

public enum VigiliaJSON {
    public static func decoder() -> JSONDecoder {
        let decoder = JSONDecoder()
        decoder.dateDecodingStrategy = .custom { decoder in
            let c = try decoder.singleValueContainer()
            let raw = try c.decode(String.self)
            guard let date = VigiliaDate.parse(raw) else {
                throw DecodingError.dataCorruptedError(
                    in: c, debugDescription: "not an RFC 3339 timestamp: \(raw)")
            }
            return date
        }
        return decoder
    }
}

/// RFC 3339 timestamps. The service (chrono) writes up to nine fractional digits, which the
/// Foundation ISO 8601 parsers don't all accept, so the fraction is split off and added back.
public enum VigiliaDate {
    public static func parse(_ raw: String) -> Date? {
        var base = raw
        var fraction = 0.0
        if let dot = raw.firstIndex(of: "."), raw[..<dot].contains("T") {
            let digits = raw[raw.index(after: dot)...].prefix(while: \.isNumber)
            guard !digits.isEmpty, let value = Double("0." + digits) else { return nil }
            fraction = value
            base = String(raw[..<dot]) + String(raw[digits.endIndex...])
        }
        guard let whole = try? Date(base, strategy: .iso8601) else { return nil }
        return whole.addingTimeInterval(fraction)
    }
}
