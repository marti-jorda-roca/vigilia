import AppKit
import ServiceManagement
import SwiftUI
import VigiliaProtocol

struct SettingsView: View {
    @ObservedObject var store: SessionStore
    @ObservedObject var setup: SetupModel
    @AppStorage(Preferences.lidClosedByDefault) private var lidClosedByDefault = false
    @State private var launchAtLogin = LoginItem.enabled
    @State private var loginError: String?

    var body: some View {
        VStack(alignment: .leading, spacing: Pixel.unit * 10) {
            HStack(spacing: Pixel.unit * 5) {
                LanternView(state: store.session != nil ? .lit : .unlit, scale: Pixel.unit * 1.5)
                PixelText("SETTINGS", scale: Pixel.unit * 2, color: Pixel.orange)
            }

            if store.capabilities.powerEvents {
                Section(title: "RULES") {
                    if let rules = store.settings?.rules {
                        HStack {
                            PixelText("BATTERY FLOOR")
                            Spacer()
                            Stepper(value: rules.batteryFloorPercent) { store.setBatteryFloor($0) }
                        }
                        Text("On battery, the session ends below this charge.").foregroundColor(Pixel.dim)
                        Toggle(isOn: Binding(get: { rules.endAtCriticalHeat }, set: { store.setEndAtCriticalHeat($0) })) {
                            PixelText("END AT CRITICAL HEAT")
                        }
                    } else if store.settingsUnsupported {
                        Text("The running service (\(store.serverVersion ?? "older")) has no settings yet. Update the CLI with `cargo install --path crates/cli --locked` from the repo; the service picks it up the next time it starts.")
                            .foregroundColor(Pixel.dim)
                    } else {
                        Text(store.connection == .connecting ? "Connecting…" : "Not connected to the service.")
                            .foregroundColor(Pixel.dim)
                    }
                }
            }

            if store.capabilities.keepAwakeLidClosed {
                Section(title: "LID") {
                    Toggle(isOn: $lidClosedByDefault) { PixelText("LID CLOSED BY DEFAULT") }
                        .disabled(!setup.status.sudoersInstalled)
                    if !setup.status.sudoersInstalled {
                        Text("Lid-closed sessions need a one-time setup, under Setup below.")
                            .foregroundColor(Pixel.dim)
                    }
                }
            }

            Section(title: "GENERAL") {
                Toggle(isOn: $launchAtLogin) { PixelText("LAUNCH AT LOGIN") }
                    .disabled(!LoginItem.available)
                    .help(LoginItem.available ? "Start Vigilia when you log in." : "Needs Vigilia to run as an app from Applications.")
                    .onChange(of: launchAtLogin) { on in
                        guard on != LoginItem.enabled else { return }
                        do {
                            try LoginItem.set(on)
                            loginError = nil
                        } catch {
                            loginError = error.localizedDescription
                            launchAtLogin = LoginItem.enabled
                        }
                    }
                if !LoginItem.available {
                    Text("Available when Vigilia runs as an app from Applications.").foregroundColor(Pixel.dim)
                }
                if let loginError {
                    Text(loginError).foregroundColor(Pixel.red)
                }
            }

            Section(title: "SETUP") {
                SetupSection(setup: setup, sessionLive: store.session != nil)
            }

            Section(title: "ABOUT") {
                Row(label: "VIGILIA", value: Self.appVersion)
                Row(label: "SERVICE", value: store.serverVersion ?? "–")
                Row(label: "PROTOCOL", value: "v\(protocolVersion)")
            }
        }
        .toggleStyle(PixelToggleStyle())
        .fixedSize(horizontal: false, vertical: true)
        .padding(.horizontal, Pixel.unit * 12)
        .padding(.top, 36)  // under the window's close button
        .padding(.bottom, Pixel.unit * 12)
        .frame(width: 440)
        .background(ZStack(alignment: .top) { Pixel.background; PixelStars().frame(height: 60) })
        .pixelSurface()
    }

    private static var appVersion: String {
        Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? "dev"
    }
}

/// Installing the service and the lid-closed rule from the app, and removing them again. Only a
/// packaged app in Applications sets anything up: the LaunchAgent stores the helper's path.
private struct SetupSection: View {
    @ObservedObject var setup: SetupModel
    let sessionLive: Bool
    @State private var copied = false

    private static let setupCommand = "vigilia install"

    var body: some View {
        VStack(alignment: .leading, spacing: Pixel.unit * 5) {
            switch setup.location {
            case .notBundled:
                terminalFallback
            case .translocated, .elsewhere:
                moveToApplications
            case .applications:
                steps
            }
            if let error = setup.error {
                Text(error).foregroundColor(Pixel.red)
            }
        }
        .buttonStyle(PixelButtonStyle())
        .disabled(setup.busy)
    }

    // MARK: Packaged app in Applications

    @ViewBuilder private var steps: some View {
        let status = setup.status
        Row(label: "SERVICE", value: serviceState)
        Text(serviceExplanation).foregroundColor(Pixel.dim)
        HStack {
            Button { setup.installService() } label: {
                PixelText(!status.agentInstalled ? "INSTALL" : status.needsRepair ? "REPAIR" : "REINSTALL")
            }
            Spacer()
        }

        Row(label: "LID CLOSED", value: status.sudoersInstalled ? "set up" : "not set up")
        if status.sudoersInstalled {
            Text("Sessions can keep the Mac awake with the lid closed. Removing asks for your password.")
                .foregroundColor(Pixel.dim)
        } else {
            Text("To keep the Mac awake with the lid closed, Vigilia needs this rule in /etc/sudoers.d/vigilia. It asks for your password once, and also puts `vigilia` in /usr/local/bin for Terminal.")
                .foregroundColor(Pixel.dim)
            Text(SudoersRule.text(user: NSUserName()))
                .font(.system(size: 10, design: .monospaced))
                .textSelection(.enabled)
        }
        HStack {
            if status.sudoersInstalled {
                Button { setup.removeLidSetup() } label: { PixelText("REMOVE…") }
            } else {
                Button { setup.installLidSetup() } label: { PixelText("SET UP…") }
            }
            Spacer()
        }

        PixelRule(color: Pixel.faint)
        HStack {
            Button { setup.uninstall() } label: { PixelText("UNINSTALL") }
                .disabled(sessionLive || !somethingInstalled)
            Spacer()
        }
        Text(uninstallExplanation).foregroundColor(Pixel.dim)
    }

    private var somethingInstalled: Bool {
        let status = setup.status
        return status.agentInstalled || status.sudoersInstalled || status.cliLinked
    }

    private var serviceState: String {
        let status = setup.status
        if !status.agentInstalled { return "not installed" }
        if status.needsRepair { return "needs repair" }
        return "installed"
    }

    private var serviceExplanation: String {
        let status = setup.status
        guard let program = status.agentProgram else {
            return "Lets Vigilia start the service when it's needed and undo a session's changes after a restart. No password needed."
        }
        if !status.agentProgramExists {
            return "The service points at a program that is gone (\(program)). Repair points it at this app."
        }
        if status.needsRepair {
            return "The service runs another copy of vigilia (\(program)). Repair points it at this app."
        }
        return "The service starts when it's needed and stops when idle."
    }

    private var uninstallExplanation: String {
        if setup.uninstalled, !somethingInstalled {
            return "Removed. Drag Vigilia to the Trash to finish."
        }
        if sessionLive {
            return "End the session first."
        }
        return "Removes the service, the lid-closed rule and the Terminal link. Then drag Vigilia to the Trash."
    }

    // MARK: Not in Applications

    @ViewBuilder private var moveToApplications: some View {
        Text(setup.location == .translocated
            ? "Vigilia is running straight from the download. Drag it into Applications and open it from there: the service remembers where its program is."
            : "Move Vigilia to Applications first, and open it from there: the service remembers where its program is.")
            .foregroundColor(Pixel.dim)
        HStack {
            Button {
                NSWorkspace.shared.activateFileViewerSelecting([Bundle.main.bundleURL])
            } label: {
                PixelText("SHOW IN FINDER")
            }
            .disabled(setup.location == .translocated)
            Spacer()
        }
    }

    // MARK: Development build (`swift run`)

    /// A binary that isn't an app bundle has no helper to install, so the button hands over the
    /// command to run in Terminal.
    @ViewBuilder private var terminalFallback: some View {
        Row(label: "SERVICE", value: setup.status.agentInstalled ? (setup.status.needsRepair ? "needs repair" : "installed") : "started on demand")
        Row(label: "LID SETUP", value: setup.status.sudoersInstalled ? "set up" : "not set up")
        HStack {
            Button(action: copySetupCommand) { PixelText(copied ? "COPIED" : "COPY COMMAND") }
                .help("Copy `\(Self.setupCommand)` to run in Terminal.")
            Spacer()
        }
        Text("This build isn't the packaged app, so it can't set anything up. Run `\(Self.setupCommand)` in Terminal; the button copies the command.")
            .foregroundColor(Pixel.dim)
    }

    private func copySetupCommand() {
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(Self.setupCommand, forType: .string)
        copied = true
        Task {
            try? await Task.sleep(nanoseconds: 2_000_000_000)
            copied = false
        }
    }
}

private struct Section<Content: View>: View {
    let title: String
    @ViewBuilder let content: Content

    var body: some View {
        VStack(alignment: .leading, spacing: Pixel.unit * 5) {
            PixelHeading(title: title)
            content.fixedSize(horizontal: false, vertical: true)
        }
    }
}

private struct Row: View {
    let label: String
    let value: String

    var body: some View {
        HStack {
            PixelText(label)
            Spacer()
            Text(value).foregroundColor(Pixel.dim)
        }
        .accessibilityElement(children: .combine)
    }
}

/// `[-] 10% [+]` in steps of 5.
private struct Stepper: View {
    let value: Int
    let onChange: (Int) -> Void

    var body: some View {
        HStack(spacing: Pixel.unit * 4) {
            Button { onChange(step(-1)) } label: { PixelText("-") }
                .disabled(value <= 0)
                .accessibilityLabel("Lower the battery floor")
            PixelText("\(value)%", color: Pixel.yellow)
                .frame(minWidth: 40)
            Button { onChange(step(1)) } label: { PixelText("+") }
                .disabled(value >= 100)
                .accessibilityLabel("Raise the battery floor")
        }
        .buttonStyle(PixelButtonStyle())
    }

    /// The next multiple of 5 in `direction`, so a value set from the CLI (say 12) snaps.
    private func step(_ direction: Int) -> Int {
        let next = direction > 0 ? (value / 5 + 1) * 5 : ((value + 4) / 5 - 1) * 5
        return min(max(next, 0), 100)
    }
}

/// Launch at login, through the system's login items.
enum LoginItem {
    /// `SMAppService.mainApp` needs a real app bundle, not `swift run`.
    static var available: Bool { Bundle.main.bundleURL.pathExtension == "app" }

    static var enabled: Bool { available && SMAppService.mainApp.status == .enabled }

    static func set(_ on: Bool) throws {
        if on {
            try SMAppService.mainApp.register()
        } else {
            try SMAppService.mainApp.unregister()
        }
    }
}

/// The Settings window. The app has no SwiftUI `App` scene (it is an AppKit menu bar app), so
/// the window is managed here. It is drawn black, like the banner, in light mode too.
@MainActor
final class SettingsWindowController: NSObject, NSWindowDelegate {
    private let store: SessionStore
    private let setup: SetupModel
    private var window: NSWindow?

    init(store: SessionStore, setup: SetupModel) {
        self.store = store
        self.setup = setup
    }

    func show() {
        let window = self.window ?? makeWindow()
        self.window = window
        NSApp.activate(ignoringOtherApps: true)
        if !window.isVisible {
            window.center()
            store.settingsOpened()
            setup.refresh()
        }
        window.makeKeyAndOrderFront(nil)
    }

    func windowWillClose(_: Notification) {
        store.settingsClosed()
    }

    private func makeWindow() -> NSWindow {
        let window = NSWindow(contentViewController: NSHostingController(rootView: SettingsView(store: store, setup: setup)))
        window.title = "Vigilia Settings"
        window.styleMask = [.titled, .closable, .fullSizeContentView]
        window.titlebarAppearsTransparent = true
        window.titleVisibility = .hidden
        window.backgroundColor = .black
        window.appearance = NSAppearance(named: .darkAqua)
        window.isReleasedWhenClosed = false
        window.delegate = self
        return window
    }
}
