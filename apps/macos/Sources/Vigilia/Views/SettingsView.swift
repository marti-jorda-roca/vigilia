import AppKit
import ServiceManagement
import SwiftUI
import VigiliaProtocol

struct SettingsView: View {
    @ObservedObject var store: SessionStore
    @AppStorage(Preferences.lidClosedByDefault) private var lidClosedByDefault = false
    @State private var floor: Double = 10
    @State private var launchAtLogin = LoginItem.enabled
    @State private var loginError: String?

    var body: some View {
        Form {
            if store.capabilities.powerEvents {
                Section("Rules") {
                    if let rules = store.settings?.rules {
                        LabeledContent("Battery floor") {
                            HStack {
                                Slider(value: $floor, in: 0...100, step: 1) { editing in
                                    if !editing { store.setBatteryFloor(Int(floor)) }
                                }
                                Text("\(Int(floor))%").monospacedDigit().frame(width: 40, alignment: .trailing)
                            }
                        }
                        Text("On battery, the session ends below this charge.")
                            .font(.caption).foregroundColor(.secondary)
                        Toggle(
                            "End the session at critical heat",
                            isOn: Binding(get: { rules.endAtCriticalHeat }, set: { store.setEndAtCriticalHeat($0) }))
                    } else {
                        Text(store.connection == .connecting ? "Connecting…" : "Not connected to the service.")
                            .foregroundColor(.secondary)
                    }
                }
            }

            if store.capabilities.keepAwakeLidClosed {
                Section("Lid") {
                    Toggle("Keep awake with the lid closed by default", isOn: $lidClosedByDefault)
                        .disabled(!store.setup.sudoersInstalled)
                    if !store.setup.sudoersInstalled {
                        Text("Lid-closed sessions need a one-time setup. Until in-app setup is available, run `vigilia install` in Terminal.")
                            .font(.caption).foregroundColor(.secondary)
                    }
                }
            }

            Section("General") {
                Toggle("Launch at login", isOn: $launchAtLogin)
                    .disabled(!LoginItem.available)
                    .onChange(of: launchAtLogin) { on in
                        do {
                            try LoginItem.set(on)
                            loginError = nil
                        } catch {
                            loginError = error.localizedDescription
                            launchAtLogin = LoginItem.enabled
                        }
                    }
                if let loginError {
                    Text(loginError).font(.caption).foregroundColor(.red)
                }
            }

            Section("Background service") {
                LabeledContent("Service", value: serviceStatus)
                LabeledContent("Lid-closed sessions", value: store.setup.sudoersInstalled ? "Set up" : "Not set up")
                HStack {
                    Button("Set up…") {}.disabled(true)
                    Button("Repair") {}.disabled(true)
                }
                .help("In-app setup comes with the packaged app. Use `vigilia install` for now.")
            }

            Section("About") {
                LabeledContent("Vigilia", value: Self.appVersion)
                LabeledContent("Service", value: store.serverVersion ?? "–")
                LabeledContent("Protocol", value: "v\(protocolVersion)")
            }
        }
        .formStyle(.grouped)
        .frame(width: 440)
        .fixedSize(horizontal: false, vertical: true)
        .onAppear { sync(store.settings) }
        // `ConfigChanged` from the CLI or another client lands here.
        .onChange(of: store.settings) { sync($0) }
    }

    private var serviceStatus: String {
        let setup = store.setup
        if !setup.agentInstalled { return "Not installed (started on demand)" }
        if setup.needsRepair { return "Needs repair" }
        return "Installed"
    }

    private func sync(_ settings: VigiliaProtocol.Settings?) {
        if let settings {
            floor = Double(settings.rules.batteryFloorPercent)
        }
    }

    private static var appVersion: String {
        Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? "dev"
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
/// the window is managed here.
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
        window.styleMask = [.titled, .closable]
        window.isReleasedWhenClosed = false
        window.delegate = self
        return window
    }
}
