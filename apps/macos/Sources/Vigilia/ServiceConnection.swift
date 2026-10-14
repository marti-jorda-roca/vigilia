import Foundation
import VigiliaProtocol

/// One connection to the service at a time: connect → hello → status → subscribe. The first
/// two results go to `onConnected`, then events to `onEvent`; a connection that closes without
/// `disconnect()` is reported to `onDrop`.
@MainActor
final class ServiceConnection {
    /// How long a just-spawned service gets to start listening. It normally takes 50 ms.
    private static let spawnTimeout: TimeInterval = 3
    /// Seconds between reconnect attempts after an unexpected drop: about two minutes in all.
    private static let backoff: [UInt64] = [1, 2, 4, 8, 16, 30, 30, 30]

    var onConnected: (HelloResult, Session?) -> Void = { _, _ in }
    var onEvent: (Event) -> Void = { _ in }
    var onDrop: () -> Void = {}

    private(set) var client: VigiliaClient?
    private var connecting: Task<VigiliaClient, Error>?
    private var pump: Task<Void, Never>?

    var isConnected: Bool { client != nil }

    /// The current connection, or a new one; concurrent callers share one attempt. With
    /// `spawnIfAbsent`, starts the helper when nothing is listening. Without it, a missing
    /// service throws `TransportError.serviceAbsent`, which callers read as "no session".
    @discardableResult
    func connect(spawnIfAbsent: Bool) async throws -> VigiliaClient {
        if let client {
            return client
        }
        if let connecting {
            return try await connecting.value
        }
        let attempt = Task { try await self.open(spawnIfAbsent: spawnIfAbsent) }
        connecting = attempt
        defer { connecting = nil }
        return try await attempt.value
    }

    /// Closes the connection on purpose; `onDrop` is not called.
    func disconnect() {
        guard let client else { return }
        self.client = nil
        pump?.cancel()
        pump = nil
        Task { await client.close() }
    }

    /// Retries with backoff after an unexpected drop. Connecting re-activates a socket-activated
    /// service, or spawns the helper. Gives up after about two minutes and returns false.
    func reconnect(while keepTrying: @escaping () -> Bool) async -> Bool {
        for delay in Self.backoff {
            try? await Task.sleep(nanoseconds: delay * 1_000_000_000)
            guard keepTrying() else { return false }
            if (try? await connect(spawnIfAbsent: true)) != nil {
                return true
            }
        }
        return false
    }

    private func open(spawnIfAbsent: Bool) async throws -> VigiliaClient {
        let socket = VigiliaPaths.socket()
        let transport: UnixSocketTransport
        do {
            transport = try await UnixSocketTransport.connect(path: socket.path)
        } catch TransportError.serviceAbsent where spawnIfAbsent {
            guard let helper = Helper.url else { throw ConnectError.noHelper }
            try Self.spawnDaemon(helper)
            transport = try await Self.waitForService(socket)
        }
        let client = VigiliaClient(transport: transport)
        do {
            let hello = try await client.hello()
            let session = try await client.status()
            try await client.subscribe()
            self.client = client
            onConnected(hello, session)
            startPump(client)
            return client
        } catch {
            await client.close()
            throw error
        }
    }

    private func startPump(_ client: VigiliaClient) {
        pump = Task { [weak self] in
            for await event in client.events {
                self?.onEvent(event)
            }
            guard let self, self.client === client else { return }
            self.client = nil
            self.pump = nil
            self.onDrop()
        }
    }

    private static func waitForService(_ socket: URL) async throws -> UnixSocketTransport {
        let deadline = Date() + spawnTimeout
        var wait: UInt64 = 5_000_000
        while true {
            try await Task.sleep(nanoseconds: wait)
            do {
                return try await UnixSocketTransport.connect(path: socket.path)
            } catch TransportError.serviceAbsent where Date() < deadline {
                wait = min(wait * 2, 100_000_000)
            }
        }
    }

    /// Starts `<helper> daemon` the way the CLI does: cwd `/`, stdio on `/dev/null`, and its own
    /// process group so it outlives the app and isn't hit by a Ctrl-C meant for `swift run`. If
    /// two clients race, the service's instance lock makes the second one exit straight away.
    private static func spawnDaemon(_ helper: URL) throws {
        var actions: posix_spawn_file_actions_t?
        posix_spawn_file_actions_init(&actions)
        defer { posix_spawn_file_actions_destroy(&actions) }
        for fd: Int32 in [0, 1, 2] {
            posix_spawn_file_actions_addopen(&actions, fd, "/dev/null", fd == 0 ? O_RDONLY : O_WRONLY, 0)
        }
        posix_spawn_file_actions_addchdir_np(&actions, "/")

        var attributes: posix_spawnattr_t?
        posix_spawnattr_init(&attributes)
        defer { posix_spawnattr_destroy(&attributes) }
        posix_spawnattr_setflags(&attributes, Int16(POSIX_SPAWN_SETPGROUP))
        posix_spawnattr_setpgroup(&attributes, 0)

        var pid: pid_t = 0
        let path = helper.path
        let argv: [UnsafeMutablePointer<CChar>?] = [strdup(path), strdup("daemon"), nil]
        defer { argv.forEach { free($0) } }
        let status = posix_spawn(&pid, path, &actions, &attributes, argv, environ)
        guard status == 0 else {
            throw ConnectError.spawnFailed(String(cString: strerror(status)))
        }
        reapWhenDone(pid)
    }

    /// The service exits on its own once idle; collect it so it doesn't linger as a zombie.
    private static func reapWhenDone(_ pid: pid_t) {
        let source = DispatchSource.makeProcessSource(identifier: pid, eventMask: .exit, queue: .global())
        source.setEventHandler {
            var status: Int32 = 0
            waitpid(pid, &status, 0)
            source.cancel()
        }
        source.resume()
    }
}

enum ConnectError: Error, LocalizedError {
    case noHelper
    case spawnFailed(String)

    var errorDescription: String? {
        switch self {
        case .noHelper:
            "The Vigilia service isn't set up. Move Vigilia to Applications, then set it up in Settings."
        case let .spawnFailed(reason):
            "Could not start the Vigilia service: \(reason)."
        }
    }
}
