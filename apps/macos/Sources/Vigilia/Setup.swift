import Foundation
import VigiliaProtocol

/// What `vigilia install` has set up on this Mac. Detection only: running the install, the
/// admin step for the sudoers rule, and repair come with the packaged app (plan Part C).
struct SetupStatus: Equatable {
    /// `ProgramArguments[0]` of the socket-activated LaunchAgent, if it is installed.
    var agentProgram: String?
    /// The helper inside this app's bundle, when running from a bundle that has one.
    var bundledHelper: String?
    /// Whether lid-closed sessions can be used (`/etc/sudoers.d/vigilia` exists).
    var sudoersInstalled: Bool

    var agentInstalled: Bool { agentProgram != nil }

    var agentProgramExists: Bool {
        agentProgram.map(FileManager.default.isExecutableFile(atPath:)) ?? false
    }

    var pointsAtBundledHelper: Bool {
        agentProgram != nil && agentProgram == bundledHelper
    }

    /// The LaunchAgent points at a binary that is gone, or at another copy than this app's.
    var needsRepair: Bool {
        agentInstalled && (!agentProgramExists || (bundledHelper != nil && !pointsAtBundledHelper))
    }

    static func detect() -> SetupStatus {
        let fm = FileManager.default
        var program: String?
        if let data = fm.contents(atPath: VigiliaPaths.launchAgentPlist().path),
            let plist = try? PropertyListSerialization.propertyList(from: data, format: nil)
                as? [String: Any],
            let arguments = plist["ProgramArguments"] as? [String]
        {
            program = arguments.first
        }
        return SetupStatus(
            agentProgram: program,
            bundledHelper: Helper.bundled?.path,
            sudoersInstalled: fm.fileExists(atPath: VigiliaPaths.sudoersFile.path))
    }
}

/// The Rust `vigilia` binary the app starts when the service isn't running.
enum Helper {
    /// `Contents/Helpers/vigilia`: `Helpers`, not `MacOS`, because `MacOS/vigilia` and
    /// `MacOS/Vigilia` are the same file on a case-insensitive volume.
    static var bundled: URL? {
        let url = Bundle.main.bundleURL.appendingPathComponent("Contents/Helpers/vigilia")
        return FileManager.default.isExecutableFile(atPath: url.path) ? url : nil
    }

    /// The bundled helper, or `VIGILIA_BIN` for development builds run with `swift run`.
    static var url: URL? {
        if let bundled { return bundled }
        if let dev = ProcessInfo.processInfo.environment["VIGILIA_BIN"],
            FileManager.default.isExecutableFile(atPath: dev)
        {
            return URL(fileURLWithPath: dev)
        }
        return nil
    }
}
