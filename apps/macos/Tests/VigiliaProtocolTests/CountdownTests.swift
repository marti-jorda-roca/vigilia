import Foundation
import Testing

@testable import VigiliaProtocol

struct CountdownTests {
    let start = Date(timeIntervalSince1970: 1_800_000_000)

    private func timed(_ seconds: TimeInterval, state: SessionState = .active) -> Session {
        Session(id: "s", startedAt: start, endsAt: start.addingTimeInterval(seconds), state: state)
    }

    private var indefinite: Session {
        Session(id: "s", startedAt: start, endsAt: nil, state: .active)
    }

    @Test func remainingRoundsUpElapsedRoundsDown() {
        let session = timed(600)
        #expect(Countdown.shortLabel(Countdown.value(for: session, at: start)) == "10m")
        #expect(Countdown.shortLabel(Countdown.value(for: session, at: start + 0.4)) == "10m")
        #expect(Countdown.shortLabel(Countdown.value(for: session, at: start + 60)) == "9m")
        #expect(Countdown.shortLabel(Countdown.value(for: session, at: start + 599)) == "1m")
        #expect(Countdown.shortLabel(Countdown.value(for: session, at: start + 700)) == "0m")

        #expect(Countdown.shortLabel(Countdown.value(for: indefinite, at: start + 59)) == "0m")
        #expect(Countdown.shortLabel(Countdown.value(for: indefinite, at: start + 61)) == "1m")
    }

    @Test func shortLabels() {
        #expect(Countdown.shortLabel(.remaining(83 * 60)) == "1h 23m")
        #expect(Countdown.shortLabel(.remaining(2 * 86_400 + 3 * 3_600)) == "2d 3h")
        #expect(Countdown.shortLabel(.elapsed(3_600)) == "1h 0m")
    }

    @Test func longLabels() {
        #expect(Countdown.longLabel(.remaining(83 * 60 + 5)) == "1h 23m 05s")
        #expect(Countdown.longLabel(.remaining(4 * 60 + 8.2)) == "4m 09s")
        #expect(Countdown.longLabel(.elapsed(42.9)) == "42s")
        #expect(Countdown.longLabel(.remaining(86_400 + 61)) == "1d 0h 01m 01s")
    }

    @Test func nextChangeForATimedSession() {
        let session = timed(600)
        // 10m shows until 9 minutes are left.
        #expect(Countdown.nextLabelChange(for: session, after: start) == start + 60)
        #expect(Countdown.nextLabelChange(for: session, after: start + 30) == start + 60)
        #expect(Countdown.nextLabelChange(for: session, after: start + 60) == start + 120)
        // In the last minute the next change is the end itself.
        #expect(Countdown.nextLabelChange(for: session, after: start + 590) == start + 600)
        #expect(Countdown.nextLabelChange(for: session, after: start + 600) == nil)
    }

    @Test func nextChangeForAnIndefiniteSession() {
        #expect(Countdown.nextLabelChange(for: indefinite, after: start) == start + 60)
        #expect(Countdown.nextLabelChange(for: indefinite, after: start + 61) == start + 120)
    }

    @Test func nothingToRedrawWhenNotActive() {
        #expect(Countdown.nextLabelChange(for: timed(600, state: .ended), after: start) == nil)
        #expect(Countdown.nextLabelChange(for: timed(600, state: .needsAttention), after: start) == nil)
    }
}
