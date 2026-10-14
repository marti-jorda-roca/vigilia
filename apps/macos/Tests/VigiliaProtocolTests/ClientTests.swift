import Foundation
import Testing

@testable import VigiliaProtocol

/// A transport whose far end is the test: it reads what the client sent and writes replies.
final class MemoryTransport: Transport, @unchecked Sendable {
    let chunks: AsyncThrowingStream<Data, Error>
    private let toClient: AsyncThrowingStream<Data, Error>.Continuation
    let sent: AsyncStream<Data>
    private let sentSink: AsyncStream<Data>.Continuation

    init() {
        (chunks, toClient) = AsyncThrowingStream.makeStream(of: Data.self)
        (sent, sentSink) = AsyncStream.makeStream(of: Data.self)
    }

    func send(_ data: Data) async throws {
        sentSink.yield(data)
    }

    func close() {
        toClient.finish()
        sentSink.finish()
    }

    func reply(_ line: String) {
        toClient.yield(Data((line + "\n").utf8))
    }

    func hangUp() {
        toClient.finish()
    }
}

/// Reads the requests the client sends, in arrival order.
actor RequestReader {
    private var iterator: AsyncStream<Data>.Iterator

    init(_ transport: MemoryTransport) {
        iterator = transport.sent.makeAsyncIterator()
    }

    func next() async throws -> (id: Int, method: String) {
        var copy = iterator
        let next = await copy.next()
        iterator = copy
        let line = try #require(next)
        let object = try Fixtures.object(line)
        return (object["id"] as! Int, object["method"] as! String)
    }
}

struct ClientTests {
    let session = #"{"id":"s1","started_at":"2026-10-12T00:00:00Z","ends_at":"2026-10-12T02:00:00Z","state":"Active"}"#

    @Test func matchesResponsesById() async throws {
        let transport = MemoryTransport()
        let client = VigiliaClient(transport: transport)
        let requests = RequestReader(transport)

        async let status = client.status()
        async let config = client.configGet()
        let first = try await requests.next()
        let second = try await requests.next()
        let byMethod = Dictionary(uniqueKeysWithValues: [first, second].map { ($0.method, $0.id) })

        // Answer in the opposite order.
        transport.reply(#"{"id":\#(byMethod["config.get"]!),"ok":true,"result":{"config":{"rules":{"battery_floor_percent":15,"end_at_critical_heat":false}}}}"#)
        transport.reply(#"{"id":\#(byMethod["session.status"]!),"ok":true,"result":{"session":\#(session)}}"#)

        #expect(try await status?.id == "s1")
        #expect(try await config.rules.batteryFloorPercent == 15)
        await client.close()
    }

    @Test func eventsArriveBetweenResponses() async throws {
        let transport = MemoryTransport()
        let client = VigiliaClient(transport: transport)
        let requests = RequestReader(transport)

        async let subscribed: Void = client.subscribe()
        let request = try await requests.next()
        #expect(request.method == "subscribe")
        transport.reply(#"{"id":\#(request.id),"ok":true,"result":{}}"#)
        try await subscribed
        transport.reply(#"{"event":"session_changed","session":\#(session)}"#)
        transport.reply(#"{"event":"lid_changed","state":"open"}"#)
        transport.hangUp()

        var events: [Event] = []
        for await event in client.events {
            events.append(event)
        }
        #expect(events.count == 2)
        #expect(events.last == .lidChanged(.open))
    }

    @Test func errorResponsesThrowTheirBody() async throws {
        let transport = MemoryTransport()
        let client = VigiliaClient(transport: transport)
        let requests = RequestReader(transport)

        let extended = Task { try await client.extend(seconds: 900) }
        let request = try await requests.next()
        transport.reply(#"{"id":\#(request.id),"ok":false,"error":{"code":"no_session","message":"no session is running"}}"#)
        await #expect(throws: ErrorBody(code: .noSession, message: "no session is running")) {
            try await extended.value
        }
        await client.close()
    }

    @Test func aDisconnectFailsPendingCallsAndEndsEvents() async throws {
        let transport = MemoryTransport()
        let client = VigiliaClient(transport: transport)
        let requests = RequestReader(transport)

        let hello = Task { try await client.hello() }
        _ = try await requests.next()
        transport.hangUp()
        await #expect(throws: ClientError.disconnected) { try await hello.value }

        for await _ in client.events {
            Issue.record("no events expected")
        }
        await #expect(throws: ClientError.disconnected) { try await client.status() }
    }

    @Test func aServiceThatNeverAnswersTimesOut() async throws {
        let transport = MemoryTransport()
        let client = VigiliaClient(transport: transport, requestTimeout: 0.05)
        await #expect(throws: ClientError.timedOut) { try await client.hello() }
        // The connection is closed: the stream ends and later calls fail straight away.
        for await _ in client.events {
            Issue.record("no events expected")
        }
        await #expect(throws: ClientError.disconnected) { try await client.status() }
    }

    @Test func aResponseWithoutAnIdFailsPendingCalls() async throws {
        let transport = MemoryTransport()
        let client = VigiliaClient(transport: transport)
        let requests = RequestReader(transport)

        let stopped = Task { try await client.stop() }
        _ = try await requests.next()
        transport.reply(#"{"id":null,"ok":false,"error":{"code":"bad_request","message":"request line too long"}}"#)
        await #expect(throws: ErrorBody(code: .badRequest, message: "request line too long")) {
            try await stopped.value
        }
        await client.close()
    }
}
