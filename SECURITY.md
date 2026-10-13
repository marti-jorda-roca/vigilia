# Security Policy

## Supported versions

| Version | Supported |
|---|---|
| 0.2.x | Yes |
| < 0.2 | No |

## Reporting a vulnerability

Please do **not** open a public issue for a security vulnerability.

Report privately through GitHub:
<https://github.com/marti-jorda-roca/vigilia/security/advisories/new>

Alternatively, use GitHub's private vulnerability reporting from the
repository's **Security** tab ("Report a vulnerability").

Include: affected version or commit, platform (macOS version), steps to
reproduce, and impact. You should receive an acknowledgement within 7 days.
Please allow a reasonable disclosure window before publishing details.

## Security model and scope

Vigilia runs a per-user background service that can keep a Mac awake,
including with the lid closed. The security-sensitive surface is:

- **Owner-only local socket** (`crates/daemon/src/transports/local.rs`):
  the daemon listens on a Unix socket with `0600` permissions inside a
  `0700` directory and refuses connections from other UIDs.
- **Privileged lid-closed mode** (`crates/daemon/src/install.rs`,
  `packaging/macos/sudoers.vigilia`): `vigilia install` shows and, only
  after confirmation, installs a narrow sudoers rule allowing only
  `/usr/bin/pmset -a disablesleep 1` and `... disablesleep 0`, plus
  LaunchAgent plists for the service and recovery jobs.
- **Unsafe macOS FFI** (`crates/platform/src/macos/`): IOKit power
  assertions, lid/power events, and GCD timers. Every `unsafe` block must
  document why it is sound (`undocumented_unsafe_blocks` is a CI warning).

In scope: bypassing the socket owner check, widening the sudoers rule or
`pmset` invocation, journal/recovery tampering by another local user, and
memory-safety issues in the FFI layer.

Out of scope: attacks requiring an already-compromised account running as
the same user, and physical-access attacks.
