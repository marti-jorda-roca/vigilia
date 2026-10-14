import AppKit
import Foundation
import VigiliaProtocol

/// What `vigilia install` has set up on this Mac.
struct SetupStatus: Equatable {
    /// `ProgramArguments[0]` of the socket-activated LaunchAgent, if it is installed.
    var agentProgram: String?
    /// The helper inside this app's bundle, when running from a bundle that has one.
    var bundledHelper: String?
    /// Whether lid-closed sessions can be used (`/etc/sudoers.d/vigilia` exists).
    var sudoersInstalled: Bool
    /// Where `/usr/local/bin/vigilia` points, when it is a symlink.
    var cliLinkTarget: String?

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

    /// `vigilia` on `PATH` is this app's helper.
    var cliLinked: Bool {
        cliLinkTarget != nil && cliLinkTarget == bundledHelper
    }

    static func detect(
        launchAgentPlist: URL = VigiliaPaths.launchAgentPlist(),
        bundledHelper: URL? = Helper.bundled,
        sudoersFile: URL = VigiliaPaths.sudoersFile,
        cliLink: URL = Helper.cliLink
    ) -> SetupStatus {
        let fm = FileManager.default
        var program: String?
        if let data = fm.contents(atPath: launchAgentPlist.path),
            let plist = try? PropertyListSerialization.propertyList(from: data, format: nil)
                as? [String: Any],
            let arguments = plist["ProgramArguments"] as? [String]
        {
            program = arguments.first
        }
        return SetupStatus(
            agentProgram: program,
            bundledHelper: bundledHelper?.path,
            sudoersInstalled: fm.fileExists(atPath: sudoersFile.path),
            cliLinkTarget: try? fm.destinationOfSymbolicLink(atPath: cliLink.path))
    }
}

/// Where this copy of the app runs from. The LaunchAgent stores the helper's absolute path, so
/// the service is set up only from Applications, where the app stays put.
enum AppLocation: Equatable {
    case applications
    /// Opened straight from a download: macOS runs it from a random read-only copy.
    case translocated
    case elsewhere
    /// Not an app bundle: `swift run`, or the binary from `swift build`.
    case notBundled

    static func classify(bundlePath: String, home: String = NSHomeDirectory()) -> AppLocation {
        guard bundlePath.hasSuffix(".app") else { return .notBundled }
        if bundlePath.contains("/AppTranslocation/") { return .translocated }
        let userApplications = (home as NSString).appendingPathComponent("Applications") + "/"
        if bundlePath.hasPrefix("/Applications/") || bundlePath.hasPrefix(userApplications) {
            return .applications
        }
        return .elsewhere
    }

    static var current: AppLocation { classify(bundlePath: Bundle.main.bundlePath) }
}

/// The Rust `vigilia` binary the app starts when the service isn't running.
enum Helper {
    /// Where lid-closed setup links the helper, so `vigilia` works in Terminal.
    static let cliLink = URL(fileURLWithPath: "/usr/local/bin/vigilia")

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

/// The rule lid-closed setup installs. Mirrors `sudoers_rule` in `crates/daemon/src/install.rs`,
/// which is what actually gets written; it is shown here so the user knows what they allow.
enum SudoersRule {
    static func text(user: String) -> String {
        "\(user) ALL=(root) NOPASSWD: /usr/bin/pmset -a disablesleep 1, /usr/bin/pmset -a disablesleep 0"
    }
}

/// The AppleScript for the steps that need root, run with the system's admin password dialog.
/// Arguments are quoted for the shell, then the whole command for an AppleScript string.
enum AdminScript {
    /// Installs the sudoers rule for `user`, then links the helper into `/usr/local/bin`
    /// unless something other than a symlink is already there.
    static func install(helper: String, user: String) -> String {
        let link = shellQuote(Helper.cliLink.path)
        return script(
            "\(shellQuote(helper)) sudoers install --user \(shellQuote(user))"
                + " && if [ ! -e \(link) ] || [ -L \(link) ]; then"
                + " /bin/mkdir -p /usr/local/bin && /bin/ln -sfn \(shellQuote(helper)) \(link); fi")
    }

    /// Removes the sudoers rule, and the `/usr/local/bin` link if it points into a Vigilia app.
    static func remove(helper: String) -> String {
        let link = shellQuote(Helper.cliLink.path)
        return script(
            "\(shellQuote(helper)) sudoers remove"
                + " && case \"$(/usr/bin/readlink \(link))\" in"
                + " */Contents/Helpers/vigilia) /bin/rm -f \(link) ;; esac")
    }

    static func shellQuote(_ argument: String) -> String {
        "'" + argument.replacingOccurrences(of: "'", with: "'\\''") + "'"
    }

    static func appleScriptString(_ text: String) -> String {
        let escaped =
            text
            .replacingOccurrences(of: "\\", with: "\\\\")
            .replacingOccurrences(of: "\"", with: "\\\"")
        return "\"\(escaped)\""
    }

    private static func script(_ shell: String) -> String {
        "do shell script \(appleScriptString(shell)) with administrator privileges"
    }
}

/// Runs the setup steps: the helper's own `install`/`uninstall` for the LaunchAgents, and
/// `osascript` for the admin steps. Each runs off the main thread.
enum Installer {
    enum Failure: Error, LocalizedError, Equatable {
        /// The user closed the password dialog.
        case cancelled
        case failed(String)

        var errorDescription: String? {
            switch self {
            case .cancelled: "Cancelled."
            case .failed(let reason): reason
            }
        }
    }

    /// The socket-activated service and the recover-at-login job, pointing at `helper`.
    static func installService(helper: URL) async throws {
        try await run(helper, ["install", "--yes", "--skip-sudoers"])
    }

    static func uninstallService(helper: URL) async throws {
        try await run(helper, ["uninstall", "--yes", "--skip-sudoers"])
    }

    static func installLidSetup(helper: URL, user: String = NSUserName()) async throws {
        try await runAdmin(AdminScript.install(helper: helper.path, user: user))
    }

    static func removeLidSetup(helper: URL) async throws {
        try await runAdmin(AdminScript.remove(helper: helper.path))
    }

    private static func runAdmin(_ script: String) async throws {
        do {
            try await run(URL(fileURLWithPath: "/usr/bin/osascript"), ["-e", script])
        } catch let Failure.failed(output) where output.contains("(-128)") {
            throw Failure.cancelled
        }
    }

    /// Runs `executable` to completion. Fails with its output when it exits non-zero.
    private static func run(_ executable: URL, _ arguments: [String]) async throws {
        try await Task.detached {
            let process = Process()
            process.executableURL = executable
            process.arguments = arguments
            process.standardInput = FileHandle.nullDevice
            let pipe = Pipe()
            process.standardOutput = pipe
            process.standardError = pipe
            do {
                try process.run()
            } catch {
                throw Failure.failed(error.localizedDescription)
            }
            let data = pipe.fileHandleForReading.readDataToEndOfFile()
            process.waitUntilExit()
            guard process.terminationStatus == 0 else {
                let output = String(decoding: data, as: UTF8.self).trimmingCharacters(in: .whitespacesAndNewlines)
                throw Failure.failed(output.isEmpty ? "exit status \(process.terminationStatus)" : output)
            }
        }.value
    }
}

/// Setup as the app shows it: what is installed, and the actions that change it.
@MainActor
final class SetupModel: ObservableObject {
    private static let offeredKey = "setupOffered"

    @Published private(set) var status = SetupStatus.detect()
    @Published private(set) var busy = false
    @Published private(set) var error: String?
    /// Set once Uninstall has run, until something is installed again.
    @Published private(set) var uninstalled = false
    let location = AppLocation.current

    /// Setup runs only from a packaged app in Applications.
    var canSetUp: Bool { location == .applications && Helper.bundled != nil }

    func refresh() {
        status = .detect()
    }

    /// At launch: whether to open Settings on the setup. Always when the app runs from a
    /// download or the service points at a program that is gone; once when nothing is set up.
    func shouldOfferSetup(defaults: UserDefaults = .standard) -> Bool {
        guard location != .notBundled else { return false }
        if location == .translocated || (status.agentInstalled && !status.agentProgramExists) {
            return true
        }
        guard location == .elsewhere || !status.agentInstalled else { return false }
        guard !defaults.bool(forKey: Self.offeredKey) else { return false }
        defaults.set(true, forKey: Self.offeredKey)
        return true
    }

    func installService() {
        perform { try await Installer.installService(helper: $0) }
    }

    func installLidSetup() {
        perform { try await Installer.installLidSetup(helper: $0) }
    }

    func removeLidSetup() {
        perform { try await Installer.removeLidSetup(helper: $0) }
    }

    /// Removes the LaunchAgents, then the sudoers rule and the CLI link if they are there.
    func uninstall() {
        let removeAdmin = status.sudoersInstalled || status.cliLinked
        perform(uninstalling: true) { helper in
            try await Installer.uninstallService(helper: helper)
            if removeAdmin {
                try await Installer.removeLidSetup(helper: helper)
            }
        }
    }

    private func perform(uninstalling: Bool = false, _ action: @escaping (URL) async throws -> Void) {
        guard !busy, canSetUp, let helper = Helper.bundled else { return }
        busy = true
        error = nil
        Task {
            do {
                try await action(helper)
                uninstalled = uninstalling
            } catch Installer.Failure.cancelled {
                // The user closed the password dialog: nothing to report.
            } catch {
                self.error = error.localizedDescription
            }
            busy = false
            refresh()
        }
    }
}
