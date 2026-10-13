# Vigilia documentation

Docs for using Vigilia, checking that it meets its battery budget, and following how it's planned. For installing it, see the [project README](../README.md#install).

## Using Vigilia

| Doc | What it covers |
|---|---|
| [CLI](cli.md) | Every `vigilia` command and flag, duration syntax, lid-closed sessions, JSON output, exit codes, use from coding agents, the background service, its files and configuration |

## Testing and measurements

| Doc | What it covers |
|---|---|
| [Manual hardware checks](testing.md) | Checks that can't run in CI because they need a real lid, battery or sudo prompt: lid-closed sessions, crash recovery, battery floor, the backstop job |
| [Energy measurements](energy.md) | The energy budgets and the numbers measured for each release (wake-ups, RSS, idle exit) |

## Plans

| Doc | What it covers |
|---|---|
| [Architecture plan](plans/architecture-plan.md) | What Vigilia is for, its goals and non-goals, the daemon/protocol/platform design, and the energy-efficiency rules |
| [Execution plan](plans/execution-plan.md) | The build order from v0.1 to v1.0, with what each release adds and the rules every step follows |

## Assets

`assets/` holds images used by the docs and the project README, such as the [banner](assets/vigilia-banner.svg).
