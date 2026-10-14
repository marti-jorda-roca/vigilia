import AppKit
import SwiftUI
import VigiliaProtocol

/// The menu under the lantern.
struct MenuView: View {
    @ObservedObject var store: SessionStore
    let openSettings: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: Pixel.unit * 6) {
            Header(state: lanternState, status: status)
            if let detail = store.needsAttention {
                NeedsAttentionView(detail: detail, busy: store.busy) { store.stop() }
            }
            ForEach(store.warnings) { warning in
                WarningRow(warning: warning) { store.dismiss(warning) }
            }
            if let session = store.session, session.state != .needsAttention {
                SessionView(store: store, session: session)
            } else if store.session == nil {
                StartView(store: store, openSettings: openSettings)
            }
            if let error = store.lastError {
                Text(error)
                    .foregroundColor(Pixel.red)
                    .fixedSize(horizontal: false, vertical: true)
            }
            PixelRule(color: Pixel.faint)
            HStack {
                Button(action: openSettings) { PixelText("SETTINGS") }
                Spacer()
                Button { NSApp.terminate(nil) } label: { PixelText("QUIT") }
                    .help("Quit the menu bar app. A running session continues.")
            }
            .buttonStyle(PixelButtonStyle(kind: .plain))
        }
        .padding(Pixel.unit * 8)
        .frame(width: 288)
        .background(
            ZStack(alignment: .top) {
                PixelBox()
                // Stars only behind the header, where nothing else is drawn.
                PixelStars().frame(height: 40).padding(Pixel.unit * 2)
            }
        )
        .pixelSurface()
    }

    private var lanternState: Lantern.State {
        store.hasAlert ? .alert : (store.session != nil ? .lit : .unlit)
    }

    private var status: String {
        switch store.connection {
        case .reconnecting: return "RECONNECTING"
        case .connecting where store.session == nil: return "…"
        default: return store.session != nil ? "AWAKE" : "ASLEEP"
        }
    }
}

private struct Header: View {
    let state: Lantern.State
    let status: String

    var body: some View {
        HStack(alignment: .center, spacing: Pixel.unit * 5) {
            LanternView(state: state)
            PixelText("VIGILIA", scale: Pixel.unit * 1.5, color: Pixel.orange)
            Spacer()
            PixelText(status, color: state == .unlit ? Pixel.dim : Pixel.yellow)
        }
    }
}

// MARK: - No session

private struct StartView: View {
    @ObservedObject var store: SessionStore
    let openSettings: () -> Void
    @AppStorage(Preferences.lidClosedByDefault) private var lidClosedByDefault = false
    @State private var lidClosed: Bool?
    @State private var timeout = ""
    @FocusState private var timeoutFocused: Bool

    private static let chips = ["30m", "1h", "2h", "4h"]

    private var lidAvailable: Bool {
        store.capabilities.keepAwakeLidClosed && store.setup.sudoersInstalled
    }

    private var lidChoice: Binding<Bool> {
        Binding(get: { lidClosed ?? lidClosedByDefault }, set: { lidClosed = $0 })
    }

    /// Seconds, `nil` for no timeout, or a parse error to show.
    private var parsed: Result<Int?, ParseError> {
        let text = timeout.trimmingCharacters(in: .whitespaces)
        if text.isEmpty { return .success(nil) }
        guard let seconds = HumanDuration.parse(text), seconds > 0 else { return .failure(ParseError()) }
        return .success(seconds)
    }

    private struct ParseError: Error {}

    var body: some View {
        VStack(alignment: .leading, spacing: Pixel.unit * 5) {
            PixelText("TIMEOUT", color: Pixel.dim)
            TextField("", text: $timeout, prompt: Text("none: hold to end").foregroundColor(Pixel.dim))
                .textFieldStyle(.plain)
                .font(.system(size: 13, design: .monospaced))
                .focused($timeoutFocused)
                .onSubmit(start)
                .padding(.horizontal, Pixel.unit * 4)
                .padding(.vertical, Pixel.unit * 3)
                .background(PixelBox(border: timeoutFocused ? Pixel.orange : Pixel.ink))
                .accessibilityLabel("Timeout")
            HStack(spacing: Pixel.unit * 3) {
                ForEach(Self.chips, id: \.self) { chip in
                    let selected = HumanDuration.parse(timeout) == HumanDuration.parse(chip)
                    Button {
                        timeout = selected ? "" : chip
                    } label: {
                        PixelText(chip.uppercased()).frame(maxWidth: .infinity)
                    }
                    .buttonStyle(PixelButtonStyle(kind: selected ? .primary : .normal))
                    .accessibilityAddTraits(selected ? .isSelected : [])
                }
            }
            Group {
                switch parsed {
                case .failure:
                    Text("Use a duration like 2h, 45m or 1h30m.").foregroundColor(Pixel.red)
                case .success(nil):
                    Text("Stays awake until you hold to end.").foregroundColor(Pixel.dim)
                case let .success(seconds?):
                    Text("Ends at \(Date().addingTimeInterval(TimeInterval(seconds)).formatted(date: .omitted, time: .shortened)).")
                        .foregroundColor(Pixel.dim)
                }
            }
            .fixedSize(horizontal: false, vertical: true)

            if store.capabilities.keepAwakeLidClosed {
                if lidAvailable {
                    Toggle(isOn: lidChoice) { PixelText("LID CLOSED") }
                        .toggleStyle(PixelToggleStyle())
                } else {
                    HStack {
                        Toggle(isOn: .constant(false)) { PixelText("LID CLOSED") }
                            .toggleStyle(PixelToggleStyle())
                            .disabled(true)
                        Spacer()
                        Button(action: openSettings) { PixelText("SET UP", color: Pixel.orange) }
                            .buttonStyle(PixelButtonStyle(kind: .plain))
                    }
                }
            }

            Button(action: start) {
                PixelText(startTitle, scale: Pixel.unit * 1.5).frame(maxWidth: .infinity)
            }
            .buttonStyle(PixelButtonStyle(kind: .primary))
            .disabled(isInvalid)
        }
        .disabled(store.busy)
    }

    private var isInvalid: Bool {
        if case .failure = parsed { return true }
        return false
    }

    private var startTitle: String {
        if case let .success(seconds?) = parsed {
            return "START · " + HumanDuration.format(seconds: seconds)
        }
        return "START"
    }

    private func start() {
        guard case let .success(seconds) = parsed else { return }
        store.start(duration: seconds, lidClosedAwake: lidAvailable && lidChoice.wrappedValue)
        lidClosed = nil
        timeout = ""
    }
}

// MARK: - Session running

private struct SessionView: View {
    @ObservedObject var store: SessionStore
    let session: Session

    var body: some View {
        VStack(alignment: .leading, spacing: Pixel.unit * 5) {
            PixelText(session.endsAt == nil ? "AWAKE FOR" : "TIME LEFT", color: Pixel.dim)
            // Seconds only while the menu is open: the view goes away, and its timer with it,
            // when the menu closes.
            if store.menuOpen {
                TimelineView(.periodic(from: .now, by: 1)) { context in
                    let label = Countdown.longLabel(Countdown.value(for: session, at: context.date))
                    PixelText(label, scale: label.count > 10 ? Pixel.unit * 1.5 : Pixel.unit * 2, color: Pixel.yellow)
                }
            }
            Text(caption).foregroundColor(Pixel.dim)
            if session.lidClosedAwake {
                Text("Stays awake with the lid closed.").foregroundColor(Pixel.dim)
            }
            if session.endsAt != nil {
                HStack(spacing: Pixel.unit * 3) {
                    ForEach([("+15M", 900), ("+30M", 1_800), ("+1H", 3_600)], id: \.0) { label, seconds in
                        Button { store.extend(seconds: seconds) } label: {
                            PixelText(label).frame(maxWidth: .infinity)
                        }
                        .buttonStyle(PixelButtonStyle())
                    }
                }
                .disabled(store.busy || session.state != .active)
            }
            HoldToEndButton { store.stop() }
                .disabled(store.busy || session.state != .active)
        }
    }

    private var caption: String {
        switch session.state {
        case .ending:
            return "Ending…"
        default:
            guard let endsAt = session.endsAt else { return "No timeout. Hold to end." }
            return "Ends at \(endsAt.formatted(date: .omitted, time: .shortened))."
        }
    }
}

// MARK: - Alerts

private struct NeedsAttentionView: View {
    let detail: String
    let busy: Bool
    let retry: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: Pixel.unit * 4) {
            PixelText("SLEEP NOT RESTORED", color: Pixel.red)
            Text(detail).fixedSize(horizontal: false, vertical: true)
            Button(action: retry) { PixelText("RETRY") }
                .buttonStyle(PixelButtonStyle())
                .disabled(busy)
        }
        .padding(Pixel.unit * 5)
        .background(PixelBox(border: Pixel.red))
    }
}

private struct WarningRow: View {
    let warning: SessionStore.Warning
    let dismiss: () -> Void

    var body: some View {
        HStack(alignment: .top, spacing: Pixel.unit * 4) {
            PixelText("!", color: Pixel.orange)
            Text(warning.detail).fixedSize(horizontal: false, vertical: true)
            Spacer(minLength: 0)
            Button(action: dismiss) { PixelText("X", color: Pixel.dim) }
                .buttonStyle(PixelButtonStyle(kind: .plain))
                .accessibilityLabel("Dismiss")
        }
    }
}

enum Preferences {
    /// App-only: the lid toggle's starting value. The CLI and protocol don't know about it.
    static let lidClosedByDefault = "lidClosedByDefault"
}
