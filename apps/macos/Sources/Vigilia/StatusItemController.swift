import AppKit
import Combine
import SwiftUI
import VigiliaProtocol
import os

/// The menu bar item: an icon, the countdown in minutes, and a popover with the menu.
///
/// Energy: the title is redrawn only when its text changes, by a one-shot timer set for the
/// next minute boundary. Nothing animates in the menu bar, and nothing runs while idle.
@MainActor
final class StatusItemController: NSObject, NSPopoverDelegate {
    private let store: SessionStore
    private let item = NSStatusBar.system.statusItem(withLength: NSStatusItem.variableLength)
    private let popover = NSPopover()
    private var timer: Timer?
    private var observers: Set<AnyCancellable> = []
    private var redraws = 0
    private let log = Logger(subsystem: "com.vigilia.app", category: "status-item")

    init(store: SessionStore, openSettings: @escaping () -> Void) {
        self.store = store
        super.init()

        popover.behavior = .transient
        popover.delegate = self
        let menu = MenuView(store: store) { [weak self] in
            self?.popover.performClose(nil)
            openSettings()
        }
        popover.contentViewController = NSHostingController(rootView: menu)

        if let button = item.button {
            button.target = self
            button.action = #selector(togglePopover)
            button.imagePosition = .imageLeading
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

    @objc private func togglePopover() {
        guard let button = item.button else { return }
        if popover.isShown {
            popover.performClose(nil)
        } else {
            NSApp.activate(ignoringOtherApps: true)
            popover.show(relativeTo: button.bounds, of: button, preferredEdge: .minY)
            store.menuOpened()
        }
    }

    func popoverDidClose(_: Notification) {
        store.menuClosed()
    }

    private func update() {
        guard let button = item.button else { return }

        let symbol: String
        let description: String
        if store.hasAlert {
            symbol = "exclamationmark.triangle.fill"
            description = "Vigilia needs attention"
        } else if store.session != nil {
            symbol = "cup.and.saucer.fill"
            description = "Vigilia: keeping awake"
        } else {
            symbol = "cup.and.saucer"
            description = "Vigilia"
        }
        if button.image?.accessibilityDescription != description {
            let image = NSImage(systemSymbolName: symbol, accessibilityDescription: description)
            image?.isTemplate = true
            button.image = image
        }

        let now = Date()
        var title = ""
        if let session = store.session, session.state == .active {
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
        guard let session = store.session,
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
