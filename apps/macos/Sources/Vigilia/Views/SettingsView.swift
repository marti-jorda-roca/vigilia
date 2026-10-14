import AppKit
import ServiceManagement
import SwiftUI
import VigiliaProtocol

struct SettingsView: View {
    @ObservedObject var store: SessionStore
    @AppStorage(Preferences.lidClosedByDefault) private var lidClosedByDefault = false
    @State private var launchAtLogin = LoginItem.enabled
    @State private var loginError: String?
    @State private var copied = false

    private static let setupCommand = "vigilia install"

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
                        .disabled(!store.setup.sudoersInstalled)
                    if !store.setup.sudoersInstalled {
                        Text("Lid-closed sessions need a one-time setup. Until in-app setup is available, run `vigilia install` in Terminal.")
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

            Section(title: "SERVICE") {
                Row(label: "SERVICE", value: serviceStatus)
                Row(label: "LID SETUP", value: store.setup.sudoersInstalled ? "set up" : "not set up")
                // In-app setup comes with the packaged app. Until then, the buttons hand over the
                // command to run in Terminal.
                HStack(spacing: Pixel.unit * 4) {
                    Button(action: copySetupCommand) {
                        PixelText(copied ? "COPIED" : setupAction.title)
                    }
                    .help("Copy `\(Self.setupCommand)` to run in Terminal.")
                    Spacer()
                }
                .buttonStyle(PixelButtonStyle())
                Text(setupAction.explanation).foregroundColor(Pixel.dim)
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

    private var setupAction: (title: String, explanation: String) {
        let setup = store.setup
        let run = "Run `\(Self.setupCommand)` in Terminal"
        if setup.needsRepair {
            return ("REPAIR", "The service points at a program that moved. \(run) to fix it; the button copies the command.")
        }
        if setup.agentInstalled, setup.sudoersInstalled {
            return ("REINSTALL", "The service and lid-closed sessions are set up. \(run) again to reinstall; the button copies the command.")
        }
        return ("SET UP", "\(run) to install the service and lid-closed sessions; the button copies the command.")
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

    private var serviceStatus: String {
        let setup = store.setup
        if !setup.agentInstalled { return "started on demand" }
        if setup.needsRepair { return "needs repair" }
        return "installed"
    }

    private static var appVersion: String {
        Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? "dev"
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
    private var window: NSWindow?

    init(store: SessionStore) {
        self.store = store
    }

    func show() {
        let window = self.window ?? makeWindow()
        self.window = window
        NSApp.activate(ignoringOtherApps: true)
        if !window.isVisible {
            window.center()
            store.settingsOpened()
        }
        window.makeKeyAndOrderFront(nil)
    }

    func windowWillClose(_: Notification) {
        store.settingsClosed()
    }

    private func makeWindow() -> NSWindow {
        let window = NSWindow(contentViewController: NSHostingController(rootView: SettingsView(store: store)))
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
