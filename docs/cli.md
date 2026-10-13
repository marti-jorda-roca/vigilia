# Vigilia CLI

How to use the `vigilia` command. To install it, see [Install](../README.md#install).

## Quick start

```sh
vigilia start 2h        # keep the Mac awake for 2 hours
vigilia start           # keep it awake until `vigilia stop`
vigilia status --watch  # live countdown; Ctrl-C to quit
vigilia extend 30m      # add 30 minutes
vigilia stop            # end the session now
```

While a session runs, the Mac doesn't idle-sleep and the display can still turn off. With `--lid` (see below) it also stays awake with the lid closed. When the session ends, sleep goes back to normal.

## Durations

`extend`, and `start` when given one, take a duration written as one or more `<number><unit>` groups:

| Unit | Meaning |
|---|---|
| `d` | days |
| `h` | hours |
| `m` | minutes |
| `s` | seconds |

Examples: `90s`, `45m`, `2h`, `1h30m`, `1d2h`. Units are case-insensitive. A bare number such as `30` is rejected, because the unit would be a guess.

A duration can't be zero, and a session with a duration can't run longer than 30 days. For longer, start one without a duration (see below).

## Commands

### `vigilia start [<duration>] [--lid] [--extend]`

With `--lid`, the session also keeps the Mac awake with the lid closed (`pmset disablesleep`, through the sudoers rule installed by `vigilia install`). During a lid-closed session a backstop job runs `vigilia recover` every 5 minutes, so a crash restores normal sleep; the job is removed when the session ends. Battery and heat rules can end a lid-closed session early (see Configuration).

### `vigilia start [<duration>] [--extend]`

Starts a session that ends after `<duration>`. Only one session can run at a time, so this fails with `session_active` if one is already running.

Without a duration, the session has no end: it runs until `vigilia stop`, and the 30-day limit doesn't apply. `status` shows how long it has been running instead of a countdown.

With `--extend`, it makes sure a session runs for at least `<duration>` from now:

- If no session is running, it starts one.
- If the running session ends too soon, it extends that session so it ends `<duration>` from now (to the second).
- If the session already lasts that long, nothing changes.

`--extend` is the one call a script or an agent needs. It doesn't have to check first whether a session is running. It needs a duration; a session without an end already lasts long enough, so `--extend` leaves it as it is.

### `vigilia extend <duration>`

Adds `<duration>` to the running session's remaining time. `vigilia extend 30m` on a session with 10 minutes left leaves 40 minutes. This fails if no session is running, or if the total remaining time would be over 30 days. A session without an end stays as it is, and the command still succeeds.

### `vigilia stop`

Ends the session now and lets the Mac sleep normally again. If no session is running, it prints `No session running` and still succeeds, so a cleanup step doesn't fail because the session already ran out.

If undoing a session's changes ever fails, the session stays in the `NeedsAttention` state and the Mac may not sleep normally. Running `vigilia stop` again retries the undo.

### `vigilia status [--watch]`

Shows the current session, or that there is none. It exits with code 4 if the session needs attention.

`--watch` keeps running until you press Ctrl-C. The service only sends a message when something changes (start, extend, stop, end), so watching costs almost nothing. What it prints depends on where the output goes:

- **In a terminal**, one line with a countdown, redrawn in place. The countdown is computed locally from the session's end time. It redraws once a second in the last hour, and once a minute before that. A session without an end shows the time since it started instead, redrawn once a second in its first hour and once a minute after that.
- **In a pipe or a file**, one line each time the session changes.
- **With `--json`**, one `{"session": …}` line each time the session changes (NDJSON).

### `vigilia recover`

Undoes anything a crashed session left behind and restores normal sleep. It takes the service's lock itself, so it works exactly when the service is dead; while the service runs it reports that there is nothing to do. The backstop job and the run-once-at-login job both call this.

### `vigilia install` / `vigilia uninstall`

`install` (macOS) shows the sudoers rule below and asks for confirmation (skip with `--yes`), then installs the socket-activated service LaunchAgent, the run-once-at-login `recover` job, and the rule as `/etc/sudoers.d/vigilia`. The rule covers exactly two commands:

```
<user> ALL=(root) NOPASSWD: /usr/bin/pmset -a disablesleep 1, /usr/bin/pmset -a disablesleep 0
```

`uninstall` removes all of it, including any backstop job.

### `vigilia debug simulate-lid close|open`

Pretends the lid closed or opened: the service publishes the same `lid_changed` event a real lid movement produces. Useful for testing lid behaviour without shutting the laptop.

### `vigilia daemon`

Runs the service in the foreground and logs to the terminal. You don't normally run this yourself (see below). It is useful when debugging.

## JSON output

`--json` works with every command except `daemon`, before or after the command name (`vigilia --json stop` or `vigilia stop --json`). `start`, `extend`, `stop` and `status` all print the session in the same shape the [protocol](../crates/protocol/schema/protocol.schema.json) uses. Scripts and agents should use this rather than parse the human output.

A session in `--json` output looks like this (`session` is `null` when nothing is running):

```json
{
  "session": {
    "id": "6f0c…",
    "started_at": "2026-10-12T09:00:00Z",
    "ends_at": "2026-10-12T11:00:00Z",
    "state": "Active",
    "lid_closed_awake": false
  }
}
```

`ends_at` is `null` for a session without an end. `state` is one of `Active`, `Ending`, `Ended` or `NeedsAttention`. `stop` returns the session it ended, in state `Ended`. Times are UTC, in RFC 3339 format.

With `--json`, errors go to stderr as `{"error":{"code":"no_session","message":"no session is running"}}`. The codes are listed under [Errors](#errors).

## Exit codes

| Code | Meaning |
|---|---|
| `0` | Done |
| `1` | The service refused the command; the error code says why |
| `2` | Usage error, such as an unknown command or a bad duration |
| `3` | The service couldn't be reached or started (code `unreachable`) |
| `4` | The session needs attention: undoing its changes failed, so the Mac may not sleep normally. Run `vigilia stop` to retry |

## Using it from a coding agent

An agent can call `vigilia start` before a long task, with no setup step, and `vigilia stop` when it's done:

```sh
vigilia start 3h --extend
# … long-running work …
vigilia stop
```

`--extend` makes the call safe when a session is already running: it never shortens that session, and only lengthens it when needed. `stop` succeeds even when the session already ended.

Prefer a duration over a plain `vigilia start` here. A session without an end lasts until something runs `vigilia stop`, so an agent that crashes or forgets to stop keeps the Mac awake indefinitely, while a timed session still ends on its own.

## The service

The CLI is a client. The real work happens in the service, which is the same `vigilia` binary running as `vigilia daemon`.

- **It starts on demand.** Any command starts the service if it isn't running. In v0.2, `vigilia install` will add a LaunchAgent so launchd starts it instead (socket activation).
- **It exits when idle.** With no session and no `status --watch` running, the service exits after 5 minutes. When no session runs, no Vigilia process runs.
- **It runs as your user**, not as root, and only your user can connect to its socket.
- **A crash doesn't leave the Mac awake.** If the service is killed, macOS drops its power assertion. On the next start the service reads its journal and finishes any undo steps.

### Files

On macOS everything lives in `~/Library/Application Support/vigilia/`:

| File | What it is |
|---|---|
| `config.toml` | Settings (optional) |
| `vigilia.sock` | The socket clients connect to |
| `journal.jsonl` | Recovery journal: every change to the system and how to undo it |
| `vigilia.lock` | Makes sure only one service runs |
| `logs/vigilia.log` | Service log |

Set `VIGILIA_HOME=/some/dir` to put all of these in one directory instead. This is useful for testing a dev build next to an installed one.

### Config

`config.toml` is optional. Every setting has a default, and unknown keys are ignored.

```toml
[energy]
idle_exit_minutes = 5   # the service exits after this long with nothing to do
timer_leeway_secs = 1   # how late the session-end timer may fire, so the OS can group wake-ups

[log]
level = "info"          # a tracing filter, e.g. "debug" or "vigilia_daemon=debug"
```

## Errors

When the service refuses a command, it returns an error code. The CLI prints the message followed by the code, for example `vigilia: no session is running (no_session)`, and exits with code 1.

| Code | When |
|---|---|
| `session_active` | `start` while a session is already running (or ending, or needs attention) |
| `no_session` | `extend` with no session running |
| `bad_request` | Invalid input, such as a zero duration or one over 30 days |
| `unsupported` | The feature isn't available on this OS or machine |
| `forbidden` | The client lacks permission. Local commands have every permission, so you only see this from remote clients (later) |
| `unsupported_version` | The CLI and the service speak different protocol versions; reinstall so both match |
| `internal` | A bug in the service; check `logs/vigilia.log` |
| `unreachable` | Set by the CLI, not the service: it couldn't connect to the service or start it (exit code 3) |

## Planned commands

These are designed in the [architecture plan](architecture-plan.md#cli) but not part of v0.1:

| Command | Release |
|---|---|
| `vigilia start <duration> --lid`, `--no-lid-actions` | v0.2 (lid closed), v0.6 (lid actions) |
| `vigilia install` / `uninstall` | v0.2 |
| `vigilia recover` | v0.2 |
| `vigilia config get\|set\|edit` | Later |
| `vigilia debug simulate-lid close\|open` | v0.2 |
| `vigilia agent list\|tail\|send\|notify` | v0.4 |
| `vigilia remote enable\|disable\|pair\|devices\|revoke` | v0.5 |

## Configuration

`config.toml` accepts a `[rules]` section:

```toml
[rules]
battery_floor_percent = 10   # end the session below this charge
```

Critical heat always ends the session. Rules are checked only when the OS reports a battery or heat change (and once when a session starts), never on a timer.
