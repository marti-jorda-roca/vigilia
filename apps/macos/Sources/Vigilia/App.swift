import AppKit

@main
@MainActor
final class AppDelegate: NSObject, NSApplicationDelegate {
    private var store: SessionStore!
    private var statusItem: StatusItemController!
    private var settingsWindow: SettingsWindowController!
    private var doorbell: Doorbell?

    static func main() {
        let app = NSApplication.shared
        let delegate = AppDelegate()
        app.delegate = delegate
        // A menu bar app: no Dock icon or main menu. The bundle's Info.plist sets LSUIElement
        // too; this also covers `swift run`, where there is no bundle.
        app.setActivationPolicy(.accessory)
        withExtendedLifetime(delegate) { app.run() }
    }

    func applicationDidFinishLaunching(_: Notification) {
        let store = SessionStore()
        let settingsWindow = SettingsWindowController(store: store)
        self.store = store
        self.settingsWindow = settingsWindow
        statusItem = StatusItemController(store: store) { settingsWindow.show() }
        doorbell = Doorbell { store.doorbellRang() }
        store.launched()
    }

    func applicationWillTerminate(_: Notification) {
        // Only the socket closes; the session carries on in the service.
        store.disconnect()
    }
}
