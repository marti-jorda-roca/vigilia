# Vigilia

Keep your laptop awake for a set time, from the terminal or from a coding agent, and pay almost nothing in battery for it. Later: with the lid closed, and with your agents reachable from your phone.

Vigilia is a per-user Rust service plus a CLI. The concept comes from [Insomnia](https://github.com/krishhgg/Insomnia) (MIT), a macOS menu bar app; Vigilia aims for macOS, Linux and Windows.

- Architecture: [`docs/architecture-plan.md`](docs/architecture-plan.md)
- Build order and release plan: [`docs/execution-plan.md`](docs/execution-plan.md)

## Status

Work in progress towards v0.1 (macOS idle-sleep sessions from the CLI).

## Layout

| Crate | What it holds |
|---|---|
| `crates/protocol` | Wire types (requests, responses, events), `ClientContext` and scopes, the generated JSON Schema |
| `crates/core` | Session state machine, recovery journal, config, paths. No OS code |

## Development

```sh
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all
```

The protocol's JSON Schema in `crates/protocol/schema/protocol.schema.json` is generated from the Rust types, and a test fails when it is stale. After changing protocol types:

```sh
UPDATE_SCHEMA=1 cargo test -p vigilia-protocol --test schema
```

Protocol changes may only add things (new methods, events, optional fields). From v0.1 on, CI will also compare the schema against the previous release's.

## License

MIT
