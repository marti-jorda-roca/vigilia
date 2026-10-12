# Vigilia: plan

## Context

[Insomnia](https://github.com/krishhgg/Insomnia) (MIT) is a macOS-only Swift menu bar app. It keeps a Mac awake for a timed session, can stay awake with the lid closed, and applies reversible "lid actions" (freeze chat apps, mute, screen off, Low Power Mode). Every change goes into a recovery journal so it can be undone after a crash.

Vigilia brings the same idea to **macOS, Linux and Windows**:

- One **Rust backend**: a per-user background service plus a CLI. It owns sessions, the journal and all OS actions.
- **Native UIs added later**, one per OS, written in that OS's own language (Swift on macOS). Each UI is a thin client of the service.
- **Later: talking to running coding agents while the laptop is closed**, from a phone or another machine. The architecture is shaped for this from day one, so adding it doesn't require rewriting the core.

Targets:
- **Battery efficiency is a design rule, not an optimisation pass.** With no session, Vigilia isn't running at all. During a session it wakes the CPU about once a minute at most, and Activity Monitor's "Energy Impact" stays near 0. Memory stays under 10 MB.
- Everything works from a terminal or a coding agent (`vigilia start 2h`) before any GUI exists.

## Goals and non-goals

**Goals (v1)**
- Timed sessions: start, extend, end early, status.
- Keep the machine awake while idle, and with the lid closed, on all three OSes.
- Lid actions with guaranteed undo: freeze apps, mute, screen/keyboard backlight off, power saver.
- Battery and heat rules: switch to power saver below a battery level, end the session below a lower level or at critical heat.
- Crash-safe recovery through a write-ahead journal and an OS-scheduled backstop.
- A stable, versioned message protocol whose request handling doesn't depend on the connection type, so native UIs and, later, remote clients all use it.
- Minimal battery use: everything is driven by events (no polling), the service starts on demand, and the session can end itself once agents are done (see Energy efficiency).

**Non-goals (v1)**
- GUIs. They come after the backend is stable (milestones M6–M7).
- Agent integration and remote access. They come later (M8–M10), but v1 includes the pieces they need: dispatch separate from connections, per-client permissions, and an internal event bus.
- Listening on any network interface. In v1 the service is local-only and owner-only.

## Architecture

```
                          ┌──────────────────── vigilia daemon (Rust) ──────────────────────┐
 CLI / native UIs ─local──┤ transports/local ──┐                                            │
                          │                    ├─► dispatch ─► session · journal · rules ───┼─► platform::macos
 phone / web ─remote──────┤ transports/remote ─┘   (ClientContext,         │                ├─► platform::linux
 (M9+, optional)          │                         scopes)                ▼                ├─► platform::windows
                          │                                            event bus ◄── agents │
                          │                                                │     (tmux,     │
                          │                                   subscribers ◄┘      hooks)    │
                          └─────────────────────────────────────────────────────────────────┘
```

- **One binary, `vigilia`.** `vigilia daemon` runs the service. Every other subcommand is a client.
- **The service starts on demand and exits when idle.**
  - On macOS and Linux, launchd/systemd socket activation starts it on the first connection.
  - It exits after a few minutes (configurable) with no session, no subscribers and remote access off.
  - Without socket activation, as on Windows or in a dev build, client commands start it detached.
  - Either way, agents never need a setup step, and no session means no process.
- **The service runs per user, not as root.** Muting, brightness, freezing apps and typing into agent terminals must happen inside the user's login session. Commands that need privileges go through a narrow, documented path for each OS (see Privilege model).
- **A backstop that only exists during a session.**
  - At session start, the service registers an OS-scheduled `vigilia recover` job (launchd, systemd user timer or Task Scheduler) that runs every 5 minutes. It removes the job at session end. Both steps are recorded in the journal.
  - `vigilia recover` also runs once at login.
  - If the service died after a session's end time, `recover` replays the journal's undo steps and removes the job.
- **Request handling is separate from connections.** `dispatch` takes `(ClientContext, Request)` and returns a response or a stream of events. It never knows whether the request arrived over the local socket, a tailnet listener or a chat bot. Adding a new kind of client means adding one transport module.
- **Every request carries a `ClientContext`.** `{ origin: Local | Remote(device_id), scopes }`. Local clients get every permission. Remote devices only get the scopes granted when they were paired. Permissions are checked in `dispatch`, so every transport gets the same checks.
- **One internal event bus.** Session changes, lid and power changes, warnings and agent events are all published on one `tokio::sync::broadcast` channel. Each subscriber (CLI `--watch`, a UI, a phone, a bot) registers a filter. Events are only sent when something changes; there is no periodic tick.

## Energy efficiency (design rule)

Vigilia exists to run during long sessions on battery, so it must cost almost nothing itself. Every feature follows these rules, and every release measures them.

**Rules**
1. **No process without a reason.** Start on demand, exit when idle (see Architecture). The backstop job exists only during a session.
2. **React to OS events instead of polling.** Lid, battery, thermal and network changes come from OS notifications:
   - IOKit and `NWPathMonitor` on macOS.
   - logind, UPower and NetworkManager signals on Linux.
   - `RegisterPowerSettingNotification` and `INetworkListManager` on Windows.

   Poll only when an OS has no notification for something, and then no more often than every 5 minutes.
3. **No periodic ticks.** The protocol sends `ends_at` and clients draw the countdown themselves. The service keeps one timer, for the next deadline (session end, or a rule's re-check), using coalescing timers where the OS supports it (leeway on macOS dispatch timers, `timerfd` slack on Linux).
4. **Read agent output only when asked.** Agent state comes from hook events. `tmux capture-pane` runs only for an explicit `agent.tail`. Agent rediscovery happens only while a client is subscribed to agent events.
5. **Keep the network quiet.** With remote access off there is no listener and no network activity at all. With it on, keepalive pings are sent only while a device is connected, at most once every 60 s. Push notifications go out only on real events. Clients receive pushed updates and never poll.
6. **Keep disk writes rare.** Logging is buffered and at info level by default. fsync happens only when the journal records an action, which is rare.
7. **Use the lightest OS mechanism.** With the lid open, hold only the idle-sleep assertion and let the display sleep. Never hold a display assertion. With the lid closed, the default lid actions (screen off, Low Power Mode, freeze chat apps) are on.
8. **UIs are cheap too.** Native UIs redraw the countdown once a minute, or once a second only while their menu or window is open. No animations in the menu bar item. App Nap is allowed on macOS.

**Ending the session when agents are done.** With the lid closed, agents use far more power than Vigilia does. The `auto_end` rule ends the session (and the Mac goes to sleep) once every tracked agent has reported `finished`, or no agent event has arrived for N minutes. It is opt-in, sends a notification first, and gives a short grace period during which any client can cancel. This rule arrives with agents (M8).

**Budgets checked in every milestone**

| State | Budget |
|---|---|
| No session, remote off | No Vigilia process running |
| Session active, lid open or closed | ≤ 1 Vigilia wake-up per minute on average; Activity Monitor Energy Impact ≈ 0; RSS < 10 MB |
| Session active, phone connected | ≤ 1 network keepalive per 60 s, plus real events |
| macOS menu bar app, menu closed | ≤ 1 redraw per minute; Energy Impact ≈ 0 |

**How it's measured**
- macOS: `sudo powermetrics --samplers tasks --show-process-energy` and `top -stats pid,command,cpu,idlew,power`.
- Linux: `powertop` and `perf stat -e power/energy-pkg/`.
- Windows: `powercfg /energy` and `powercfg /srumutil`.
- Results for each release go into `docs/energy.md`, so regressions show up release to release.

## Repository layout

```
vigilia/
  Cargo.toml                 # workspace
  crates/
    core/                    # no OS code. Session state machine, timer math, journal, rules, config
      src/session.rs
      src/journal.rs
      src/rules.rs           # battery and heat floors
      src/actions.rs         # Action trait: apply / undo / prove ownership
      src/config.rs
    protocol/                # request/response/event types, ClientContext, scopes; generated JSON Schema
      schema/protocol.schema.json
    platform/                # Platform trait, plus one module per OS behind cfg
      src/lib.rs             # trait + Capabilities
      src/fake.rs            # in-memory fake used in tests
      src/macos/  src/linux/  src/windows/
    daemon/
      src/dispatch.rs        # (ClientContext, Request) -> Response | event stream; permission checks
      src/bus.rs             # internal event bus with subscriber filters
      src/transports/local.rs   # Unix socket / named pipe
      src/lock.rs
    agents/                  # AgentAdapter trait, tmux adapter, hook ingest (M8)
    remote/                  # optional, feature "remote": tailnet listener, pairing, audit log (M9+)
    cli/                     # bin "vigilia" (clap)
  packaging/
    macos/   # LaunchAgent plists, sudoers template, install.sh
    linux/   # systemd user service and timer units, install.sh
    windows/ # Task Scheduler XML, install.ps1
  apps/                      # native UIs, added later
    macos/   # SwiftPM menu bar app (M6)
    linux/   # tray (M7)
    windows/ # (M7)
  .github/workflows/ci.yml   # build matrix for macOS, Linux and Windows
```

## Core concepts (crates/core)

- **Session**: `{id, started_at, ends_at, state: Active | Ending | Ended | NeedsAttention}`. Time is computed from the wall clock with a monotonic check, so system clock changes can't extend a session.
- **Action**: any reversible change to the OS. Each one has `apply()`, `undo()` and an `UndoRecord` (the state needed to restore, for example the previous brightness). Undo is idempotent.
- **Journal**: an append-only JSONL file. Write and fsync an `Intent` entry before calling `apply`, then append `Applied`, `Undone` or `Failed`. On startup, and in `recover`, replay every entry that isn't finished.
- **Ownership proof for frozen processes.** Record the pid **and the process start time**. Only send `SIGCONT` (or resume the process) when both still match. This guards against the OS reusing a pid. A process we can't prove is ours is reported to the user, never touched.
- **Rules**: evaluated only when an OS power, thermal or agent event arrives, with no fixed poll. Defaults, taken from Insomnia: power saver below 40% battery, end the session below 10%, power saver at serious heat, end at critical heat. `auto_end` (end when agents are done) is opt-in.
- **Lock**: a file lock shared by the service and `recover`, so only one of them changes state at a time.

### Rules that must never be broken
1. Write the journal entry before changing anything.
2. When a session ends, sleep is re-enabled. If that fails, the session becomes `NeedsAttention` and the failure is visible from every client.
3. Never resume a process we can't prove we froze.
4. Quitting the service is refused while undo work is still pending, unless `--force` is given, and that is logged.
5. Never freeze anything an agent session depends on: the terminal, tmux, the agent processes, or the remote connection.
6. Remote clients never get a raw shell. They can only send text to agent panes the user marked, and only within their granted scopes.

## Platform trait (crates/platform)

```rust
trait Platform {
    fn capabilities(&self) -> Capabilities;           // what this OS / machine supports
    fn keep_awake(&self, lid_closed_too: bool) -> Result<UndoRecord>;
    fn release_awake(&self, rec: &UndoRecord) -> Result<()>;
    fn lid_events(&self) -> Receiver<LidState>;
    fn power_events(&self) -> Receiver<PowerSnapshot>; // battery %, on AC, thermal level
    fn network_events(&self) -> Receiver<NetState>;    // online/offline; needed for remote access and failover
    fn freeze(&self, procs: &[ProcRef]) -> Result<Vec<FrozenProc>>;
    fn thaw(&self, procs: &[FrozenProc]) -> Result<()>;
    fn mute(&self) -> Result<UndoRecord>;
    fn backlight_off(&self) -> Result<UndoRecord>;
    fn power_saver(&self, on: bool) -> Result<UndoRecord>;
    fn list_processes(&self, matcher: &AppMatcher) -> Result<Vec<ProcRef>>;
    fn restore(&self, rec: &UndoRecord) -> Result<()>; // generic undo dispatch
}
```

`Capabilities` is returned to clients, so a UI can hide features the machine doesn't support instead of failing silently.

### How each backend works

| Feature | macOS | Linux | Windows |
|---|---|---|---|
| Stay awake while idle | IOPMAssertion `PreventUserIdleSystemSleep` | logind inhibitor `sleep:idle` | `SetThreadExecutionState(ES_SYSTEM_REQUIRED\|ES_CONTINUOUS)` |
| Stay awake with lid closed | `pmset -a disablesleep 1/0` (sudoers rule) | logind inhibitor `handle-lid-switch` (no root) | `powercfg` LIDACTION set to 0, old value journaled and restored |
| Lid events | IOKit `AppleClamshellState` notifications | logind `LidClosed` property over D-Bus | `RegisterPowerSettingNotification(GUID_LIDSWITCH_STATE_CHANGE)` |
| Freeze apps | `SIGSTOP`/`SIGCONT` | `SIGSTOP`/`SIGCONT` (cgroup freezer later) | `NtSuspendProcess`/`NtResumeProcess` (undocumented; behind a capability flag) |
| Mute | CoreAudio default output | PipeWire/Pulse (`pactl`, later libpulse) | `IAudioEndpointVolume` |
| Backlight off | DisplayServices/IOKit (private APIs) | `/sys/class/backlight` or logind `SetBrightness` | display power off via `SC_MONITORPOWER` |
| Power saver | `pmset -b lowpowermode` (sudoers) | power-profiles-daemon `power-saver` over D-Bus | Energy Saver / power scheme overlay |
| Battery and heat | `IOPSNotificationCreateRunLoopSource`, `ProcessInfo.thermalStateDidChangeNotification` | UPower `PropertiesChanged` signals; thermal zones polled ≤ every 5 min (no signal exists) | `RegisterPowerSettingNotification(GUID_BATTERY_PERCENTAGE_REMAINING)`; WMI thermal (best effort) |
| Network state | `NWPathMonitor` (Network framework) | NetworkManager over D-Bus | `INetworkListManager` events |
| Start on demand | launchd `Sockets` key (socket activation) | systemd `.socket` unit | Not available: started by the client; stays blocked doing nothing while idle |
| Backstop (session only) | LaunchAgent loaded at session start, `StartInterval` 300, removed at end | Transient `systemd-run --user --on-unit-active=5min` timer | Task Scheduler task created at start, deleted at end |

### Privilege model
- **macOS**: a sudoers file covering exactly the four `pmset` commands Insomnia uses. The installer shows it to the user before installing it. Later this could become an `SMAppService` privileged helper.
- **Linux**: no root needed. logind and polkit allow inhibitor locks for the active session user. Writing to backlight sysfs may need a udev rule; otherwise use logind `SetBrightness`.
- **Windows**: check whether `powercfg` changes to the active scheme need admin on target machines (open question). If they do, use a one-time elevated install that grants the user rights, or show a clear capability warning.

## Message protocol (crates/protocol)

- **Local transport**
  - macOS: `~/Library/Application Support/vigilia/vigilia.sock`
  - Linux: `$XDG_RUNTIME_DIR/vigilia.sock`
  - Windows: `\\.\pipe\vigilia-<user>`
  - Owner-only permissions. The service checks the connecting process's user id (`getpeereid` / `SO_PEERCRED`) and the pipe's security descriptor. These clients get `origin: Local` with every permission.
- **Framing**: one JSON object per line. Requests carry `id`, `v` (protocol version) and `method`. The same messages are carried unchanged over remote transports (WebSocket text frames, or wrapped in a bot bridge).
- **Methods are grouped by area:**
  - `hello`: returns protocol version, capabilities, and the scopes this client was granted.
  - `session.start {duration, lid_closed_awake}`, `session.extend {duration}`, `session.stop`, `session.status`
  - `config.get`, `config.set`
  - `recover`
  - `agent.list`, `agent.tail {agent, lines}`, `agent.send {agent, text, submit}` (M8)
  - `device.list`, `device.revoke` (M9; local clients only)
  - `subscribe {filter}`: the filter is a list of event kinds and, optionally, agent ids.
- **Events**: `session_changed {session}` (includes `ends_at`; clients compute the remaining time locally, and there is no periodic tick), `lid_changed`, `power_changed`, `network_changed`, `warning {kind, detail}`, `needs_attention`, `agent_event {agent, kind: needs_input | finished | error, summary}`.
- **Scopes**: `session.read`, `session.control`, `agent.read`, `agent.send:<agent-id or *>`, `config.write`, `admin`. `dispatch` rejects any method the client's scopes don't cover with a typed `forbidden` error.
- A JSON Schema is generated with `schemars` into `crates/protocol/schema/`. This is the contract for the Swift, Windows and phone clients, and CI fails if the schema drifts from the Rust types.

Example:
```json
{"id":1,"v":1,"method":"session.start","params":{"duration":"2h","lid_closed_awake":true}}
{"id":1,"ok":true,"result":{"session":{"id":"…","ends_at":"2026-10-12T02:00:00Z","state":"Active"}}}
{"event":"agent_event","agent":"tmux:work:1.0","kind":"needs_input","summary":"Allow edit to src/main.rs?"}
```

## Agents (crates/agents, M8)

Insomnia's tmux nudge, which types `continue` into a pane, is one case of "talk to an agent". Vigilia treats agents as a first-class part of the service:

```rust
trait AgentAdapter {
    fn discover(&self) -> Vec<AgentRef>;                 // running agent sessions
    fn tail(&self, a: &AgentRef, lines: usize) -> Result<String>;
    fn send(&self, a: &AgentRef, text: &str, submit: bool) -> Result<()>;
    fn events(&self) -> Receiver<AgentEvent>;            // needs_input, finished, error
}
```

- **tmux adapter first.** It works with any terminal agent (Claude Code, Codex, Aider and others). It only touches panes the user marked with `tmux set-option -p @vigilia-agent on`, the same opt-in model as Insomnia.
- **Agents report in through hooks.** Agents call `vigilia agent notify --id <id> --kind needs_input --summary "…"` to push events into the bus. Claude Code's `Notification` and `Stop` hooks map onto this directly. Other agents can call the same command from a wrapper script.
- Agent events are published on the event bus, so local UIs, the CLI and later remote clients all get them the same way.
- The agents crate never needs to know about network transports.
- **No polling:**
  - Agent state comes from hooks.
  - Pane output is captured only for `agent.tail`.
  - Panes are rescanned only while a client is subscribed to agent events.
- **`auto_end` rule (opt-in):** once all tracked agents report `finished`, or none has sent an event for `idle_minutes`, Vigilia notifies, waits a grace period (default 2 min, any client can cancel), then ends the session so the machine can sleep.

## Remote access (crates/remote, M9–M10, optional)

Built only with `--features remote` and off by default in config. The default build stays small and opens no network listener.

| Option | How it works | When |
|---|---|---|
| **Tailscale / WireGuard listener** | The service listens on the private tailnet address only and speaks the same protocol over a WebSocket. The phone joins the same tailnet. | M9, first: no server to run, encrypted end to end |
| **Messaging bridge** (Telegram, Slack, ntfy) | The service opens an outbound connection to a bot. `agent_event`s become messages, and replies become `agent.send` requests. | M10: best phone experience, but the third party sees message contents |
| **Relay server** | The service holds an outbound WebSocket to a relay. Traffic is end-to-end encrypted (Noise), so the relay can't read it. | M10+, only if a no-VPN option is needed |

**Security model for every remote transport:**
- Pairing: `vigilia remote pair` shows a QR code with a one-time token. The device registers its own key, and the user picks the scopes it gets.
- Each device has its own key, scopes and expiry. `vigilia remote revoke <device>` takes effect immediately.
- Every remote request is written to an append-only audit log (`remote-audit.jsonl`) with the device, method and result.
- Remote requests are rate-limited per device.
- No raw shell: `agent.send` only reaches marked agent panes.

**Energy:**
- Remote off: no listener and no network traffic.
- Remote on: the listener sits idle, and keepalives (≥ 60 s apart) only run while a device is connected.
- Push notifications are sent only for real events.

**Lid-closed networking:**
- Network loss is reported as `network_changed` and shown as a warning to clients.
- Wi-Fi/hotspot failover (from Insomnia) is included in M9, since remote access depends on staying online.
- Windows laptops with Modern Standby may drop Wi-Fi with the lid closed. Check this on real hardware, and report it as a capability if it can't be prevented.

## CLI

```
vigilia start 2h [--lid] [--no-lid-actions]
vigilia extend 30m
vigilia stop
vigilia status [--json] [--watch]
vigilia config get|set|edit
vigilia recover            # backstop entry point; also safe to run by hand
vigilia install|uninstall  # backstop job, sudoers rule (macOS), units (Linux)
vigilia daemon             # run the service in the foreground
vigilia debug simulate-lid close|open

# M8
vigilia agent list
vigilia agent tail <agent> [-n 50]
vigilia agent send <agent> "text" [--submit]
vigilia agent notify --id <id> --kind needs_input|finished|error [--summary "…"]   # called by agent hooks

# M9 (feature "remote")
vigilia remote enable|disable
vigilia remote pair [--scopes session.read,agent.read,agent.send:*]
vigilia remote devices
vigilia remote revoke <device>
```

`--json` output uses the protocol types, so scripts and agents get the same shape the UIs do.

## Config

TOML at the standard per-user location for each OS (`directories` crate). The defaults match Insomnia: freeze list Slack, WhatsApp and Discord; mute on; Docker rule off; power saver below 40%; end below 10%; heat rules on. The "never freeze" list covers:
- Meeting and recording apps: Zoom, Teams, Webex, FaceTime, OBS, Loom and similar.
- Terminals, tmux, known agent CLIs, and the Tailscale/VPN client. These are always added, even if the user edits the list.

`[agents]` and `[remote]` sections are added in M8 and M9. Remote access is disabled by default.

```toml
[energy]
idle_exit_minutes = 5          # service exits after this long with no session, subscribers or remote

[agents.auto_end]              # M8, opt-in
enabled = false
idle_minutes = 20
grace_minutes = 2
```

## Dependencies (kept small)

`serde`, `serde_json`, `toml`, `clap`, `directories`, `tracing` + `tracing-appender`, `fs4` (file lock), `tokio` with the current-thread runtime only, `schemars` (dev and build only).
Per OS: `objc2` / `objc2-io-kit` / `core-foundation` (macOS); `zbus` and `nix` (Linux); `windows` crate (Windows).
Only with the `remote` feature: `tokio-tungstenite` (WebSocket) and `rustls`; later `snow` (Noise) for the relay option.
Release builds use LTO, `opt-level = "s"` and `panic = "abort"` in the daemon profile.

## Milestones

| # | What gets built | Done when |
|---|---|---|
| M0 | Workspace scaffold, CI matrix (fmt, clippy, test on macOS, Linux and Windows), `plan.md` | CI is green on all three OSes |
| M1 | `core`: session, journal, rules, config, plus the `FakePlatform` | Unit and property tests pass, including journal replay after a simulated crash at every step |
| M2 | `daemon` with `dispatch`, `ClientContext` + scopes, event bus and local transport; socket activation and idle exit; `protocol`; `cli`; keep-awake while idle on all 3 OSes | `vigilia start 1m` blocks idle sleep, then releases it on time; `status --watch` shows a countdown computed locally from `ends_at`; a test with a remote-origin context and limited scopes gets `forbidden`; no process runs 5 min after the session ends; ≤ 1 wake-up/min during a session |
| M3 | Lid-closed awake + lid events on macOS and Linux; `install`, and a `recover` backstop that exists only during a session | Lid closed for 5 minutes during a session and the machine stays awake; kill the service mid-session and the backstop restores sleep after the end time; no backstop job exists outside a session |
| M4 | Lid actions on macOS and Linux (freeze, mute, backlight, power saver) + battery and heat rules | Each action applies on lid close and undoes on lid open; crash tests leave nothing behind |
| M5 | Windows backend (lid, freeze, mute, power saver) | Same checks as M3 and M4 on a real Windows laptop, including a Modern Standby machine |
| M6 | `apps/macos` Swift menu bar app (countdown in the menu bar, start/extend/hold-to-end, settings) using the protocol | The app talks to the service; quitting the app doesn't end the session |
| M7 | Linux tray (Rust `ksni` or GTK) and Windows UI (WinUI/C#, or Rust) | Same as M6 on each OS |
| M8 | `agents`: adapter trait, tmux adapter, `agent notify` hook ingest, `agent.*` methods and events; agent panes added to "never freeze"; `auto_end` rule | A marked Claude Code pane shows up in `agent list`; its `Notification` hook produces an `agent_event` on `status --watch`; `agent send` types into the pane, locally only; with `auto_end` on, the session ends after the agent's `Stop` hook plus the grace period; no `tmux` processes are spawned while nobody is subscribed |
| M9 | `remote` (feature-gated): tailnet WebSocket listener, pairing, per-device scopes, revoke, audit log, rate limits; `network_events`; Wi-Fi/hotspot failover; hotspot password stored with `keyring` | With the lid closed, a paired phone on the tailnet receives an `agent_event` and answers with `agent send`; a revoked device is refused; every call appears in the audit log |
| M10 | Messaging bridge (Telegram or ntfy first); relay server only if needed | An agent `needs_input` arrives as a chat message, and a reply reaches the pane |

## Testing strategy

- **Core**: unit tests and `proptest` against `FakePlatform`. Inject crashes between journal entries and check that recovery always ends in a clean state.
- **Daemon**: integration tests that start the service on a temporary socket with `FakePlatform`, then drive it with protocol messages (start, extend, stop, subscribe, lid simulation). Dispatch tests call it directly with `Remote` contexts to check every method against every scope.
- **Agents**: CI on macOS and Linux starts a real tmux server with a marked pane running a fake agent script. Tests cover discover, tail, send and hook ingest, and check that unmarked panes are never touched.
- **Remote**: a loopback listener with a test device key covers pairing, scope checks, revocation, rate limits and audit log entries.
- **Platform**: thin per-OS smoke tests in CI where possible (power assertions, process listing, D-Bus reachability). Anything that needs real hardware is `#[ignore]` and covered by the manual checklist.
- **Energy**:
  - A daemon integration test counts timer wake-ups over a simulated 1-hour session with `FakePlatform` and a mocked clock, and fails above 60.
  - Real-hardware numbers from `powermetrics` / `powertop` / `powercfg` are recorded in `docs/energy.md` for each release.
- **Manual hardware checklist** (`docs/testing.md`), per OS: lid-closed session, battery floor, crash recovery, audio restore after switching outputs, freeze ownership after a pid is reused, network staying up with the lid closed.

## Verification (end to end)

1. `cargo test --workspace` is green on all three OSes in CI.
2. `vigilia start 2m && vigilia status --watch`: the countdown streams and the session ends on its own.
3. While the session runs: `pmset -g assertions` (macOS), `systemd-inhibit --list` (Linux) or `powercfg /requests` (Windows) shows Vigilia; after the end it's gone.
4. `vigilia debug simulate-lid close`: Slack is stopped (`ps -o stat` shows `T`) and audio is muted, while tmux and the agent panes keep running. `simulate-lid open` restores everything.
5. `kill -9` the service mid-session, wait past the end time, and confirm the backstop restored sleep settings and logged it.
6. Energy:
   - With no session, no `vigilia` process exists 5 minutes after the last command.
   - During a 30-minute lid-closed session, `powermetrics --samplers tasks` shows ≤ 1 Vigilia wake-up per minute on average and Energy Impact ≈ 0, and RSS stays under 10 MB.
   - Repeat with remote enabled and a phone connected.
7. (M8) Run Claude Code in a marked tmux pane with a `Notification` hook calling `vigilia agent notify`. The event appears in `status --watch`, and `vigilia agent send` answers it.
8. (M9) Close the lid, then use a paired phone on the tailnet to read status, receive the agent event and reply. Revoke the phone and confirm the next request is refused and logged.

## Open questions and risks

- Can `powercfg` change the lid action without admin on Windows? Prototype this early in M5.
- macOS backlight control depends on private APIs that may break between macOS versions. Treat it as best effort, behind a capability flag.
- GNOME hides tray icons without the AppIndicator extension. This affects the M7 Linux UI choice, not the backend.
- `NtSuspendProcess` is undocumented. Ship it behind a capability flag and keep freezing optional on Windows.
- Does Wi-Fi stay connected with the lid closed? Check on each OS, especially Windows Modern Standby, before promising remote access there.
- Claude Code has its own remote-control feature. Vigilia's agent layer should work with any agent and stay complementary, not duplicate agent-specific features.
- Socket activation and idle exit make the first command after a pause slightly slower (the service has to start). Keep startup under ~50 ms; measure it in M2.
- Windows has no socket activation, so the service stays resident there. It must truly block (no timers) while idle.
- Remote access is the largest security risk. It stays opt-in, behind a build feature, scoped per device, audited, and never offers a raw shell.
- Name and license: the plan assumes the name `vigilia` and the Apache-2.0 license, with credit to Insomnia for the concept.
