# Vigilia: execution plan

This is the build order for the architecture in [`plan.md`](plan.md). Each step ends with a release you can install and use. Each step adds to the previous one without rewriting it.

## Rules for every step

- **Every step ends in a release that works.** Tag it (`v0.1`, `v0.2`, …). `main` is never left half-done.
- **CI builds macOS, Linux and Windows from step 1.** Backends that don't exist yet are stubs that report "unsupported" through `Capabilities`. Porting to an OS later means filling in a module, not restructuring.
- **macOS first, because that's the machine we use every day.** The shared layers (core, protocol, daemon, CLI, agents, remote) are written once. Linux and Windows only add a `platform` backend and packaging.
- **Protocol changes only add things.** New methods, events and optional fields are allowed. Removing or renaming anything requires a version bump. The JSON Schema check in CI enforces this.
- **New features sit behind a capability, a config flag or a build feature.** That way they can be shipped off by default and turned on once proven.
- **Every action that changes the OS goes through the journal.** No exceptions, including during prototyping.
- **Meeting the energy budget is part of "done".** Each step meets the budgets in [`plan.md` → Energy efficiency](plan.md#energy-efficiency-design-rule) and records the measured numbers in `docs/energy.md`. Two rules apply everywhere: react to OS events instead of polling, and never send periodic ticks.

## Overview

| Step | Release | What the app can do afterwards |
|---|---|---|
| 1 | v0.1 | `vigilia start 2h` keeps a Mac awake for a set time, from the CLI or from an agent, and costs nothing when no session runs |
| 2 | v0.2 | The Mac stays awake **with the lid closed**, safely: crash recovery, battery and heat limits |
| 3 | v0.3 | A macOS menu bar app with a countdown, start/extend/end and settings |
| 4 | v0.4 | Vigilia knows about your running agents, notifies you when one needs input, and can end the session once they're done so the Mac sleeps |
| 5 | v0.5 | **Reach your agents from your phone while the lid is closed** |
| 6 | v0.6 | Lid actions save battery and quiet the Mac: freeze apps, mute, screen off, Low Power Mode |
| 7 | v0.7 | Everything above works on Linux |
| 8 | v0.8 | Everything above works on Windows |
| 9 | v0.9 | Native tray UIs on Linux and Windows |
| 10 | v1.0 | Extras: two-way chat bridge, Wi-Fi/hotspot failover, relay option |

---

## Step 1: Daemon and CLI that keep the Mac awake (v0.1)

**What the app can do:** `vigilia start 2h`, `extend 30m`, `stop`, `status --watch`. The Mac doesn't idle-sleep while a session runs and goes back to normal when it ends. An agent can call `vigilia start` itself before a long task.

**What to build:**
- Cargo workspace with `crates/core`, `protocol`, `platform`, `daemon`, `cli`. CI on all three OSes runs fmt, clippy and tests.
- `core`:
  - Session state machine and timer math.
  - Journal: append-only JSONL, fsync before acting, replay on startup.
  - Config loading with defaults.
- `protocol`:
  - Request, response and event types.
  - `ClientContext { origin, scopes }`.
  - JSON Schema generation, with a CI check for drift.
- `platform`:
  - The `Platform` trait and `Capabilities`.
  - `FakePlatform` for tests.
  - macOS backend: only the `PreventUserIdleSystemSleep` power assertion.
  - Linux and Windows: stubs.
- `daemon`:
  - `dispatch.rs`, with permission checks in place even though only local clients exist.
  - `bus.rs`, the event bus with subscriber filters.
  - `transports/local.rs`: owner-only Unix socket with a peer user-id check.
  - `lock.rs`.
- Energy, built in from the start:
  - **Socket activation:** a LaunchAgent with a `Sockets` key starts the service on the first connection. The service exits after `[energy] idle_exit_minutes` (default 5) with no session and no subscribers.
  - **No `tick` event:** `session_changed` carries `ends_at`, and clients compute the remaining time themselves.
  - **One deadline timer**, with leeway so the OS can group wake-ups.
  - **Buffered, info-level logging.** fsync only when the journal records an action.
- `cli`: commands `start`, `extend`, `stop`, `status [--json] [--watch]` and `daemon`.
  - `status --watch` draws the countdown locally from `ends_at`.
  - Client commands start the service themselves when socket activation isn't installed (dev builds).

**Built for later changes:**
- Dispatch, `ClientContext` and the event bus exist from day one. Steps 3, 4 and 5 plug into them.
- The journal already wraps the power assertion, so step 2's riskier actions use the same path.

**Done when:**
- `vigilia start 2m` shows Vigilia in `pmset -g assertions` (idle-sleep assertion only; the display still sleeps). The assertion is gone after 2 minutes.
- `status --watch` shows the countdown, computed locally from `ends_at`, with no per-second messages from the service.
- `kill -9` of the service releases the assertion (the OS does this automatically). The next start replays the journal cleanly.
- A dispatch test with a remote context and limited scopes gets `forbidden`.
- Energy:
  - No `vigilia` process exists 5 minutes after the session ends.
  - Over a 30-minute session, `powermetrics --samplers tasks` shows ≤ 1 Vigilia wake-up per minute on average.
  - The service starts in under 50 ms.
  - A daemon test with a mocked clock counts ≤ 60 timer wake-ups per simulated hour.
  - Memory under 10 MB.
  - `docs/energy.md` exists with the first numbers.

**Not in this step:** lid-closed sessions, any UI, Linux/Windows backends.

---

## Step 2: Stay awake with the lid closed, safely (v0.2)

**What the app can do:** `vigilia start 4h --lid` keeps the Mac running with the lid shut, so agents keep working. If anything goes wrong (crash, low battery, overheating) the Mac goes back to normal sleep behaviour.

**What to build:**
- macOS backend:
  - `keep_awake(lid_closed_too = true)` using `pmset -a disablesleep 1/0` through the sudoers rule.
  - Lid events from IOKit `AppleClamshellState`.
  - Battery and heat readings from OS notifications (`IOPSNotificationCreateRunLoopSource`, `ProcessInfo.thermalStateDidChangeNotification`), not polling.
- `core` rules: end the session below the battery floor (default 10%) and at critical heat. Rules are evaluated only when a power or thermal event arrives.
- `vigilia recover`: the backstop entry point. It takes the lock, replays unfinished journal entries and restores sleep.
- The backstop only exists during a session:
  - At session start the service loads a backstop LaunchAgent (`StartInterval` 300) that runs `vigilia recover`.
  - At session end the service removes it. Both steps are recorded in the journal.
  - `recover` also runs once at login. When it finds nothing left to do, it removes the backstop job.
- `vigilia install` / `uninstall`:
  - Shows the sudoers rule and asks for confirmation before installing it.
  - Installs the socket-activated LaunchAgent for the service and the run-once-at-login `recover` job.
- `vigilia debug simulate-lid close|open`.
- `NeedsAttention` state: if undo fails, every client shows it and `status` exits with a non-zero code.

**Built for later changes:**
- Rules are a list of `(condition → action)` entries in `core/rules.rs`. Step 6's "power saver below 40%" is one more entry.
- Lid events are published on the bus. Step 6's lid actions subscribe to them; nothing else changes.

**Done when:**
- Lid closed for 10 minutes during a session: the machine stays awake, and a test job keeps writing to a file.
- `kill -9` the service mid-session and wait past the end time: the backstop restores `disablesleep 0` and logs it.
- With the battery floor raised in config above the current level, the session ends straight away and sleep is restored.
- `launchctl list` shows no backstop job outside a session.
- Energy budget met during a 30-minute lid-closed session; numbers recorded in `docs/energy.md`.

**Not in this step:** freezing apps, mute, screen off, Low Power Mode.

---

## Step 3: macOS menu bar app (v0.3)

**What the app can do:**
- A menu bar icon with the countdown next to it.
- Start a session with days/hours/minutes, click to extend, hold to end.
- A settings window (battery floor, heat rule, lid-closed default).
- Recovery warnings shown in the menu.

The CLI keeps working alongside it.

**What to build:**
- `apps/macos`: a SwiftPM app (SwiftUI + `NSStatusItem`).
- A small Swift protocol client: socket connection, newline-delimited JSON, `Codable` types generated or checked against `protocol.schema.json`.
- `hello` → capabilities decide which controls are shown. `subscribe` delivers session changes and warnings.
- Energy:
  - The countdown is drawn from `ends_at`.
  - It redraws once a minute (showing `1h 23m`), and once a second only while the menu is open.
  - No animation in the menu bar item. App Nap is allowed.
- Launch at login with `SMAppService`. The app starts the service if it isn't running.
- Packaging: a signed `.app` that bundles the `vigilia` binary, and an installer that runs `vigilia install`. A Homebrew cask is optional.

**Built for later changes:**
- The app contains no session logic: it is drawing plus protocol calls. New features (agents in step 4, lid actions in step 6) are new menu sections bound to new protocol methods.
- The Swift protocol client is its own module, so it can be reused by a future iOS app.

**Done when:**
- The app and the CLI show the same session at the same time.
- Quitting the app doesn't end the session.
- Killing the service shows a warning in the app, and the app reconnects when the service is back.
- Memory of the app plus the service is measured and noted in the release notes.
- With the menu closed, the app's Energy Impact in Activity Monitor is ≈ 0 and it redraws at most once a minute. Numbers recorded in `docs/energy.md`.

---

## Step 4: Agents, on the local machine (v0.4)

**What the app can do:**
- `vigilia agent list` shows your running agents (tmux panes you marked).
- `agent tail` shows an agent's recent output; `agent send` types into its pane.
- When an agent needs input or finishes, you get a macOS notification and the menu bar shows it.
- Agent panes are never frozen.

**What to build:**
- New crate `agents`:
  - `AgentAdapter` trait.
  - tmux adapter: discover panes marked with `@vigilia-agent on`, capture the pane's output, `send-keys`.
  - Hook ingest: `vigilia agent notify --id … --kind … --summary …`.
- Protocol additions: `agent.list`, `agent.tail`, `agent.send`, and the `agent_event` event. All are additive.
- Docs and examples:
  - Claude Code `Notification` and `Stop` hooks that call `vigilia agent notify`.
  - A generic wrapper for other agents.
- macOS app:
  - An "Agents" section in the menu.
  - Notifications through `UserNotifications`.
  - A quick reply field that sends `agent.send`.
- Config: an `[agents]` section. Terminals, tmux and agent CLIs are added to the never-freeze list automatically.
- No polling:
  - Agent state comes only from hook events.
  - `tmux capture-pane` runs only for an explicit `agent.tail`.
  - Panes are rescanned only while a client is subscribed to agent events.
- **`auto_end` rule**, the biggest battery saving in this step, opt-in, configured in `[agents.auto_end]` with `enabled`, `idle_minutes` and `grace_minutes`:
  - It triggers when every tracked agent has reported `finished`, or no agent event has arrived for `idle_minutes`.
  - Vigilia then sends a notification and waits a grace period (default 2 min) that any client can cancel.
  - After that it ends the session so the Mac can sleep.
  - The macOS app shows the grace countdown with a "Keep awake" button.

**Built for later changes:**
- Agent events go on the event bus, so step 5's remote clients get them with no change to the agents crate.
- Other agent integrations become new adapters (for example a native Claude Code integration, or a non-tmux terminal).

**Done when:**
- Claude Code in a marked pane, with the hook installed, asks for permission → a notification appears within 2 seconds.
- Replying from the menu reaches the pane.
- Unmarked panes are never listed or touched; CI checks this with a real tmux server.
- With `auto_end` on: the agent's `Stop` hook fires, the grace period passes, the session ends and the Mac is allowed to sleep. Cancelling during the grace period keeps the session running.
- No `tmux` processes are spawned while nobody is subscribed to agent events (checked over 30 minutes).
- Energy budget still met with agents tracked.

---

## Step 5: Talk to your agents from your phone, lid closed (v0.5)

**What the app can do:**
- Pair your phone once with a QR code.
- With the laptop closed in your bag on a desk, the phone gets a push notification when an agent needs input.
- You open a small web page over your Tailscale network, see the agent's latest output and reply.
- You can check or extend the session, and revoke the phone at any time.

**What to build:**
- New crate `remote`, behind `--features remote` (included in the macOS release build, but off in config by default):
  - `transports/remote.rs`: WebSocket listener bound **only** to the Tailscale interface address, carrying the same protocol messages.
  - Pairing: `vigilia remote pair` shows a QR code with a one-time token. The device registers its own key, and the user picks its scopes.
  - Device store, `remote devices`, `remote revoke`.
  - Per-device rate limits.
  - Append-only `remote-audit.jsonl` audit log.
  - Embedded phone web client: one static HTML/JS file compiled into the binary and served over the tailnet. It shows status, agents, tail and reply. No iOS app is needed yet.
  - Push notifications through ntfy (outbound HTTPS POST to a self-chosen topic or self-hosted server). An option sends only "agent X needs input" with no output text.
- Energy:
  - Remote off: no listener and no network traffic at all.
  - Remote on: the listener sits idle until a device connects.
  - Keepalive pings only while a device is connected, at least 60 s apart.
  - The phone web client receives pushed updates over the WebSocket, never polls, and draws the countdown from `ends_at`.
  - Push notifications are sent only for real events.
- `network_events` in the macOS backend. Losing the network shows a warning on all clients.
- macOS app: a "Remote" settings pane (enable, pair with QR, device list, revoke, audit log viewer).

**Built for later changes:**
- The remote transport only converts WebSocket frames to and from protocol messages. Scopes and auditing live in `dispatch`, so step 10's chat bridge and relay reuse them.
- The phone web client uses the same protocol as the Swift app. A native iOS app later can reuse the Swift protocol module from step 3.

**Done when:**
- Lid closed: a paired phone on the tailnet gets a push, opens the page, reads the agent's output, replies, and the agent continues.
- A phone that isn't paired, a revoked phone, and a phone missing the `agent.send` scope are each refused, and each refusal appears in the audit log.
- With remote disabled, no port is open (checked with `lsof -i`).
- With remote on and no device connected, Vigilia produces no network traffic (`nettop -p <pid>` over 10 minutes). With a phone connected: at most 1 keepalive per 60 s plus real events.
- Energy budget met during a 30-minute lid-closed session with a phone connected; numbers recorded in `docs/energy.md`.

---

## Step 6: Lid actions on macOS (v0.6)

**What the app can do:** when the lid closes during a session, Vigilia freezes distracting apps (Slack, WhatsApp, Discord by default), mutes audio, turns off the screen and keyboard light, and turns on Low Power Mode. Opening the lid undoes all of it. This step brings the main battery saving for the whole machine during lid-closed sessions: background apps stop using CPU and network, and the CPU runs in its low-power mode.

**What to build:**
- macOS backend:
  - `freeze`/`thaw` with SIGSTOP/SIGCONT. Each frozen process is recorded with its pid and start time.
  - `mute` through CoreAudio, restoring the original output device even if you switched.
  - `backlight_off` through DisplayServices/IOKit, behind a capability flag.
  - `power_saver` with `pmset -b lowpowermode`.
- `core`:
  - A lid-actions module that subscribes to lid events, applies the configured actions through the journal and undoes them on lid open.
  - The power-saver-below-40% and power-saver-at-serious-heat rules.
- The never-freeze list is enforced: meeting apps, terminals, tmux, agents and the VPN client.
- macOS app: a lid-actions section in settings.

**Built for later changes:**
- Each lid action is a separate `Action` with its own capability flag. Adding the Docker idle rule later is one more `Action`.

**Done when:**
- `simulate-lid close` → Slack shows state `T`, audio is muted, Low Power Mode is on, and agents and tmux keep running. `simulate-lid open` restores all of it.
- Crash tests at every step of the journal leave nothing frozen or muted after `recover`.
- A pid-reuse test: a new process that gets a recorded pid is never sent SIGCONT.
- Battery drain per hour during a lid-closed session with a typical agent workload is measured with lid actions on and off. Both numbers go into `docs/energy.md`.

---

## Step 7: Linux (v0.7)

**What the app can do:** the CLI, the service, lid-closed sessions, rules, lid actions, agents and remote access all work on Linux. Linux users have no GUI yet, but the phone web client and the CLI cover daily use.

**What to build:**
- Linux backend:
  - logind inhibitors (`sleep:idle:handle-lid-switch`) and the `LidClosed` property.
  - Battery and heat: UPower and `/sys/class/power_supply`.
  - Freeze: SIGSTOP/SIGCONT.
  - Mute: PipeWire/Pulse.
  - Backlight: sysfs or logind `SetBrightness`.
  - Power saver: power-profiles-daemon.
  - Network: NetworkManager.
  - Events: UPower and NetworkManager `PropertiesChanged` signals. Thermal zones have no signal, so they're polled at most every 5 minutes.
- Packaging:
  - A systemd `.socket` unit starts the service on demand.
  - The backstop is a transient `systemd-run --user` timer, created at session start and removed at session end.
  - `vigilia install` for Linux, `.deb`/`.rpm` or a tarball.
- The manual hardware checklist (`docs/testing.md`) is run on one GNOME laptop and one KDE laptop.

**Done when:** the step 2, step 5 and step 6 checks pass on a real Linux laptop, and the energy budgets are met as measured with `powertop`. Numbers recorded in `docs/energy.md`.

---

## Step 8: Windows (v0.8)

**What the app can do:** same as step 7, on Windows.

**What to build:**
- Windows backend:
  - `SetThreadExecutionState` for staying awake.
  - `powercfg` lid action, journaled and restored.
  - Lid notifications.
  - Battery changes through `RegisterPowerSettingNotification(GUID_BATTERY_PERCENTAGE_REMAINING)`, with `GetSystemPowerStatus` to read the current state.
  - `NtSuspendProcess` for freezing, behind a capability flag.
  - `IAudioEndpointVolume` for mute.
  - `INetworkListManager` for network state.
- Named-pipe transport with an owner-only security descriptor.
- Windows has no socket activation, so the service stays running. While idle it must truly block, with no timers or polling.
- The Task Scheduler backstop task is created at session start and deleted at session end. `install.ps1`, MSI or zip.
- First thing in this step: prototype the `powercfg` admin requirement and the Modern Standby network behaviour, then adjust the scope based on the results.

**Done when:**
- The step 2, step 5 and step 6 checks pass on a real Windows laptop, including a Modern Standby machine. Anything that can't be supported is reported in `Capabilities`.
- The energy budgets are met (`powercfg /energy`, `powercfg /srumutil`). The idle service shows no CPU wake-ups. Numbers recorded in `docs/energy.md`.

---

## Step 9: Linux and Windows UIs (v0.9)

**What the app can do:** a tray icon with the countdown, start/extend/end, agents and settings on Linux and Windows. Feature parity with the macOS app is the goal.

**What to build:**
- `apps/linux`: Rust `ksni` tray (StatusNotifierItem), plus a small settings window. Document the AppIndicator extension requirement on GNOME.
- `apps/windows`: WinUI/C# tray app, or Rust. Decide at the start of the step.
- Both are thin protocol clients, like the Swift app.
- The same energy rules as step 3: draw the countdown from `ends_at`, redraw once a minute while closed, no tray animation.

**Done when:** the step 3 checks pass on each OS, including the energy check.

---

## Step 10: Extras (v1.0)

Each item is independent, behind a config flag, and can ship in any order:
- **Two-way chat bridge (Telegram or Slack):** agent events become messages and replies become `agent.send`. Uses the same scopes and audit log as the tailnet transport.
- **Wi-Fi/hotspot failover:** join a configured iPhone hotspot when Wi-Fi drops. The password is stored with `keyring` (Keychain / Secret Service / Credential Manager).
- **Relay server with Noise end-to-end encryption**, for people who don't want a VPN.
- **Native iOS app** reusing the Swift protocol module, if the web client isn't enough.
- **Linux cgroup freezer** instead of SIGSTOP.
- **Docker idle rule.**

## How steps map to `plan.md` milestones

| Step | `plan.md` milestone |
|---|---|
| 1 | M0, M1, M2 (macOS only) |
| 2 | M3 (macOS) + the battery and heat parts of M4 |
| 3 | M6 |
| 4 | M8 |
| 5 | M9 (without failover) |
| 6 | M4 (macOS) |
| 7 | M3, M4 and M9 on Linux |
| 8 | M5 |
| 9 | M7 |
| 10 | M10 + the remaining extras |

Order changes compared with `plan.md`, made so the main goal (talk to agents while the lid is closed) arrives sooner:
- The macOS UI, agents and remote access come before lid actions and before the other OSes.
- Step 5 adds an embedded phone web client and ntfy push notifications. They are the smallest way to make remote access usable from a phone without building an iOS app.
