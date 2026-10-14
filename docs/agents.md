# Using Vigilia with coding agents

Coding agents can keep your Mac awake on their own. Hook Vigilia into the agent once, and every prompt starts or extends a session. The Mac stays awake while the agent works, including with the lid closed, and goes back to sleeping normally after the agent goes quiet.

This page covers [Claude Code](#claude-code) and [Codex](#codex). Any agent with hooks works the same way (see [Other agents](#other-agents)).

## How it works

The hooks run one command:

```sh
vigilia start 30m --lid --extend
```

This sets a rolling 30-minute window. If no session is running, the command starts one. If a session is running but ends in less than 30 minutes, it moves the end to 30 minutes from now. A session that already lasts longer is left alone, so the agent never shortens a session you started by hand.

The command runs when you send a prompt and before every tool call. As long as the agent keeps working, the session keeps moving forward. When the agent stops, the session runs out 30 minutes after its last tool call and the Mac can sleep again. Nothing needs to call `vigilia stop`, so an agent that crashes or gets killed can't leave the Mac awake.

Each call takes a few milliseconds. When no session is running, no Vigilia process runs either.

## Before you start

**Lid-closed sessions need a one-time setup.** Run `vigilia install` (or **Lid closed → Set up…** in the Mac app's Settings). Without it, `--lid` fails and the hook starts no session at all. If you only want the Mac awake with the lid open, remove `--lid` from the commands below.

**Use a path the agent can find.** Agents don't always load your shell profile, so `~/.cargo/bin` may not be on their `PATH`. If `vigilia` isn't found, use the full path:

| How you installed it | Path |
|---|---|
| `cargo install` | `~/.cargo/bin/vigilia` |
| Mac app, after lid-closed setup | `/usr/local/bin/vigilia` |
| Mac app, without lid-closed setup | `/Applications/Vigilia.app/Contents/Helpers/vigilia` |

**The hooks throw away Vigilia's output.** Both agents add a `UserPromptSubmit` hook's printed output to the model's context. The commands end in `>/dev/null 2>&1 || true` so the model never sees it and a failed call never interrupts the agent.

## Claude Code

Add this to `~/.claude/settings.json` to cover every project, or to `.claude/settings.json` in a repo to share it with your team. If the file already has a `hooks` key, merge these two events into it.

```json
{
  "hooks": {
    "UserPromptSubmit": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "vigilia start 30m --lid --extend >/dev/null 2>&1 || true",
            "async": true
          }
        ]
      }
    ],
    "PreToolUse": [
      {
        "matcher": "*",
        "hooks": [
          {
            "type": "command",
            "command": "vigilia start 30m --lid --extend >/dev/null 2>&1 || true",
            "async": true
          }
        ]
      }
    ]
  }
}
```

`async: true` runs the command in the background, so Claude never waits on it. `"matcher": "*"` matches every tool, including MCP tools and subagents.

To check it works, send Claude a prompt and run `vigilia status` in another terminal. You should see a session ending about 30 minutes from now.

## Codex

Add this to `~/.codex/hooks.json` to cover every project, or to `.codex/hooks.json` in a repo. Hooks are on by default in current Codex releases.

```json
{
  "hooks": {
    "UserPromptSubmit": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "vigilia start 30m --lid --extend >/dev/null 2>&1 || true",
            "async": true
          }
        ]
      }
    ],
    "PreToolUse": [
      {
        "matcher": "*",
        "hooks": [
          {
            "type": "command",
            "command": "vigilia start 30m --lid --extend >/dev/null 2>&1 || true",
            "async": true
          }
        ]
      }
    ]
  }
}
```

If you prefer `~/.codex/config.toml`, the same hooks look like this:

```toml
[[hooks.UserPromptSubmit]]
[[hooks.UserPromptSubmit.hooks]]
type = "command"
command = "vigilia start 30m --lid --extend >/dev/null 2>&1 || true"
async = true

[[hooks.PreToolUse]]
matcher = "*"
[[hooks.PreToolUse.hooks]]
type = "command"
command = "vigilia start 30m --lid --extend >/dev/null 2>&1 || true"
async = true
```

Use one format per directory. Codex merges the two and warns at startup if both exist.

Codex runs a hook only after you trust it. Open Codex, run `/hooks`, and approve both. Codex ties trust to the hook's content, so if you change the command later, approve it again.

## Choosing the window

30 minutes covers most tool calls, including long builds and test runs. The window counts from when a tool call starts, so a single command that runs longer than the window can outlast the session. Raise it if your agent runs long jobs:

```sh
vigilia start 2h --lid --extend
```

A shorter window lets the Mac sleep sooner after the agent finishes. A longer one is safer for long jobs. Either way, the agent never cuts short a longer session you started yourself.

## Ending the session when the agent exits

The rolling window ends the session on its own, so you don't need this. If you want the Mac to sleep as soon as you quit the agent, add a `SessionEnd` hook that runs `vigilia stop`:

```json
"SessionEnd": [
  {
    "hooks": [
      {
        "type": "command",
        "command": "vigilia stop >/dev/null 2>&1 || true",
        "timeout": 3
      }
    ]
  }
]
```

Only do this if one agent at a time uses Vigilia. Every tool shares one session, so `vigilia stop` from one agent also ends the session for another agent that is still working, and ends a session you started by hand.

## Other agents

Any agent that can run a shell command at the start of a prompt or before a tool call can use the same setup. Run `vigilia start <window> --extend` at those points, discard the output, and don't fail the agent if the command fails.

If the agent can't run hooks, tell it to call Vigilia itself, in `AGENTS.md`, `CLAUDE.md` or its system prompt:

```md
Before a task that may take more than a few minutes, run `vigilia start 30m --extend`.
```

Hooks are more reliable. A model can skip an instruction, but a hook always runs.

## Troubleshooting

**`vigilia status` shows no session after a prompt.** Run the hook's command in a terminal without the redirect to see the error:

```sh
vigilia start 30m --lid --extend
```

- `command not found`: use the full path from [Before you start](#before-you-start).
- `sudo refused (run vigilia install first)`: run `vigilia install`, or remove `--lid`.
- In Codex, the hook may not be trusted yet. Run `/hooks`.

**The Mac stays awake but the lid-closed mode is off.** A running session keeps the lid mode it started with. If you started a session without `--lid` and the agent then calls `start --lid --extend`, the session gets longer but still sleeps when you close the lid. Run `vigilia stop` and let the next prompt start a new session.

**Claude Code already keeps the Mac awake.** On macOS, Claude Code runs `caffeinate -i` while it works. That stops idle sleep but not sleep with the lid closed. The Vigilia hooks add lid-closed sessions, the battery and heat cutoffs, and one session that the menu bar app, your terminal and every agent share.
