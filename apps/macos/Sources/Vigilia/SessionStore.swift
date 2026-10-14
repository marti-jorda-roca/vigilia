import Foundation
import VigiliaProtocol

/// What the app shows, and the only place that talks to the service. Holds no session logic:
/// every change goes through a protocol call and comes back as a session or an event.
///
/// The connection is kept only while it is useful (a live session, or the menu or Settings
/// open), so the service can still exit when idle.
@MainActor
final class SessionStore: ObservableObject {
    enum Connection: Equatable {
        case disconnected, connecting, connected, reconnecting
    }

    struct Warning: Identifiable, Equatable {
        let id = UUID()
        let kind: String
        let detail: String
    }

    static let serviceDownWarning = "service"
    private static let capabilitiesKey = "capabilities"

    @Published private(set) var connection: Connection = .disconnected
    /// The running session. `nil` when there is none, including once it has ended.
    @Published private(set) var session: Session?
    /// Cached between launches, so the menu can be drawn without connecting.
    @Published private(set) var capabilities: Capabilities
    @Published private(set) var settings: Settings?
    @Published private(set) var serverVersion: String?
    @Published private(set) var warnings: [Warning] = []
    @Published private(set) var needsAttention: String?
    /// The last call that failed, shown in the menu until the next one.
    @Published private(set) var lastError: String?
    @Published private(set) var busy = false
    @Published private(set) var setup = SetupStatus.detect()
    @Published private(set) var menuOpen = false
    private var settingsOpen = false

    private let service = ServiceConnection()

    init() {
        let cached = UserDefaults.standard.data(forKey: Self.capabilitiesKey)
        capabilities = cached.flatMap { try? JSONDecoder().decode(Capabilities.self, from: $0) } ?? Capabilities()
        service.onConnected = { [weak self] hello, session in self?.connected(hello, session) }
        service.onEvent = { [weak self] event in self?.handle(event) }
        service.onDrop = { [weak self] in self?.dropped() }
    }

    var hasAlert: Bool { needsAttention != nil || !warnings.isEmpty }

    /// The session when it has a timeout and is running: the only time the menu bar shows time.
    var timedSession: Session? {
        guard let session, session.state == .active, session.endsAt != nil else { return nil }
        return session
    }

    // MARK: Lifecycle

    /// Once at launch: learn the session and capabilities, then stay connected only if a
    /// session is running.
    func launched() {
        Task { await refresh(spawn: false) }
    }

    /// The service announced a session change: connect and subscribe; the first event is the
    /// current session.
    func doorbellRang() {
        Task { await refresh(spawn: false) }
    }

    func menuOpened() {
        menuOpen = true
        setup = SetupStatus.detect()
        Task { await refresh(spawn: false) }
    }

    func menuClosed() {
        menuOpen = false
        lastError = nil
        reevaluate()
    }

    func settingsOpened() {
        settingsOpen = true
        setup = SetupStatus.detect()
        Task {
            await refresh(spawn: true)
            await perform { client in
                self.settings = try await client.configGet()
            }
        }
    }

    func settingsClosed() {
        settingsOpen = false
        reevaluate()
    }

    /// Closes the socket. The session, if any, carries on.
    func disconnect() {
        service.disconnect()
        connection = .disconnected
    }

    // MARK: Intents

    /// `duration` in seconds; `nil` runs until stopped.
    func start(duration: Int?, lidClosedAwake: Bool) {
        warnings.removeAll { $0.kind != Self.serviceDownWarning }
        Task {
            await perform { client in
                self.apply(try await client.start(duration: duration, lidClosedAwake: lidClosedAwake))
            }
        }
    }

    func extend(seconds: Int) {
        Task {
            await perform { client in
                self.apply(try await client.extend(seconds: seconds))
            }
        }
    }

    /// Ends the session, or retries the undo steps after `needs_attention`.
    func stop() {
        Task {
            await perform { client in
                self.apply(try await client.stop())
            }
        }
    }

    func setBatteryFloor(_ percent: Int) {
        Task {
            await perform { client in
                self.settings = try await client.configSet(RulesPatch(batteryFloorPercent: percent))
            }
        }
    }

    func setEndAtCriticalHeat(_ on: Bool) {
        Task {
            await perform { client in
                self.settings = try await client.configSet(RulesPatch(endAtCriticalHeat: on))
            }
        }
    }

    func dismiss(_ warning: Warning) {
        warnings.removeAll { $0.id == warning.id }
    }

    // MARK: Connection

    private var sessionIsLive: Bool {
        guard let session else { return false }
        return session.state != .ended
    }

    /// Connects without starting the service: if it isn't running, there is no session.
    private func refresh(spawn: Bool) async {
        guard !service.isConnected, connection != .reconnecting else { return }
        connection = .connecting
        do {
            try await service.connect(spawnIfAbsent: spawn)
        } catch {
            connection = .disconnected
            if case TransportError.serviceAbsent = error {
                apply(nil)
            } else if spawn {
                lastError = Self.describe(error)
            }
        }
        reevaluate()
    }

    /// Runs a user action, starting the service first if needed.
    private func perform(_ action: @escaping (VigiliaClient) async throws -> Void) async {
        busy = true
        defer { busy = false }
        do {
            if !service.isConnected { connection = .connecting }
            let client = try await service.connect(spawnIfAbsent: true)
            try await action(client)
            lastError = nil
        } catch {
            if !service.isConnected { connection = .disconnected }
            lastError = Self.describe(error)
        }
        reevaluate()
    }

    /// Drops the connection when nothing needs it.
    private func reevaluate() {
        guard service.isConnected else { return }
        if !(sessionIsLive || menuOpen || settingsOpen) {
            disconnect()
        }
    }

    private func connected(_ hello: HelloResult, _ session: Session?) {
        connection = .connected
        serverVersion = hello.serverVersion
        if hello.capabilities != capabilities {
            capabilities = hello.capabilities
            if let data = try? JSONEncoder().encode(hello.capabilities) {
                UserDefaults.standard.set(data, forKey: Self.capabilitiesKey)
            }
        }
        warnings.removeAll { $0.kind == Self.serviceDownWarning }
        apply(session)
    }

    private func dropped() {
        connection = .disconnected
        guard sessionIsLive else {
            reevaluate()
            return
        }
        connection = .reconnecting
        warnings.removeAll { $0.kind == Self.serviceDownWarning }
        warnings.append(Warning(kind: Self.serviceDownWarning, detail: "Service not running, reconnecting"))
        Task {
            let back = await service.reconnect { [weak self] in self?.connection == .reconnecting }
            guard connection == .reconnecting else { return }
            if back {
                connection = .connected
            } else {
                // Give up and wait for the doorbell. Without the service there is no session:
                // the next service start replays the journal and undoes it.
                connection = .disconnected
                apply(nil)
                warnings.removeAll { $0.kind == Self.serviceDownWarning }
                warnings.append(Warning(kind: Self.serviceDownWarning, detail: "Service not running"))
            }
            reevaluate()
        }
    }

    // MARK: State

    private func handle(_ event: Event) {
        switch event {
        case let .sessionChanged(session):
            apply(session)
        case let .warning(kind, detail):
            guard !warnings.contains(where: { $0.detail == detail }) else { break }
            warnings.append(Warning(kind: kind, detail: detail))
            if warnings.count > 5 { warnings.removeFirst() }
        case let .needsAttention(detail):
            needsAttention = detail
        case let .configChanged(config):
            settings = config
        case .lidChanged, .powerChanged, .unknown:
            break
        }
        reevaluate()
    }

    private func apply(_ new: Session?) {
        let live = new.flatMap { $0.state == .ended ? nil : $0 }
        if live?.state != .needsAttention {
            needsAttention = nil
        } else if needsAttention == nil {
            needsAttention = "Normal sleep could not be restored."
        }
        if session != live {
            session = live
        }
    }

    private static func describe(_ error: Error) -> String {
        switch error {
        case TransportError.serviceAbsent, ClientError.disconnected:
            "The Vigilia service isn't running."
        case let TransportError.failed(reason):
            "Could not reach the Vigilia service: \(reason)"
        default:
            error.localizedDescription
        }
    }
}
