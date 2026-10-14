//! macOS backend: idle-sleep assertion, `pmset disablesleep` for lid-closed sessions,
//! IOKit/IOPS lid and power events, and the launchd backstop.

mod assertion;
mod backstop;
mod events;
mod power;
mod timer;

pub use power::sleep_disabled;
pub use timer::GcdTimer;

use std::path::Path;
use std::sync::Mutex;
use std::time::Duration;

use tokio::sync::mpsc;
use vigilia_core::{ActionKind, PendingAction, UndoRecord};
use vigilia_protocol::{LidState, PowerSnapshot};

use crate::{Capabilities, Platform, PlatformError, Result};

const ASSERTION_NAME: &str = "Vigilia session";

#[derive(Debug)]
pub struct MacPlatform {
    lid_rx: Mutex<Option<mpsc::UnboundedReceiver<LidState>>>,
    power_rx: Mutex<Option<mpsc::UnboundedReceiver<PowerSnapshot>>>,
}

impl Default for MacPlatform {
    fn default() -> Self {
        Self::new()
    }
}

impl MacPlatform {
    pub fn new() -> Self {
        let (lid_tx, lid_rx) = mpsc::unbounded_channel();
        let (power_tx, power_rx) = mpsc::unbounded_channel();
        events::spawn(lid_tx, power_tx);
        Self {
            lid_rx: Mutex::new(Some(lid_rx)),
            power_rx: Mutex::new(Some(power_rx)),
        }
    }
}

impl Platform for MacPlatform {
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            keep_awake_idle: true,
            keep_awake_lid_closed: true,
            lid_events: true,
            power_events: true,
            backstop: true,
            socket_activation: false,
        }
    }

    fn keep_awake(&self, lid_closed_too: bool) -> Result<UndoRecord> {
        if !lid_closed_too {
            let id = create_assertion()?;
            return Ok(UndoRecord::IdleSleepAssertion {
                pid: std::process::id(),
                assertion_id: id,
            });
        }
        // Disable sleep first: if sudo refuses, nothing has changed yet.
        let sleep_was_disabled = power::set_sleep_disabled(true)?;
        match create_assertion() {
            Ok(id) => Ok(UndoRecord::LidClosedAwake {
                pid: std::process::id(),
                assertion_id: id,
                sleep_was_disabled,
            }),
            Err(e) => {
                let _ = power::set_sleep_disabled(sleep_was_disabled);
                Err(e)
            }
        }
    }

    fn release_awake(&self, rec: &UndoRecord) -> Result<()> {
        match *rec {
            UndoRecord::IdleSleepAssertion { pid, assertion_id } => {
                release_assertion(pid, assertion_id)
            }
            UndoRecord::LidClosedAwake {
                pid,
                assertion_id,
                sleep_was_disabled,
            } => {
                release_assertion(pid, assertion_id)?;
                power::set_sleep_disabled(sleep_was_disabled).map(|_| ())
            }
            _ => Ok(()),
        }
    }

    fn restore(&self, action: &PendingAction) -> Result<()> {
        // The idle-sleep assertion died with its process, and a backstop left by a crashed
        // service must remove itself. Only the persisted disablesleep flag needs work.
        match (action.kind, &action.undo) {
            (ActionKind::Backstop, _) => backstop::remove(),
            (
                ActionKind::KeepAwake,
                Some(UndoRecord::LidClosedAwake {
                    sleep_was_disabled, ..
                }),
            ) => power::set_sleep_disabled(*sleep_was_disabled).map(|_| ()),
            _ => Ok(()),
        }
    }

    fn lid_events(&self) -> Option<mpsc::UnboundedReceiver<LidState>> {
        self.lid_rx.lock().unwrap().take()
    }

    fn power_events(&self) -> Option<mpsc::UnboundedReceiver<PowerSnapshot>> {
        self.power_rx.lock().unwrap().take()
    }

    fn power_snapshot(&self) -> Option<PowerSnapshot> {
        Some(power::power_snapshot())
    }

    fn register_backstop(&self, program: &Path, interval: Duration) -> Result<UndoRecord> {
        backstop::register(program, interval)
    }

    fn unregister_backstop(&self, _rec: &UndoRecord) -> Result<()> {
        backstop::remove()
    }

    fn remove_backstop(&self) -> Result<()> {
        backstop::remove()
    }
}

fn create_assertion() -> Result<u32> {
    assertion::prevent_idle_sleep(ASSERTION_NAME).map_err(|code| PlatformError::Os {
        op: "IOPMAssertionCreateWithName",
        detail: format!("IOReturn {code:#x}"),
    })
}

fn release_assertion(pid: u32, assertion_id: u32) -> Result<()> {
    if pid == std::process::id() && assertion::is_held(assertion_id) {
        assertion::release(assertion_id).map_err(|code| PlatformError::Os {
            op: "IOPMAssertionRelease",
            detail: format!("IOReturn {code:#x}"),
        })?;
    }
    // Held by another (dead) process, already released, or nothing to release.
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keep_awake_then_release() {
        let p = MacPlatform::new();
        let rec = p.keep_awake(false).unwrap();
        let UndoRecord::IdleSleepAssertion { assertion_id, pid } = rec else {
            panic!("unexpected record {rec:?}")
        };
        assert_eq!(pid, std::process::id());
        assert!(assertion::is_held(assertion_id));
        p.release_awake(&rec).unwrap();
        assert!(!assertion::is_held(assertion_id));
        p.release_awake(&rec).unwrap();
    }

    #[test]
    fn lid_closed_needs_the_sudoers_rule() {
        // Without `vigilia install`, sudo -n refuses: an error, and sleep stays enabled.
        // With the rule installed this succeeds instead, so only the error shape is checked.
        let p = MacPlatform::new();
        if let Err(e) = p.keep_awake(true) {
            assert!(matches!(e, PlatformError::Os { .. }), "{e}");
            assert!(
                !power::sleep_disabled().unwrap_or(true)
                    || p.release_awake(&UndoRecord::Noop).is_ok()
            );
        }
    }
}
