//! An in-memory platform for tests. Clones share state, so a test can keep one handle and give
//! another to the service.

use std::collections::BTreeSet;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::mpsc;
use vigilia_core::{ActionKind, PendingAction, UndoRecord};
use vigilia_protocol::{LidState, PowerSnapshot, ThermalLevel};

use crate::{BACKSTOP_LABEL, Capabilities, Platform, PlatformError, Result};

#[derive(Debug)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "independent fault-injection and state flags for tests"
)]
struct State {
    held: BTreeSet<u32>,
    next_id: u32,
    sleep_disabled: bool,
    backstop_registered: bool,
    fail_keep_awake: bool,
    fail_release: bool,
    fail_backstop: bool,
    restored: Vec<PendingAction>,
    announce_count: usize,
    power: PowerSnapshot,
    lid_tx: Option<mpsc::UnboundedSender<LidState>>,
    power_tx: Option<mpsc::UnboundedSender<PowerSnapshot>>,
    lid_rx: Option<mpsc::UnboundedReceiver<LidState>>,
    power_rx: Option<mpsc::UnboundedReceiver<PowerSnapshot>>,
}

impl Default for State {
    fn default() -> Self {
        let (lid_tx, lid_rx) = mpsc::unbounded_channel();
        let (power_tx, power_rx) = mpsc::unbounded_channel();
        Self {
            held: BTreeSet::new(),
            next_id: 0,
            sleep_disabled: false,
            backstop_registered: false,
            fail_keep_awake: false,
            fail_release: false,
            fail_backstop: false,
            restored: Vec::new(),
            announce_count: 0,
            power: PowerSnapshot {
                battery_percent: Some(100),
                on_ac: true,
                thermal: ThermalLevel::Nominal,
            },
            lid_tx: Some(lid_tx),
            power_tx: Some(power_tx),
            lid_rx: Some(lid_rx),
            power_rx: Some(power_rx),
        }
    }
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

    /// Whether sleep is disabled, as a lid-closed session leaves it.
    pub fn sleep_disabled(&self) -> bool {
        self.0.lock().unwrap().sleep_disabled
    }

    pub fn backstop_registered(&self) -> bool {
        self.0.lock().unwrap().backstop_registered
    }

    pub fn fail_keep_awake(&self, fail: bool) {
        self.0.lock().unwrap().fail_keep_awake = fail;
    }

    pub fn fail_release(&self, fail: bool) {
        self.0.lock().unwrap().fail_release = fail;
    }

    pub fn fail_backstop(&self, fail: bool) {
        self.0.lock().unwrap().fail_backstop = fail;
    }

    /// Pretends the lid moved; delivered on the lid-events channel.
    pub fn simulate_lid(&self, state: LidState) {
        let s = self.0.lock().unwrap();
        if let Some(tx) = &s.lid_tx {
            let _ = tx.send(state);
        }
    }

    /// Sets the current power reading and delivers it on the power-events channel.
    pub fn emit_power(&self, power: PowerSnapshot) {
        let mut s = self.0.lock().unwrap();
        s.power = power;
        if let Some(tx) = &s.power_tx {
            let _ = tx.send(power);
        }
    }

    /// Sets the current power reading without an event, as read at session start.
    pub fn set_power(&self, power: PowerSnapshot) {
        self.0.lock().unwrap().power = power;
    }

    /// Actions passed to [`Platform::restore`], in order.
    pub fn restored(&self) -> Vec<PendingAction> {
        self.0.lock().unwrap().restored.clone()
    }

    /// How many times [`Platform::announce_session_change`] was called.
    pub fn announce_count(&self) -> usize {
        self.0.lock().unwrap().announce_count
    }
}

impl Platform for FakePlatform {
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
        let mut s = self.0.lock().unwrap();
        if s.fail_keep_awake {
            return Err(PlatformError::Os {
                op: "keep_awake",
                detail: "injected failure".into(),
            });
        }
        s.next_id += 1;
        let id = s.next_id;
        s.held.insert(id);
        if lid_closed_too {
            let was = s.sleep_disabled;
            s.sleep_disabled = true;
            return Ok(UndoRecord::LidClosedAwake {
                pid: std::process::id(),
                assertion_id: id,
                sleep_was_disabled: was,
            });
        }
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
        match rec {
            UndoRecord::IdleSleepAssertion { assertion_id, .. } => {
                s.held.remove(assertion_id);
            }
            UndoRecord::LidClosedAwake {
                assertion_id,
                sleep_was_disabled,
                ..
            } => {
                s.held.remove(assertion_id);
                s.sleep_disabled = *sleep_was_disabled;
            }
            _ => {}
        }
        Ok(())
    }

    fn restore(&self, action: &PendingAction) -> Result<()> {
        match action.kind {
            ActionKind::KeepAwake => {
                if let Some(rec) = &action.undo {
                    // A crashed process's assertion is already gone; only the persisted
                    // disablesleep flag needs restoring, which release_awake also does.
                    self.release_awake(rec)?;
                }
            }
            ActionKind::Backstop => {
                self.0.lock().unwrap().backstop_registered = false;
            }
        }
        self.0.lock().unwrap().restored.push(action.clone());
        Ok(())
    }

    fn lid_events(&self) -> Option<mpsc::UnboundedReceiver<LidState>> {
        self.0.lock().unwrap().lid_rx.take()
    }

    fn power_events(&self) -> Option<mpsc::UnboundedReceiver<PowerSnapshot>> {
        self.0.lock().unwrap().power_rx.take()
    }

    fn power_snapshot(&self) -> Option<PowerSnapshot> {
        Some(self.0.lock().unwrap().power)
    }

    fn register_backstop(&self, _program: &Path, _interval: Duration) -> Result<UndoRecord> {
        let mut s = self.0.lock().unwrap();
        if s.fail_backstop {
            return Err(PlatformError::Os {
                op: "register_backstop",
                detail: "injected failure".into(),
            });
        }
        s.backstop_registered = true;
        Ok(UndoRecord::Backstop {
            label: BACKSTOP_LABEL.into(),
            plist: String::new(),
        })
    }

    fn unregister_backstop(&self, _rec: &UndoRecord) -> Result<()> {
        self.0.lock().unwrap().backstop_registered = false;
        Ok(())
    }

    fn remove_backstop(&self) -> Result<()> {
        self.0.lock().unwrap().backstop_registered = false;
        Ok(())
    }

    fn announce_session_change(&self) {
        self.0.lock().unwrap().announce_count += 1;
    }
}
