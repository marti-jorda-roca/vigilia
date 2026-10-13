## What changed

<!-- Describe the change and why. Link related issues, e.g. Closes #123. -->

## How it was tested

<!-- Commands run and results, e.g. cargo test --workspace on macOS 15. -->

## Checklist

- [ ] `cargo fmt --all --check` passes
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` passes
- [ ] `cargo test --workspace` passes
- [ ] Protocol schema regenerated if protocol types changed
      (`UPDATE_SCHEMA=1 cargo test -p vigilia-protocol --test schema`)
- [ ] `CHANGELOG.md` updated under Unreleased (user-visible changes)

## Security impact

<!-- Required if touching crates/platform/src/macos/, crates/daemon/src/install.rs,
     crates/daemon/src/transports/, packaging/, or .github/workflows/.
     Otherwise write "None". -->
