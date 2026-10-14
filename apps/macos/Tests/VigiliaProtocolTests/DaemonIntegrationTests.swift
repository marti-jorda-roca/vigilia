import Foundation
import Testing

@testable import VigiliaProtocol

/// The real service, end to end. Runs only when `VIGILIA_BIN` points at a built `vigilia`:
///
///     cargo build -p vigilia
///     VIGILIA_BIN=$PWD/target/debug/vigilia apps/macos/scripts/test.sh
///
/// The session takes a real power assertion and registers the crash-recovery job for a moment,
/// like `cargo test -p vigilia --test binary -- --ignored`. `HOME` is pointed at the temp
/// directory too, so that job's plist doesn't land in the real `~/Library/LaunchAgents`.
@Suite(.enabled(if: ProcessInfo.processInfo.environment["VIGILIA_BIN"] != nil), .serialized)
struct DaemonIntegrationTests {
    let binary = ProcessInfo.processInfo.environment["VIGILIA_BIN"] ?? ""

    @Test func appAndCliSeeTheSameSession() async throws {
        // Short, so the socket path stays under the 104-byte limit.
        let home = URL(fileURLWithPath: "/tmp/vigilia-swift-\(getpid())")
        try FileManager.default.createDirectory(at: home, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: home) }
        let environment = ProcessInfo.processInfo.environment.merging(
            ["VIGILIA_HOME": home.path, "HOME": home.path]) { $1 }

        let daemon = Process()
        daemon.executableURL = URL(fileURLWithPath: binary)
        daemon.arguments = ["daemon"]
        daemon.environment = environment
        daemon.standardOutput = FileHandle.nullDevice
        daemon.standardError = FileHandle.nullDevice
        try daemon.run()
        defer { daemon.terminate() }

        let socket = home.appendingPathComponent("vigilia.sock")
        let client = try await connect(socket)

        let hello = try await client.hello()
        #expect(hello.protocolVersion == protocolVersion)
        #expect(hello.capabilities.keepAwakeIdle)

        let started = try #require(try await client.start(duration: 600))
        #expect(started.state == .active)

        try await client.subscribe()
        var events = client.events.makeAsyncIterator()
        // The first event is the session as of subscribing.
        #expect(await events.next() == .sessionChanged(started))

        let extended = try #require(try await client.extend(seconds: 300))
        #expect(extended.endsAt == started.endsAt.map { $0 + 300 })
        #expect(await events.next() == .sessionChanged(extended))

        let cli = try run(["status", "--json"], environment: environment)
        let seen = try ServerMessage.result(
            SessionResult.self, from: Data(#"{"id":0,"ok":true,"result":"#.utf8) + cli + Data("}".utf8))
        #expect(seen.session?.id == started.id)
        #expect(seen.session?.endsAt == extended.endsAt)

        _ = try await client.stop()
        guard case .sessionChanged(let ended?) = await events.next() else {
            Issue.record("expected the session to end")
            return
        }
        #expect(ended.state == .ended || ended.state == .ending)
        await client.close()
    }

    /// The service listens within milliseconds of starting.
    private func connect(_ socket: URL) async throws -> VigiliaClient {
        let deadline = Date() + 5
        while true {
            do {
                return try await VigiliaClient.connect(socket: socket)
            } catch TransportError.serviceAbsent where Date() < deadline {
                try await Task.sleep(nanoseconds: 20_000_000)
            }
        }
    }

    private func run(_ arguments: [String], environment: [String: String]) throws -> Data {
        let process = Process()
        let stdout = Pipe()
        process.executableURL = URL(fileURLWithPath: binary)
        process.arguments = arguments
        process.environment = environment
        process.standardOutput = stdout
        try process.run()
        let data = stdout.fileHandleForReading.readDataToEndOfFile()
        process.waitUntilExit()
        #expect(process.terminationStatus == 0)
        return data
    }
}
