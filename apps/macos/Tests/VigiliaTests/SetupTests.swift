import Foundation
import Testing

@testable import Vigilia

/// The LaunchAgent stores the helper's absolute path, so setup runs only from Applications.
struct AppLocationTests {
    let home = "/Users/ana"

    @Test func applicationsFolders() {
        #expect(AppLocation.classify(bundlePath: "/Applications/Vigilia.app", home: home) == .applications)
        #expect(AppLocation.classify(bundlePath: "/Applications/Tools/Vigilia.app", home: home) == .applications)
        #expect(AppLocation.classify(bundlePath: "/Users/ana/Applications/Vigilia.app", home: home) == .applications)
    }

    @Test func translocatedDownload() {
        let path = "/private/var/folders/xy/abc/T/AppTranslocation/1234-5678/d/Vigilia.app"
        #expect(AppLocation.classify(bundlePath: path, home: home) == .translocated)
    }

    @Test func elsewhere() {
        #expect(AppLocation.classify(bundlePath: "/Users/ana/Downloads/Vigilia.app", home: home) == .elsewhere)
        #expect(AppLocation.classify(bundlePath: "/ApplicationsOld/Vigilia.app", home: home) == .elsewhere)
    }

    @Test func notABundle() {
        #expect(
            AppLocation.classify(bundlePath: "/Users/ana/vigilia/apps/macos/.build/release", home: home) == .notBundled)
    }
}

/// The admin step is one string run by `osascript`, then `/bin/sh`: both layers of quoting must
/// survive any path or user name.
struct AdminScriptTests {
    static let awkward = #"/Applications/My "Tools" \ Bob's/Vigilia.app/Contents/Helpers/vigilia"#

    @Test(arguments: ["plain", "with space", "it's", #"quote " and \ backslash"#, "$(touch x) `y`"])
    func shellQuotingRoundTrips(_ argument: String) throws {
        let output = try run("/bin/sh", ["-c", "printf %s \(AdminScript.shellQuote(argument))"])
        #expect(output == argument)
    }

    @Test(arguments: ["plain", #"say "hi""#, #"back\slash"#, Self.awkward])
    func appleScriptStringRoundTrips(_ text: String) throws {
        let output = try run("/usr/bin/osascript", ["-e", "return \(AdminScript.appleScriptString(text))"])
        #expect(output == text + "\n")
    }

    @Test func installRunsTheHelperThenLinksIt() {
        let script = AdminScript.install(helper: "/Applications/Vigilia.app/Contents/Helpers/vigilia", user: "ana")
        #expect(script.hasPrefix("do shell script \""))
        #expect(script.hasSuffix("\" with administrator privileges"))
        #expect(
            script.contains("'/Applications/Vigilia.app/Contents/Helpers/vigilia' sudoers install --user 'ana' && "))
        // Never replaces a real file at the link's path.
        #expect(script.contains("if [ ! -e '/usr/local/bin/vigilia' ] || [ -L '/usr/local/bin/vigilia' ]; then"))
    }

    @Test func removeUnlinksOnlyAVigiliaHelper() {
        let script = AdminScript.remove(helper: "/Applications/Vigilia.app/Contents/Helpers/vigilia")
        #expect(script.contains("sudoers remove && case"))
        #expect(script.contains("*/Contents/Helpers/vigilia) /bin/rm -f '/usr/local/bin/vigilia'"))
    }

    @Test func sudoersRuleMatchesTheRustOne() throws {
        let source = try String(
            contentsOf: Self.repoRoot.appendingPathComponent("crates/daemon/src/install.rs"), encoding: .utf8)
        #expect(source.contains(SudoersRule.text(user: "{user}")))
    }

    static let repoRoot = URL(fileURLWithPath: #filePath)
        .deletingLastPathComponent()  // VigiliaTests
        .deletingLastPathComponent()  // Tests
        .deletingLastPathComponent()  // macos
        .deletingLastPathComponent()  // apps
        .deletingLastPathComponent()

    private func run(_ executable: String, _ arguments: [String]) throws -> String {
        let process = Process()
        process.executableURL = URL(fileURLWithPath: executable)
        process.arguments = arguments
        let pipe = Pipe()
        process.standardOutput = pipe
        try process.run()
        let data = pipe.fileHandleForReading.readDataToEndOfFile()
        process.waitUntilExit()
        return String(decoding: data, as: UTF8.self)
    }
}

struct SetupStatusTests {
    let dir = FileManager.default.temporaryDirectory
        .appendingPathComponent("vigilia-setup-\(UUID().uuidString)")

    private func helper(_ name: String) throws -> URL {
        let url = dir.appendingPathComponent(name)
        try FileManager.default.createDirectory(at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
        FileManager.default.createFile(atPath: url.path, contents: Data(), attributes: [.posixPermissions: 0o755])
        return url
    }

    private func agent(program: String) throws -> URL {
        let url = dir.appendingPathComponent("com.vigilia.service.plist")
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        let data = try PropertyListSerialization.data(
            fromPropertyList: ["ProgramArguments": [program, "daemon"]], format: .xml, options: 0)
        try data.write(to: url)
        return url
    }

    private func detect(agent: URL?, bundled: URL?, link: URL? = nil) -> SetupStatus {
        SetupStatus.detect(
            launchAgentPlist: agent ?? dir.appendingPathComponent("absent.plist"),
            bundledHelper: bundled,
            sudoersFile: dir.appendingPathComponent("sudoers"),
            cliLink: link ?? dir.appendingPathComponent("absent-link"))
    }

    @Test func nothingInstalled() throws {
        let status = detect(agent: nil, bundled: try helper("App/vigilia"))
        #expect(!status.agentInstalled)
        #expect(!status.needsRepair)
        #expect(!status.sudoersInstalled)
        #expect(!status.cliLinked)
    }

    @Test func pointsAtThisApp() throws {
        let bundled = try helper("App/vigilia")
        let status = detect(agent: try agent(program: bundled.path), bundled: bundled)
        #expect(status.pointsAtBundledHelper)
        #expect(!status.needsRepair)
    }

    @Test func programIsGone() throws {
        let status = detect(agent: try agent(program: dir.appendingPathComponent("moved/vigilia").path), bundled: nil)
        #expect(status.agentInstalled)
        #expect(!status.agentProgramExists)
        #expect(status.needsRepair)
    }

    @Test func anotherCopy() throws {
        let other = try helper("cargo/vigilia")
        let status = detect(agent: try agent(program: other.path), bundled: try helper("App/vigilia"))
        #expect(status.agentProgramExists)
        #expect(status.needsRepair)
        // Without a bundle (`swift run`), another copy is the only one there is.
        #expect(!detect(agent: try agent(program: other.path), bundled: nil).needsRepair)
    }

    @Test func cliLink() throws {
        let bundled = try helper("App/vigilia")
        let link = dir.appendingPathComponent("bin/vigilia")
        try FileManager.default.createDirectory(at: link.deletingLastPathComponent(), withIntermediateDirectories: true)
        try FileManager.default.createSymbolicLink(at: link, withDestinationURL: bundled)
        #expect(detect(agent: nil, bundled: bundled, link: link).cliLinked)
        #expect(!detect(agent: nil, bundled: try helper("Other/vigilia"), link: link).cliLinked)
    }
}
