<p align="center"><img src="docs/assets/vigilia-banner.svg" alt="Vigilia: a pixel-art guard holding a lantern" width="640"></p>

Keep your laptop awake for a set time, from the terminal or from a coding agent, and pay almost nothing in battery for it.

<p align="center"><img src="docs/assets/demo.gif" alt="Terminal demo: vigilia start 2h, extend 30m, start 3h --extend, then stop" width="720"></p>

For more details, see the [CLI documentation](docs/cli.md).

## Install

There are no prebuilt binaries yet, so you install the CLI from [crates.io](https://crates.io/crates/vigilia). It needs macOS and [rustup](https://rustup.rs).

```sh
cargo install vigilia --locked
```

To build from source instead:

```sh
git clone https://github.com/marti-jorda-roca/vigilia.git
cd vigilia
cargo install --path crates/cli --locked
```

This puts a single `vigilia` binary in `~/.cargo/bin`. Make sure that directory is on your `PATH`.

For lid-closed sessions, run `vigilia install` once: it shows a narrow sudoers rule (only `pmset disablesleep 1/0`) and asks before installing it, along with the socket-activated service LaunchAgent and a run-once-at-login `vigilia recover` job. `vigilia uninstall` removes them.

### Menu bar app

The menu bar app in `apps/macos` shows the countdown next to the clock and lets you start, extend and end sessions with the mouse. It talks to the same service as the CLI, so a session started in one shows up in the other. There is no downloadable app yet, so you build it from source. It needs macOS 13 or later, the Swift toolchain from Xcode or the Command Line Tools (`xcode-select --install`), and the CLI installed as above.

1. Install the service so the app can start it when needed:

   ```sh
   vigilia install
   ```

   It shows the sudoers rule for lid-closed sessions and asks before installing anything. If you don't need lid-closed sessions, run `vigilia install --skip-sudoers` instead.

2. Build the app and start it in the background:

   ```sh
   cd apps/macos
   swift build -c release
   nohup .build/release/Vigilia >/dev/null 2>&1 &
   ```

The app has no Dock icon. Click the lantern in the menu bar to open it; it is lit while a session keeps the Mac awake. Leave the timeout empty to stay awake until you hold the end button, or set one (`2h`, `45m`) to see the time left next to the lantern. Quitting the app (Quit in its menu) leaves a running session alone.

Without `vigilia install`, the app can still show sessions you start from the CLI, but it can't start the service itself. To let it, run it with `VIGILIA_BIN="$(command -v vigilia)"` set.

A binary built this way isn't an app bundle, so launch at login and in-app setup stay disabled in Settings. They need the packaged app, which isn't available yet.

### Uninstall

To remove the menu bar app:

1. Quit it from its menu (Quit), or from a terminal:

   ```sh
   pkill -f .build/release/Vigilia
   ```

   Quitting doesn't end a running session. Run `vigilia stop` first if you want that too.

2. Delete the build and the app's saved preferences:

   ```sh
   rm -rf apps/macos/.build
   defaults delete Vigilia
   ```

The app installs nothing else: it isn't in Applications, isn't a login item, and has no LaunchAgent of its own.

To remove Vigilia completely, also remove the service and the CLI:

```sh
vigilia stop                                     # end any session; restores normal sleep
vigilia uninstall                                # LaunchAgents and the sudoers rule
rm -rf ~/Library/Application\ Support/vigilia    # config, journal, logs, socket
cargo uninstall vigilia                          # the CLI binary
```

`vigilia uninstall` shows what it removes and asks first; add `--skip-sudoers` to keep the sudoers rule.

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
