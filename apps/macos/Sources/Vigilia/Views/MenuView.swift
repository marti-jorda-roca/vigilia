import AppKit
import SwiftUI
import VigiliaProtocol

/// The popover under the menu bar item.
struct MenuView: View {
    @ObservedObject var store: SessionStore
    let openSettings: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
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
                    .font(.caption)
                    .foregroundColor(.red)
                    .fixedSize(horizontal: false, vertical: true)
            }
            Divider()
            HStack {
                Button("Settings…", action: openSettings)
                Spacer()
                Button("Quit") { NSApp.terminate(nil) }
                    .help("Quit the menu bar app. A running session continues.")
            }
            .buttonStyle(.borderless)
        }
        .padding(14)
        .frame(width: 290)
    }
}

// MARK: - No session

private struct StartView: View {
    @ObservedObject var store: SessionStore
    let openSettings: () -> Void
    @AppStorage(Preferences.lidClosedByDefault) private var lidClosedByDefault = false
    @State private var lidClosed: Bool?
    @State private var custom = false
    @State private var days = 0
    @State private var hours = 1
    @State private var minutes = 0

    private static let presets: [(String, Int?)] = [
        ("15m", 900), ("30m", 1_800), ("1h", 3_600), ("2h", 7_200), ("4h", 14_400), ("Until stopped", nil),
    ]

    private var lidAvailable: Bool {
        store.capabilities.keepAwakeLidClosed && store.setup.sudoersInstalled
    }

    private var lidChoice: Binding<Bool> {
        Binding(get: { lidClosed ?? lidClosedByDefault }, set: { lidClosed = $0 })
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text("Keep this Mac awake").font(.headline)
            LazyVGrid(columns: [GridItem(.adaptive(minimum: 58), spacing: 6)], spacing: 6) {
                ForEach(Self.presets.prefix(5), id: \.0) { label, seconds in
                    Button(label) { start(seconds) }
                }
            }
            Button(Self.presets[5].0) { start(nil) }
                .frame(maxWidth: .infinity)

            DisclosureGroup("Custom", isExpanded: $custom) {
                HStack(spacing: 4) {
                    picker($days, 0..<8, "d")
                    picker($hours, 0..<24, "h")
                    picker($minutes, Array(stride(from: 0, to: 60, by: 5)), "m")
                    Button("Start") { start(days * 86_400 + hours * 3_600 + minutes * 60) }
                        .disabled(days + hours + minutes == 0)
                }
                .padding(.top, 4)
            }

            if store.capabilities.keepAwakeLidClosed {
                if lidAvailable {
                    Toggle("Keep awake with the lid closed", isOn: lidChoice)
                } else {
                    HStack {
                        Toggle("Keep awake with the lid closed", isOn: .constant(false)).disabled(true)
                        Spacer()
                        Button("Set up…", action: openSettings).buttonStyle(.link)
                    }
                }
            }
        }
        .disabled(store.busy)
    }

    private func picker<S: Sequence>(_ value: Binding<Int>, _ range: S, _ unit: String) -> some View
    where S.Element == Int {
        Picker(unit, selection: value) {
            ForEach(Array(range), id: \.self) { Text("\($0)\(unit)").tag($0) }
        }
        .labelsHidden()
        .frame(width: 62)
    }

    private func start(_ seconds: Int?) {
        store.start(duration: seconds, lidClosedAwake: lidAvailable && lidChoice.wrappedValue)
        lidClosed = nil
    }
}

// MARK: - Session running

private struct SessionView: View {
    @ObservedObject var store: SessionStore
    let session: Session

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            // Seconds only while the menu is open: the view goes away, and its timer with it,
            // when the popover closes.
            if store.menuOpen {
                TimelineView(.periodic(from: .now, by: 1)) { context in
                    Text(Countdown.longLabel(Countdown.value(for: session, at: context.date)))
                        .font(.system(size: 30, weight: .semibold, design: .rounded))
                        .monospacedDigit()
                }
            }
            Text(caption).font(.caption).foregroundColor(.secondary)
            if session.lidClosedAwake {
                Label("Stays awake with the lid closed", systemImage: "laptopcomputer")
                    .font(.caption)
            }
            if session.endsAt != nil {
                HStack {
                    ForEach([("+15m", 900), ("+30m", 1_800), ("+1h", 3_600)], id: \.0) { label, seconds in
                        Button(label) { store.extend(seconds: seconds) }
                            .frame(maxWidth: .infinity)
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
            guard let endsAt = session.endsAt else { return "Awake until you end it" }
            return "Awake until \(endsAt.formatted(date: .omitted, time: .shortened))"
        }
    }
}

// MARK: - Alerts

private struct NeedsAttentionView: View {
    let detail: String
    let busy: Bool
    let retry: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            Label("Normal sleep may not be restored", systemImage: "exclamationmark.triangle.fill")
                .foregroundColor(.orange)
                .font(.headline)
            Text(detail).font(.caption).fixedSize(horizontal: false, vertical: true)
            Button("Retry", action: retry).disabled(busy)
        }
        .padding(10)
        .background(RoundedRectangle(cornerRadius: 8).fill(Color.orange.opacity(0.12)))
    }
}

private struct WarningRow: View {
    let warning: SessionStore.Warning
    let dismiss: () -> Void

    var body: some View {
        HStack(alignment: .top) {
            Image(systemName: "exclamationmark.circle").foregroundColor(.orange)
            Text(warning.detail).font(.caption).fixedSize(horizontal: false, vertical: true)
            Spacer()
            Button(action: dismiss) { Image(systemName: "xmark") }
                .buttonStyle(.borderless)
                .accessibilityLabel("Dismiss")
        }
    }
}

enum Preferences {
    /// App-only: the lid toggle's starting value. The CLI and protocol don't know about it.
    static let lidClosedByDefault = "lidClosedByDefault"
}
