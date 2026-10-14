import Foundation

/// Where the service keeps its files. Mirrors `crates/core/src/paths.rs`.
public enum VigiliaPaths {
    /// Puts everything in one directory instead (tests, dev builds).
    public static let homeEnv = "VIGILIA_HOME"
    /// Darwin notification the service posts on every session change.
    public static let sessionChangedNotification = "com.vigilia.session-changed"
    /// Label of the socket-activated LaunchAgent that `vigilia install` writes.
    public static let serviceLabel = "com.vigilia.service"
    /// Present once lid-closed sessions are set up.
    public static let sudoersFile = URL(fileURLWithPath: "/etc/sudoers.d/vigilia")

    public static func dataDirectory(
        environment: [String: String] = ProcessInfo.processInfo.environment,
        home: URL = URL(fileURLWithPath: NSHomeDirectory())
    ) -> URL {
        if let dir = environment[homeEnv], !dir.isEmpty {
            return URL(fileURLWithPath: dir)
        }
        return home.appendingPathComponent("Library/Application Support/vigilia")
    }

    public static func socket(
        environment: [String: String] = ProcessInfo.processInfo.environment,
        home: URL = URL(fileURLWithPath: NSHomeDirectory())
    ) -> URL {
        dataDirectory(environment: environment, home: home).appendingPathComponent("vigilia.sock")
    }

    public static func launchAgentPlist(home: URL = URL(fileURLWithPath: NSHomeDirectory())) -> URL {
        home.appendingPathComponent("Library/LaunchAgents/\(serviceLabel).plist")
    }
}
