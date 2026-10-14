import Foundation
import VigiliaProtocol
import notify

/// The service posts a Darwin notification on every session change. Listening costs nothing
/// while idle, so the app only connects when there is something to show.
@MainActor
final class Doorbell {
    private var token: Int32 = 0
    private var registered = false

    init(onRing: @escaping @MainActor () -> Void) {
        let status = notify_register_dispatch(
            VigiliaPaths.sessionChangedNotification, &token, DispatchQueue.main
        ) { _ in
            MainActor.assumeIsolated { onRing() }
        }
        registered = status == NOTIFY_STATUS_OK
    }

    deinit {
        if registered {
            notify_cancel(token)
        }
    }
}
