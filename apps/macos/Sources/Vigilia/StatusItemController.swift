import AppKit
import Combine
import SwiftUI
import VigiliaProtocol
import os

/// The menu bar item: the lantern, the time left of a timed session, and the menu under it.
///
/// Energy: the title is redrawn only when its text changes, by a one-shot timer set for the
/// next minute boundary. Sessions without a timeout show no time, so they need no timer at all.
/// Nothing animates in the menu bar, and nothing runs while idle.
@MainActor
final class StatusItemController: NSObject {
    private let store: SessionStore
    private let setup: SetupModel
    private let item = NSStatusBar.system.statusItem(withLength: NSStatusItem.variableLength)
    private let panel = MenuPanel()
    private let hosting = NSHostingController(rootView: AnyView(EmptyView()))
    private var contentSize: CGSize = .zero
    private var timer: Timer?
    private var observers: Set<AnyCancellable> = []
    private var appearanceObservation: NSKeyValueObservation?
    private var sizeObservation: NSKeyValueObservation?
    private var clickMonitor: Any?
    private var keyMonitor: Any?
    private var iconKey = ""
    private var redraws = 0
    private let log = Logger(subsystem: "com.vigilia.app", category: "status-item")

    init(store: SessionStore, setup: SetupModel, openSettings: @escaping () -> Void) {
        self.store = store
        self.setup = setup
        super.init()

        let menu = MenuView(store: store, setup: setup) { [weak self] in
            self?.closeMenu()
            openSettings()
        }
        hosting.rootView = AnyView(menu)
        // The panel's frame is set only by `place()`, from the content's preferred size. Left
        // to itself, the hosting view resizes its window to fit the content while keeping the
        // bottom-left corner fixed, so the menu drifts away from the menu bar whenever its
        // content changes height.
        hosting.sizingOptions = [.preferredContentSize]
        panel.contentView = hosting.view
        sizeObservation = hosting.observe(\.preferredContentSize, options: [.new]) { [weak self] _, change in
            guard let size = change.newValue else { return }
            MainActor.assumeIsolated { self?.resize(to: size) }
        }

        if let button = item.button {
            button.target = self
            button.action = #selector(toggleMenu)
            button.imagePosition = .imageLeading
            // The coloured lanterns draw their own frame colour; redraw when the menu bar
            // switches between light and dark.
            appearanceObservation = button.observe(\.effectiveAppearance) { [weak self] _, _ in
                MainActor.assumeIsolated { self?.update() }
            }
        }

        // `objectWillChange` fires before the change; read the new state on the next turn.
        store.objectWillChange
            .receive(on: RunLoop.main)
            .sink { [weak self] in self?.update() }
            .store(in: &observers)
        // Timers don't count time asleep; catch up on wake.
        NSWorkspace.shared.notificationCenter
            .publisher(for: NSWorkspace.didWakeNotification)
            .sink { [weak self] _ in self?.update() }
            .store(in: &observers)
        update()
    }

    // MARK: Menu

    @objc private func toggleMenu() {
        if panel.isVisible {
            closeMenu()
        } else {
            openMenu()
        }
    }

    func openMenu() {
        store.menuOpened()
        setup.refresh()
        contentSize = hosting.sizeThatFits(in: CGSize(width: 10_000, height: 10_000))
        hosting.view.layoutSubtreeIfNeeded()
        place()
        panel.makeKeyAndOrderFront(nil)
        item.button?.highlight(true)
        // Close on a click anywhere else, as a menu does. Clicks on the status item itself
        // arrive at `toggleMenu` instead.
        #if DEBUG
            if ProcessInfo.processInfo.environment["VIGILIA_DEBUG_OPEN_MENU"] != nil { return }
        #endif
        clickMonitor = NSEvent.addGlobalMonitorForEvents(matching: [.leftMouseDown, .rightMouseDown, .otherMouseDown]) {
            [weak self] _ in
            MainActor.assumeIsolated { self?.closeMenu() }
        }
        keyMonitor = NSEvent.addLocalMonitorForEvents(matching: .keyDown) { [weak self] event in
            guard event.keyCode == 53 else { return event }  // Escape
            self?.closeMenu()
            return nil
        }
    }

    private func closeMenu() {
        guard panel.isVisible else { return }
        panel.orderOut(nil)
        item.button?.highlight(false)
        for monitor in [clickMonitor, keyMonitor].compactMap({ $0 }) {
            NSEvent.removeMonitor(monitor)
        }
        clickMonitor = nil
        keyMonitor = nil
        store.menuClosed()
    }

    private func resize(to size: CGSize) {
        guard size != .zero else { return }
        contentSize = size
        if panel.isVisible, panel.frame.size != size { place() }
    }

    /// Puts the panel under the status item, its top edge fixed as the content grows or shrinks.
    private func place() {
        guard let button = item.button, let window = button.window else { return }
        let anchor = window.convertToScreen(button.convert(button.bounds, to: nil))
        let screen = window.screen ?? NSScreen.main
        let frame = MenuPlacement.frame(size: contentSize, below: anchor, on: screen?.frame ?? anchor)
        panel.setFrame(frame, display: true)
    }

    // MARK: Status item

    private func update() {
        guard let button = item.button else { return }

        let state: Lantern.State = store.hasAlert ? .alert : (store.session != nil ? .lit : .unlit)
        let dark = button.effectiveAppearance.bestMatch(from: [.darkAqua, .aqua]) == .darkAqua
        let key = "\(state)-\(dark)"
        if key != iconKey {
            iconKey = key
            button.image = Lantern.statusImage(state: state, darkMenuBar: dark)
        }

        let now = Date()
        var title = ""
        if let session = store.timedSession {
            title = " " + Countdown.shortLabel(Countdown.value(for: session, at: now))
        }
        if button.title != title {
            button.title = title
            redraws += 1
            log.debug("status title redraw \(self.redraws, privacy: .public): \(title, privacy: .public)")
        }
        schedule(after: now)
    }

    private func schedule(after now: Date) {
        timer?.invalidate()
        timer = nil
        guard let session = store.timedSession,
            let next = Countdown.nextLabelChange(for: session, after: now)
        else { return }
        let timer = Timer(fire: next, interval: 0, repeats: false) { [weak self] _ in
            MainActor.assumeIsolated { self?.update() }
        }
        timer.tolerance = 5
        RunLoop.main.add(timer, forMode: .common)
        self.timer = timer
    }
}
