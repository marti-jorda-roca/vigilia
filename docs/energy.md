# Energy measurements

Budgets (from `docs/plans/architecture-plan.md`): no Vigilia process with no session; at most
1 wake-up per minute during a session; RSS under 10 MB; no network traffic with remote off.

## Comparing with other keep-awake tools

`scripts/bench-energy.sh` measures what each tool itself costs while it keeps the Mac awake,
against caffeinate, KeepingYouAwake, Caffeine and Amphetamine, whichever are installed:

```sh
scripts/bench-energy.sh            # every installed tool, about 1 hour
scripts/bench-energy.sh --quick    # shorter windows, about 15 minutes
scripts/bench-energy.sh vigilia amphetamine
```

For each tool it measures the CPU time, idle wake-ups, energy impact and memory of the tool's
own processes, first during a session and then with none. It uses `top`, so it needs no sudo.
Staying awake itself costs the same with every tool, so the numbers show the overhead each
tool adds on top of that. It refuses to start while a Vigilia session is running, and marks a
window invalid when the tool holds no sleep assertion during its session or still holds one
afterwards. Results go to `target/bench-energy/<date>-<time>/summary.md`. `--help` lists the
options.

## v0.2 (step 2: lid-closed sessions)

Measured on the development Mac (Apple silicon, macOS) with the release binary
(`cargo build --release`), `VIGILIA_HOME` pointed at a scratch directory:

| Check | Result |
|---|---|
| Timer wake-ups over a simulated 1-hour session (daemon test, mocked clock) | 1 (the session end); budget ≤ 60 |
| Timer wake-ups, session without an end, 31 simulated days | 0 |
| Service start to session applied (log timestamps, first `start` on a cold service) | ~45 ms; budget < 50 ms |
| Daemon RSS (`ps -o rss`), idle / during a session | 7,824 KB / 8,096 KB — under the 10 MB budget (physical footprint 2.1–2.4 MB by `footprint`) |
| Process 5 min after the session ends | none (idle exit logged, `exit=Idle`) |
| `pmset -g assertions` during a session | `PreventUserIdleSystemSleep` "Vigilia session" held; gone after `stop` |
| Backstop job outside a session | `com.vigilia.backstop.plist` present in `~/Library/LaunchAgents` during the session, removed at session end; `launchctl list` shows no backstop afterwards |
| Lid/power events | real `lid changed state=Open` from the IOKit watcher at service start; no polling timers are armed for rules |
| Lid-closed 30-minute hardware session, battery drain | **not measured** — needs `vigilia install` (an interactive sudo prompt) and physically closing the lid; the checklist is in `docs/testing.md` and remains an open acceptance item for v0.2 |

Note on RSS: the first v0.2 measurement showed ~12,000 KB. `heap` traced a single ~4 MB
allocation to `tracing-appender`'s default non-blocking log channel, which pre-allocates a
128,000-line buffer. The daemon now caps it at 512 lines (`NonBlockingBuilder`), which
brought RSS to the 7,824–8,096 KB recorded above; logging was already lossy past the limit,
and the journal remains the record that matters.

## v0.1 (step 1)

First numbers, from the daemon integration tests (mocked clock, fake platform):

- Timer wake-ups over a simulated 1-hour session: 1.
- No process 5 minutes after the session ends (idle-exit test).
- `status --watch` receives no per-second messages: the socket test asserts silence while a
  session runs.
