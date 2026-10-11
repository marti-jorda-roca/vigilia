//! An in-memory platform for tests. Clones share state, so a test can keep one handle and give
//! another to the service.

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};

use vigilia_core::{PendingAction, UndoRecord};

use crate::{Capabilities, Platform, PlatformError, Result};

#[derive(Debug, Default)]
struct State {
    held: BTreeSet<u32>,
    next_id: u32,
    fail_keep_awake: bool,
    fail_release: bool,
    restored: Vec<PendingAction>,
}

#[derive(Debug, Clone, Default)]
pub struct FakePlatform(Arc<Mutex<State>>);

impl FakePlatform {
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether any keep-awake assertion is currently held.
    pub fn awake(&self) -> bool {
        !self.0.lock().unwrap().held.is_empty()
    }

    pub fn fail_keep_awake(&self, fail: bool) {
        self.0.lock().unwrap().fail_keep_awake = fail;
    }

    pub fn fail_release(&self, fail: bool) {
        self.0.lock().unwrap().fail_release = fail;
    }

    /// Actions passed to [`Platform::restore`], in order.
    pub fn restored(&self) -> Vec<PendingAction> {
        self.0.lock().unwrap().restored.clone()
    }
}

impl Platform for FakePlatform {
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            keep_awake_idle: true,
            ..Capabilities::default()
        }
    }

    fn keep_awake(&self, lid_closed_too: bool) -> Result<UndoRecord> {
        let mut s = self.0.lock().unwrap();
        if lid_closed_too {
            return Err(PlatformError::Unsupported(
                "keeping awake with the lid closed",
            ));
        }
        if s.fail_keep_awake {
            return Err(PlatformError::Os {
                op: "keep_awake",
                detail: "injected failure".into(),
            });
        }
        s.next_id += 1;
        let id = s.next_id;
        s.held.insert(id);
        Ok(UndoRecord::IdleSleepAssertion {
            pid: std::process::id(),
            assertion_id: id,
        })
    }

    fn release_awake(&self, rec: &UndoRecord) -> Result<()> {
        let mut s = self.0.lock().unwrap();
        if s.fail_release {
            return Err(PlatformError::Os {
                op: "release_awake",
                detail: "injected failure".into(),
            });
        }
        if let UndoRecord::IdleSleepAssertion { assertion_id, .. } = rec {
            s.held.remove(assertion_id);
        }
        Ok(())
    }

    fn restore(&self, action: &PendingAction) -> Result<()> {
        if let Some(rec) = &action.undo {
            self.release_awake(rec)?;
        }
        self.0.lock().unwrap().restored.push(action.clone());
        Ok(())
    }
}
