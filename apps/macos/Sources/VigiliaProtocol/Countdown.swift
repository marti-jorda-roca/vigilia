import Foundation

/// What to show for a session. Pure functions: the service never sends ticks, so clients draw
/// the time from `ends_at` (or `started_at` for sessions without an end).
public enum Countdown {
    public enum Value: Equatable, Sendable {
        /// Time left in a timed session.
        case remaining(TimeInterval)
        /// Time since a session without an end started.
        case elapsed(TimeInterval)
    }

    public static func value(for session: Session, at now: Date) -> Value {
        if let endsAt = session.endsAt {
            return .remaining(max(endsAt.timeIntervalSince(now), 0))
        }
        return .elapsed(max(now.timeIntervalSince(session.startedAt), 0))
    }

    /// Minutes only, for the menu bar: `"1h 23m"`, `"2d 3h"`. Remaining time rounds up, so a
    /// fresh 10-minute session reads `10m`; elapsed time rounds down.
    public static func shortLabel(_ value: Value) -> String {
        let minutes = wholeMinutes(value)
        let (d, h, m) = (minutes / 1_440, minutes % 1_440 / 60, minutes % 60)
        if d > 0 { return "\(d)d \(h)h" }
        if h > 0 { return "\(h)h \(m)m" }
        return "\(m)m"
    }

    /// With seconds, for the open menu: `"1h 23m 05s"`, `"4m 09s"`, `"42s"`.
    public static func longLabel(_ value: Value) -> String {
        let seconds = wholeSeconds(value)
        let (d, h, m, s) = (seconds / 86_400, seconds % 86_400 / 3_600, seconds % 3_600 / 60, seconds % 60)
        let ss = String(format: "%02ds", s)
        if d > 0 { return "\(d)d \(h)h " + String(format: "%02dm ", m) + ss }
        if h > 0 { return "\(h)h " + String(format: "%02dm ", m) + ss }
        if m > 0 { return "\(m)m " + ss }
        return "\(s)s"
    }

    /// The next instant `shortLabel` changes, for a one-shot timer. `nil` once a timed session
    /// has run out (its end arrives as an event) or when the session isn't running.
    public static func nextLabelChange(for session: Session, after now: Date) -> Date? {
        guard session.state == .active else { return nil }
        let next: Date
        switch value(for: session, at: now) {
        case .remaining(let left):
            guard left > 0, let endsAt = session.endsAt else { return nil }
            // The label shows ceil(left / 60) minutes; it drops when `left` reaches the next
            // lower multiple of a minute.
            let shown = (left / 60).rounded(.up)
            next = endsAt.addingTimeInterval(-(shown - 1) * 60)
        case .elapsed(let since):
            let shown = (since / 60).rounded(.down)
            next = session.startedAt.addingTimeInterval((shown + 1) * 60)
        }
        // Floating-point fuzz can land a boundary on `now`; never ask for an immediate redraw.
        return next.timeIntervalSince(now) < 0.5 ? next.addingTimeInterval(60) : next
    }

    private static func wholeMinutes(_ value: Value) -> Int {
        switch value {
        case .remaining(let t): Int((t / 60).rounded(.up))
        case .elapsed(let t): Int((t / 60).rounded(.down))
        }
    }

    private static func wholeSeconds(_ value: Value) -> Int {
        switch value {
        case .remaining(let t): Int(t.rounded(.up))
        case .elapsed(let t): Int(t.rounded(.down))
        }
    }
}
