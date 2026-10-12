<p align="center"><img src="docs/assets/vigilia-banner.svg" alt="Vigilia: a pixel-art guard holding a lantern" width="640"></p>

Keep your laptop awake for a set time, from the terminal or from a coding agent, and pay almost nothing in battery for it.

## Install

There are no prebuilt binaries yet, so for now you build the CLI from source. It needs macOS and [rustup](https://rustup.rs). The Rust version is pinned in `rust-toolchain.toml`, and rustup installs it the first time you build.

```sh
git clone https://github.com/marti-jorda-roca/vigilia.git
cd vigilia
cargo install --path crates/cli --locked
```

This puts a single `vigilia` binary in `~/.cargo/bin`. Make sure that directory is on your `PATH`, then check it works:

```sh
vigilia start 2m
vigilia status --watch
```

You don't need to start the service yourself. `vigilia daemon` is the service, and client commands start it when it isn't running. It exits on its own after 5 idle minutes with no session.

For every command, durations, `--json` output, config and errors, see the [CLI documentation](docs/cli.md).

The `crates/cli` crate is still being written as part of v0.1, so the commands above will work once it lands. A `vigilia install` command (LaunchAgent with socket activation) comes in v0.2, and a signed `.app` comes later.

## Layout

| Crate | What it holds |
|---|---|
| `crates/protocol` | Wire types (requests, responses, events), `ClientContext` and scopes, the generated JSON Schema |
| `crates/core` | Session state machine, recovery journal, config, paths. No OS code |
| `crates/platform` | `Platform` trait and `Capabilities`; macOS backend (idle-sleep assertion, GCD deadline timer), stubs for Linux and Windows, `FakePlatform` for tests |
| `crates/daemon` | The service: dispatch with scope checks, event bus, owner-only Unix socket, instance lock, launchd socket activation, idle exit |

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
