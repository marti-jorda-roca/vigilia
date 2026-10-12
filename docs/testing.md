# Manual hardware checks

Checks that need real hardware (a lid, a battery, root-owned installers) and so can't run in
CI. Record the outcome and the numbers in `docs/energy.md` when you run them.

## Step 2 (v0.2): lid-closed sessions on macOS

One-time setup:

```sh
vigilia install        # shows the sudoers rule and asks before installing it
```

Then:

1. **Lid closed for 10 minutes.** Start a job that keeps writing, close the lid, wait:
   ```sh
   vigilia start 15m --lid
   while true; do date >> /tmp/vigilia-lid-test.log; sleep 30; done   # in another terminal first
   ```
   After 10 minutes the log has ~20 new lines and `vigilia status` still shows the session.
2. **Crash recovery.** During a lid session, `kill -9` the daemon (its pid is in
   `~/Library/Application Support/vigilia/vigilia.lock`), then wait for the backstop
   (5 minutes) past the point where it fires. `pmset -g | grep SleepDisabled` must show
   `0` (or no line), and the daemon log must show the recovery. Close and open the lid:
   the Mac must sleep normally again.
3. **Battery floor.** Edit `config.toml` (`~/Library/Application Support/vigilia/config.toml`):
   ```toml
   [rules]
   battery_floor_percent = 99
   ```
   Start a session on battery: it must end straight away and sleep must be restored.
   Put the floor back to `10` afterwards.
4. **No backstop outside a session.** `launchctl list | grep vigilia` shows no
   `com.vigilia.backstop` job before a session, during step 1 above it does, and after
   `vigilia stop` it is gone again.
5. **Energy during a lid-closed session.** Over 30 minutes with the lid closed:
   `sudo powermetrics --samplers tasks --show-process-energy` — at most ~1 Vigilia wake-up
   per minute on average, Energy Impact ≈ 0, RSS under 10 MB (`ps -o rss -p <pid>`).

Status: steps 1–5 are **not yet run** — they need `vigilia install` (an interactive sudo
prompt) and physically closing the lid. Everything that does not need those two things is
covered by automated tests and the measurements in `docs/energy.md`.
