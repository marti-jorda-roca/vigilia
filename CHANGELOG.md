# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Open-source project files: `CONTRIBUTING.md`, `SECURITY.md`,
  `CODE_OF_CONDUCT.md`, issue/PR templates, `CODEOWNERS`, and Dependabot.

### Fixed

- `vigilia uninstall` clears a leftover `pmset disablesleep 1` before removing
  the sudoers rule, so the Mac sleeps with the lid closed again.

## [0.2.0] - 2026-10-13

### Added

- `vigilia stop` shows the session duration.
- Release workflow publishing all workspace crates to crates.io.

## [0.1.0] - 2026-10-12

### Added

- Timed keep-awake sessions from the CLI (`start`, `stop`, `extend`, `status`).
- macOS idle-sleep assertion and lid-closed mode via `vigilia install`.
- Per-user daemon with owner-only Unix socket, recovery journal, and
  battery/heat rules.

[Unreleased]: https://github.com/marti-jorda-roca/vigilia/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/marti-jorda-roca/vigilia/releases/tag/v0.2.0
[0.1.0]: https://github.com/marti-jorda-roca/vigilia/releases/tag/v0.1.0
