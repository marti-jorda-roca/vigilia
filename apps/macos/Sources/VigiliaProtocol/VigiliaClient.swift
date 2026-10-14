import Foundation

public enum ClientError: Error, Equatable, LocalizedError, Sendable {
    /// The connection closed before the reply arrived.
    case disconnected

    public var errorDescription: String? {
        switch self {
        case .disconnected: "the connection to the service closed"
        }
    }
}

/// A connection to the service. Calls may overlap: replies are matched to requests by id, and
/// events from `subscribe` arrive on `events`. The connection stays open until `close()` or until
/// the service hangs up.
public actor VigiliaClient {
    private let transport: Transport
    private var nextID: UInt64 = 0
    private var pending: [UInt64: CheckedContinuation<Data, Error>] = [:]
    private var closed = false

    /// Events after a successful `subscribe`. Finishes when the connection closes.
    public nonisolated let events: AsyncStream<Event>
    private let eventSink: AsyncStream<Event>.Continuation

    public init(transport: Transport) {
        self.transport = transport
        (events, eventSink) = AsyncStream.makeStream(of: Event.self)
        // Runs until the connection closes; `close()` ends it.
        Task { await self.readLoop() }
    }

    #if os(macOS)
        /// Connects to the local socket.
        public static func connect(socket: URL = VigiliaPaths.socket()) async throws -> VigiliaClient {
            VigiliaClient(transport: try await UnixSocketTransport.connect(path: socket.path))
        }
    #endif

    /// Sends `call` and decodes its result. Throws `ErrorBody` when the service refuses it.
    public func call<T: Decodable & Sendable>(_ call: Call, as _: T.Type = T.self) async throws -> T {
        guard !closed else { throw ClientError.disconnected }
        nextID += 1
        let id = nextID
        let line = try Request(id: id, call: call).encodedLine()
        let transport = transport
        let reply = try await withCheckedThrowingContinuation { continuation in
            pending[id] = continuation
            Task {
                do {
                    try await transport.send(line)
                } catch {
                    self.fail(id: id, with: error)
                }
            }
        }
        return try ServerMessage.result(T.self, from: reply)
    }

    /// Closes the connection. Pending calls fail with `ClientError.disconnected`.
    public func close() {
        transport.close()
        finish()
    }

    // MARK: Typed calls

    public func hello() async throws -> HelloResult {
        try await call(.hello)
    }

    public func status() async throws -> Session? {
        try await call(.sessionStatus, as: SessionResult.self).session
    }

    /// `duration` in seconds; `nil` runs until stopped.
    public func start(duration: Int?, lidClosedAwake: Bool = false) async throws -> Session? {
        try await call(
            .sessionStart(duration: duration, lidClosedAwake: lidClosedAwake),
            as: SessionResult.self
        ).session
    }

    public func extend(seconds: Int) async throws -> Session? {
        try await call(.sessionExtend(seconds: seconds), as: SessionResult.self).session
    }

    public func stop() async throws -> Session? {
        try await call(.sessionStop, as: SessionResult.self).session
    }

    /// After this returns, matching events arrive on `events`, starting with the current session.
    public func subscribe(_ filter: [EventKind] = []) async throws {
        _ = try await call(.subscribe(filter), as: Empty.self)
    }

    public func configGet() async throws -> Settings {
        try await call(.configGet, as: ConfigResult.self).config
    }

    public func configSet(_ patch: RulesPatch) async throws -> Settings {
        try await call(.configSet(patch), as: ConfigResult.self).config
    }

    // MARK: Reading

    private func readLoop() async {
        var framer = LineFramer()
        do {
            for try await chunk in transport.chunks {
                for line in try framer.push(chunk) {
                    handle(line)
                }
            }
        } catch {
            transport.close()
        }
        finish()
    }

    private func handle(_ line: Data) {
        // A line that isn't JSON at all is skipped; the connection stays usable.
        guard let message = try? ServerMessage.decode(line) else { return }
        switch message {
        case let .event(event):
            eventSink.yield(event)
        case let .response(id?, line):
            pending.removeValue(forKey: id)?.resume(returning: line)
        case let .response(nil, line):
            // The service couldn't read a request at all; there is no telling which one.
            let error: Error
            do {
                _ = try ServerMessage.result(Empty.self, from: line)
                error = ProtocolError("response without an id")
            } catch let refused {
                error = refused
            }
            for continuation in pending.values {
                continuation.resume(throwing: error)
            }
            pending.removeAll()
        }
    }

    private func fail(id: UInt64, with error: Error) {
        pending.removeValue(forKey: id)?.resume(throwing: error)
    }

    private func finish() {
        guard !closed else { return }
        closed = true
        for continuation in pending.values {
            continuation.resume(throwing: ClientError.disconnected)
        }
        pending.removeAll()
        eventSink.finish()
    }
}
