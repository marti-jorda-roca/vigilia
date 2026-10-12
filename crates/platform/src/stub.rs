//! Backend for OSes without an implementation yet. Reports no capabilities and changes nothing.

use vigilia_core::{PendingAction, UndoRecord};

use crate::{Capabilities, Platform, PlatformError, Result};

pub struct StubPlatform;

impl Platform for StubPlatform {
    fn capabilities(&self) -> Capabilities {
        Capabilities::default()
    }

    fn keep_awake(&self, _lid_closed_too: bool) -> Result<UndoRecord> {
        Err(PlatformError::Unsupported("keeping the machine awake"))
    }

    fn release_awake(&self, _rec: &UndoRecord) -> Result<()> {
        Ok(())
    }

    fn restore(&self, _action: &PendingAction) -> Result<()> {
        // This backend never applies anything, so a journal entry from it has nothing to undo.
        Ok(())
    }
}
