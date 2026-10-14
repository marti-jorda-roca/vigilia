<p align="center"><img src="docs/assets/vigilia-banner.svg" alt="Vigilia: a pixel-art guard holding a lantern" width="640"></p>

Keep your laptop awake for a set time, from the terminal or from a coding agent, and pay almost nothing in battery for it.

<p align="center"><img src="docs/assets/demo.gif" alt="Terminal demo: vigilia start 2h, extend 30m, start 3h --extend, then stop" width="720"></p>

For more details, see the [CLI documentation](docs/cli.md).

## Install

Vigilia comes as a command-line tool and as a Mac menu bar app. Install either one, or both: they talk to the same background service, so a session started in one shows up in the other. The Mac app includes its own copy of the CLI.

### Mac app

The menu bar app shows the time left next to the clock and lets you start, extend and end sessions with the mouse. It needs macOS 13 or later.

1. Download `Vigilia-<version>.dmg` from the [latest release](https://github.com/marti-jorda-roca/vigilia/releases/latest). Open it and drag Vigilia to Applications.
2. Open Vigilia from Applications. If the release notes say the app isn't notarized, macOS blocks it the first time. Go to System Settings → Privacy & Security and click **Open Anyway** next to the message about Vigilia.
3. On first launch, Settings opens on **Setup**:
   - **Service → Install** sets up the background service. No password needed.
   - **Lid closed → Set up…** is optional, for lid-closed sessions. It shows the sudoers rule and asks for your password once. It also puts `vigilia` in `/usr/local/bin`, so the CLI works in Terminal without a separate install.
   - **Launch at login**, under General, starts the app when you log in.

The app has no Dock icon. Click the lantern in the menu bar to open it. The lantern is lit while a session keeps the Mac awake. Leave the timeout empty to stay awake until you hold the end button, or set one (`2h`, `45m`) to see the time left next to the lantern. Quitting the app leaves a running session alone.

Keep the app in Applications: the service remembers where its program is. If you move the app, Settings offers **Repair**.

To build the app from source instead, you need rustup and the Swift toolchain, from Xcode or the Command Line Tools (`xcode-select --install`). Run:

```sh
apps/macos/scripts/build-app.sh      # ARCHS=arm64 for a quicker, Apple silicon-only build
```

It writes `apps/macos/dist/Vigilia.app` and the dmg, signed ad-hoc. Then continue from step 1 with that dmg. While working on the app, `apps/macos/scripts/reinstall.sh` does it all in one go: it removes the installed app, its service and the sudoers rule, rebuilds, and opens the new build from Applications.

### Command line

You need macOS and [rustup](https://rustup.rs).

1. Install the `vigilia` binary from [crates.io](https://crates.io/crates/vigilia):

   ```sh
   cargo install vigilia --locked
   ```

   Or build it from source:

   ```sh
   git clone https://github.com/marti-jorda-roca/vigilia.git
   cd vigilia
   cargo install --path crates/cli --locked
   ```

   Either way, the binary goes in `~/.cargo/bin`. Make sure that directory is on your `PATH`.

2. Optionally, for lid-closed sessions, run this once:

   ```sh
   vigilia install
   ```

   It shows a narrow sudoers rule (only `pmset disablesleep 1/0`) and asks before installing it. It also installs the socket-activated service LaunchAgent and a run-once-at-login `vigilia recover` job. Add `--skip-sudoers` to install only the LaunchAgents.

You can skip step 2: `vigilia start` works without any setup and starts the service when it needs it.

## Uninstall

### Mac app

1. End any session. Then open Settings and click **Uninstall** under Setup. It removes the service. If lid-closed sessions are set up, it asks for your password and removes the sudoers rule and the `/usr/local/bin/vigilia` link.
2. If you turned on launch at login, turn it off in Settings.
3. Quit the app and drag it from Applications to the Trash.
4. Delete its preferences and the service's files:

   ```sh
   defaults delete com.vigilia.app
   rm -rf ~/Library/Application\ Support/vigilia    # config, journal, logs, socket
   ```

   If you keep using the command line, skip the `rm -rf` line: it uses those files too.

### Command line

```sh
vigilia stop                                     # end any session; restores normal sleep
vigilia uninstall                                # LaunchAgents and the sudoers rule
rm -rf ~/Library/Application\ Support/vigilia    # config, journal, logs, socket
cargo uninstall vigilia                          # the binary
```

`vigilia uninstall` shows what it removes and asks first. Add `--skip-sudoers` to keep the sudoers rule. If you also use the Mac app, skip the `rm -rf` line: the app uses those files too.

If you ever ran `packaging/macos/install-dev.sh` from an early development checkout, you may also have an older LaunchAgent named `vigilia.daemon`. It listens on the same socket as the current service, and the two can block each other so commands hang. Remove it with:

```sh
launchctl unload ~/Library/LaunchAgents/vigilia.daemon.plist
rm ~/Library/LaunchAgents/vigilia.daemon.plist
```

## Layout

| Crate | What it holds |
|---|---|
| `crates/protocol` | Wire types (requests, responses, events), `ClientContext` and scopes, the generated JSON Schema |
| `crates/core` | Session state machine, recovery journal, battery/heat rules, config, paths. No OS code |
| `crates/platform` | `Platform` trait and `Capabilities`; macOS backend (idle-sleep assertion, `pmset disablesleep`, IOKit lid/power events, launchd backstop, GCD deadline timer), stubs for Linux and Windows, `FakePlatform` for tests |
| `crates/daemon` | The service: dispatch with scope checks, event bus, owner-only Unix socket, instance lock, launchd socket activation, idle exit |
| `crates/cli` | The `vigilia` binary: client commands that start the service when it isn't running, and `vigilia daemon`, the service itself |

## Development

```sh
cargo test --workspace
```

### Lint and format

CI runs these two checks and fails on any warning, so run them before you push:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
```

To run both on every commit, install [pre-commit](https://pre-commit.com) (`brew install pre-commit`, or [prek](https://github.com/j178/prek)) and enable the hooks once per clone:

```sh
pre-commit install
pre-commit run --all-files   # optional: check the whole tree now
```

`cargo fmt --all` rewrites the files in place. Clippy can fix some warnings for you with `cargo clippy --fix --workspace --all-targets --allow-dirty`.

The macOS code is only compiled on macOS. To check the other platforms from a Mac, add their targets once with `rustup target add x86_64-unknown-linux-gnu x86_64-pc-windows-msvc`, then pass `--target` to clippy:

```sh
cargo clippy --workspace --all-targets --target x86_64-unknown-linux-gnu -- -D warnings
```

The protocol's JSON Schema in `crates/protocol/schema/protocol.schema.json` is generated from the Rust types, and a test fails when it is stale. After changing protocol types:

```sh
UPDATE_SCHEMA=1 cargo test -p vigilia-protocol --test schema
```

Protocol changes may only add things (new methods, events, optional fields). From v0.1 on, CI will also compare the schema against the previous release's.

## License

Apache-2.0. See [`LICENSE`](LICENSE).
