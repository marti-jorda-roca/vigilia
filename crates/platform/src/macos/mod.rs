//! macOS backend. Step 1: only the idle-sleep power assertion.

mod assertion;
mod timer;

pub use timer::GcdTimer;

use vigilia_core::{PendingAction, UndoRecord};

use crate::{Capabilities, Platform, PlatformError, Result};

const ASSERTION_NAME: &str = "Vigilia session";

#[derive(Debug)]
pub struct MacPlatform;

impl Platform for MacPlatform {
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            keep_awake_idle: true,
            ..Capabilities::default()
        }
    }

    fn keep_awake(&self, lid_closed_too: bool) -> Result<UndoRecord> {
        if lid_closed_too {
            return Err(PlatformError::Unsupported(
                "keeping awake with the lid closed",
            ));
        }
        let id =
            assertion::prevent_idle_sleep(ASSERTION_NAME).map_err(|code| PlatformError::Os {
                op: "IOPMAssertionCreateWithName",
                detail: format!("IOReturn {code:#x}"),
            })?;
        Ok(UndoRecord::IdleSleepAssertion {
            pid: std::process::id(),
            assertion_id: id,
        })
    }

    fn release_awake(&self, rec: &UndoRecord) -> Result<()> {
        match *rec {
            UndoRecord::IdleSleepAssertion { pid, assertion_id }
                if pid == std::process::id() && assertion::is_held(assertion_id) =>
            {
                assertion::release(assertion_id).map_err(|code| PlatformError::Os {
                    op: "IOPMAssertionRelease",
                    detail: format!("IOReturn {code:#x}"),
                })
            }
            // Held by another (dead) process, already released, or nothing to release.
            _ => Ok(()),
        }
    }

    fn restore(&self, _action: &PendingAction) -> Result<()> {
        // Restore only runs at startup, for actions left by an earlier process. The only action
        // so far is the idle-sleep assertion, and it died with that process: the OS releases
        // assertions when their owner exits. We never release by id here, because the id may
        // now belong to another process.
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keep_awake_then_release() {
        let p = MacPlatform;
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
    fn lid_closed_is_not_supported_yet() {
        assert!(matches!(
            MacPlatform.keep_awake(true),
            Err(PlatformError::Unsupported(_))
        ));
    }
}
