import Foundation

#if os(macOS)
    import Network
#endif

/// A byte stream to the service. The protocol is the same over every transport.
public protocol Transport: AnyObject, Sendable {
    /// Bytes as they arrive. Finishes when the connection closes.
    var chunks: AsyncThrowingStream<Data, Error> { get }
    func send(_ data: Data) async throws
    func close()
}

public enum TransportError: Error, Equatable, Sendable {
    /// Nothing is listening: no socket file, or a stale one left by a service that exited.
    case serviceAbsent
    case failed(String)
    case closed
}

#if os(macOS)
    /// The local transport: the service's owner-only Unix socket.
    public final class UnixSocketTransport: Transport, @unchecked Sendable {
        public let chunks: AsyncThrowingStream<Data, Error>
        private let sink: AsyncThrowingStream<Data, Error>.Continuation
        private let connection: NWConnection
        private let queue = DispatchQueue(label: "com.vigilia.socket")
        /// Set until the connection is ready or has failed. Only touched on `queue` once
        /// `connection` has started.
        private var connecting: CheckedContinuation<Void, Error>?

        /// Connects to the socket at `path`. Throws `TransportError.serviceAbsent` when nothing
        /// is listening there.
        public static func connect(path: String, timeout: TimeInterval = 3) async throws
            -> UnixSocketTransport
        {
            let transport = UnixSocketTransport(path: path)
            try await transport.start(timeout: timeout)
            return transport
        }

        private init(path: String) {
            (chunks, sink) = AsyncThrowingStream.makeStream(of: Data.self)
            connection = NWConnection(to: .unix(path: path), using: .tcp)
        }

        deinit {
            connection.cancel()
        }

        private func start(timeout: TimeInterval) async throws {
            try await withCheckedThrowingContinuation { (done: CheckedContinuation<Void, Error>) in
                connecting = done
                connection.stateUpdateHandler = { [weak self] state in
                    guard let self else { return }
                    switch state {
                    case .ready:
                        self.settle(.success(()))
                        self.receive()
                    // Network reports a refused or missing socket as `waiting` and would retry
                    // forever; for a local socket that just means the service isn't there.
                    case .waiting(let error), .failed(let error):
                        self.settle(.failure(Self.map(error)))
                        self.sink.finish(throwing: Self.map(error))
                        self.connection.cancel()
                    case .cancelled:
                        self.settle(.failure(TransportError.closed))
                        self.sink.finish()
                    default:
                        break
                    }
                }
                connection.start(queue: queue)
                queue.asyncAfter(deadline: .now() + timeout) { [weak self] in
                    guard let self, self.connecting != nil else { return }
                    self.settle(.failure(TransportError.failed("timed out connecting")))
                    self.connection.cancel()
                }
            }
        }

        /// Resolves `connect` once. Called on `queue`.
        private func settle(_ result: Result<Void, Error>) {
            connecting?.resume(with: result)
            connecting = nil
        }

        private func receive() {
            connection.receive(minimumIncompleteLength: 1, maximumLength: 64 * 1024) {
                [weak self] data, _, isComplete, error in
                guard let self else { return }
                if let data, !data.isEmpty {
                    self.sink.yield(data)
                }
                if let error {
                    self.sink.finish(throwing: Self.map(error))
                    self.connection.cancel()
                } else if isComplete {
                    self.sink.finish()
                    self.connection.cancel()
                } else {
                    self.receive()
                }
            }
        }

        public func send(_ data: Data) async throws {
            try await withCheckedThrowingContinuation { (done: CheckedContinuation<Void, Error>) in
                connection.send(
                    content: data,
                    completion: .contentProcessed { error in
                        if let error {
                            done.resume(throwing: Self.map(error))
                        } else {
                            done.resume()
                        }
                    })
            }
        }

        public func close() {
            connection.cancel()
        }

        private static func map(_ error: NWError) -> TransportError {
            if case .posix(let code) = error, code == .ENOENT || code == .ECONNREFUSED {
                return .serviceAbsent
            }
            return .failed(error.localizedDescription)
        }
    }
#endif
