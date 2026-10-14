#!/usr/bin/env bash
# The Swift counterpart of `cargo fmt --check` and `cargo clippy -- -D warnings`: swift-format's
# formatting and lint rules (configured in .swift-format), then a build that fails on any compiler
# warning. `swift format` ships with the Swift toolchain, so there is nothing to install.
# To fix the formatting: `swift format --in-place --parallel -r Sources Tests Package.swift`.
set -euo pipefail
cd "$(dirname "$0")/.."

swift format lint --strict --parallel -r Sources Tests Package.swift
swift build -Xswiftc -warnings-as-errors
