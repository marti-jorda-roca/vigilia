# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- macOS menu bar app (`apps/macos`): a countdown next to the clock, start / extend /
  hold to end, battery and heat settings, and recovery warnings.
- The menu bar app as a universal `Vigilia-<version>.dmg` on each GitHub release, built
  by `apps/macos/scripts/build-app.sh`. It is signed and notarized when the release
  secrets are set, and ad-hoc signed otherwise.
- In-app setup: installs the service and, with the admin password dialog, the lid-closed
  sudoers rule and a `/usr/local/bin/vigilia` link. It offers to repair the service when
  the app moves, and uninstalls it all again.
- `vigilia config get/set`, `[rules] end_at_critical_heat`, and `--skip-sudoers` for
  `install` / `uninstall`.
- Open-source project files: `CONTRIBUTING.md`, `SECURITY.md`,
  `CODE_OF_CONDUCT.md`, issue/PR templates, `CODEOWNERS`, and Dependabot.

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
