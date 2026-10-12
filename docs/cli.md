# Vigilia CLI

How to use the `vigilia` command. To install it, see [Install](../README.md#install).

> The CLI crate (`crates/cli`) is being written as part of v0.1. This page documents the v0.1 commands from the [execution plan](execution-plan.md) and the behaviour the service already has. Commands planned for later releases are listed at the end.

## Quick start

```sh
vigilia start 2h        # keep the Mac awake for 2 hours
vigilia status --watch  # live countdown; Ctrl-C to quit
vigilia extend 30m      # add 30 minutes
vigilia stop            # end the session now
```

While a session runs, the Mac doesn't idle-sleep. The display can still turn off, and closing the lid still puts the Mac to sleep (lid-closed sessions come in v0.2). When the session ends, sleep goes back to normal.

## Durations

`start` and `extend` take a duration written as one or more `<number><unit>` groups:

| Unit | Meaning |
|---|---|
| `d` | days |
| `h` | hours |
| `m` | minutes |
| `s` | seconds |

Examples: `90s`, `45m`, `2h`, `1h30m`, `1d2h`. Units are case-insensitive. A bare number such as `30` is rejected, because the unit would be a guess.

A duration can't be zero, and a session can't run longer than 30 days.

## Commands

### `vigilia start <duration>`

Starts a session that ends after `<duration>`. Only one session can run at a time, so this fails if one is already running. Use `extend` to add time, or `stop` and then `start` to begin a new one.

### `vigilia extend <duration>`

Adds `<duration>` to the running session's remaining time. `vigilia extend 30m` on a session with 10 minutes left leaves 40 minutes. This fails if no session is running, or if the total remaining time would be over 30 days.

### `vigilia stop`

Ends the session now and lets the Mac sleep normally again.

If undoing a session's changes ever fails, the session stays in the `NeedsAttention` state and the Mac may not sleep normally. Running `vigilia stop` again retries the undo.

### `vigilia status [--json] [--watch]`

Shows the current session, or that there is none.

- `--watch` keeps running and redraws the countdown until you press Ctrl-C. The countdown is computed locally from the session's end time. The service only sends a message when something changes (start, extend, stop, end), so watching costs almost nothing.
- `--json` prints the session as JSON, in the same shape the [protocol](../crates/protocol/schema/protocol.schema.json) uses. Scripts and agents should use this rather than parse the human output.

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

`state` is one of `Active`, `Ending`, `Ended` or `NeedsAttention`. Times are UTC, in RFC 3339 format.

### `vigilia daemon`

Runs the service in the foreground and logs to the terminal. You don't normally run this yourself (see below). It is useful when debugging.

## Using it from a coding agent

An agent can call `vigilia start` before a long task, with no setup step, and `vigilia stop` when it's done:

```sh
vigilia start 3h
# … long-running work …
vigilia stop
```

If the agent might find a session already running, it can check first with `vigilia status --json`, then call `extend` instead of `start`.

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

When the service refuses a command, it returns an error code, which the CLI shows along with the message.

| Code | When |
|---|---|
| `session_active` | `start` while a session is already running (or ending, or needs attention) |
| `no_session` | `extend` or `stop` with no session running |
| `bad_request` | Invalid input, such as a zero duration or one over 30 days |
| `unsupported` | The feature isn't available on this OS or machine |
| `forbidden` | The client lacks permission. Local commands have every permission, so you only see this from remote clients (later) |
| `unsupported_version` | The CLI and the service speak different protocol versions; reinstall so both match |
| `internal` | A bug in the service; check `logs/vigilia.log` |

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
