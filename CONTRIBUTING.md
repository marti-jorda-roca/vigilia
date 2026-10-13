# Contributing to Vigilia

Thank you for contributing. Vigilia is Apache-2.0; by submitting a pull
request you agree your contribution is licensed under Apache-2.0 (see
`LICENSE`, section 5). No CLA is required.

## Development setup

You need macOS and [rustup](https://rustup.rs). The toolchain is pinned in
`rust-toolchain.toml` (Rust 1.94, with `rustfmt` and `clippy`).

```sh
git clone https://github.com/marti-jorda-roca/vigilia.git
cd vigilia
cargo test --workspace
cargo install --path crates/cli --locked   # optional: install locally
```

## Before opening a pull request

CI fails on any warning. Run locally first:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Optionally enable the git hooks once per clone:

```sh
brew install pre-commit   # or: pipx install pre-commit / cargo install prek
pre-commit install
pre-commit run --all-files
```

The macOS code only compiles on macOS. To check other platforms from a Mac:

```sh
rustup target add x86_64-unknown-linux-gnu x86_64-pc-windows-msvc
cargo clippy --workspace --all-targets --target x86_64-unknown-linux-gnu -- -D warnings
```

## Protocol changes

The JSON Schema in `crates/protocol/schema/protocol.schema.json` is generated
from the Rust types, and a test fails when it is stale:

```sh
UPDATE_SCHEMA=1 cargo test -p vigilia-protocol --test schema
```

Protocol changes may only add things: new methods, new events, and new
optional fields. Removing or renaming existing fields is a breaking change
and needs maintainer agreement first (open an issue to discuss).

## Security-sensitive areas

Changes to any of the following require maintainer review and should explain
the security impact in the pull request description:

- `crates/platform/src/macos/` — `unsafe` IOKit/GCD FFI
- `crates/daemon/src/install.rs` and `packaging/macos/` — LaunchAgent plists
  and the narrow `pmset disablesleep 1/0` sudoers rule
- `crates/daemon/src/transports/` — the owner-only Unix socket
- `.github/workflows/` — CI and release publishing

Do not report vulnerabilities in public issues; see `SECURITY.md`.

## Pull requests

- Keep PRs focused; describe what changed and how you tested it.
- Fill in the pull request template checklist.
- All CI checks and one maintainer approval are required to merge to `main`.
