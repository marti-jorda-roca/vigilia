//! OS backends behind one [`Platform`] trait.
//!
//! Each OS gets a module; OSes that don't support something yet report it through
//! [`Capabilities`] and return [`PlatformError::Unsupported`]. Later steps add trait methods
//! with default bodies that return `Unsupported`, so a new feature never forces every backend
//! to change at once.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use vigilia_core::{PendingAction, UndoRecord};
pub use vigilia_protocol::Capabilities;
pub use vigilia_protocol::{LidState, PowerSnapshot, ThermalLevel};

pub mod fake;
pub mod timer;

#[cfg(target_os = "macos")]
pub mod macos;
/// Linux (step 7) and Windows (step 8) start as stubs.
#[cfg(not(target_os = "macos"))]
mod stub;

pub use fake::FakePlatform;
pub use timer::{DeadlineTimer, TimerFired, TokioClock, TokioTimer};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PlatformError {
    #[error("{0} is not supported on this platform yet")]
    Unsupported(&'static str),
    #[error("{op} failed: {detail}")]
    Os { op: &'static str, detail: String },
}

pub type Result<T> = std::result::Result<T, PlatformError>;

pub trait Platform: Send + Sync {
    /// What this OS and machine support.
    fn capabilities(&self) -> Capabilities;

    /// Stops the machine from idle-sleeping. `lid_closed_too` also keeps it running with the lid
    /// closed. The returned record is journaled and passed back to undo the change.
    fn keep_awake(&self, lid_closed_too: bool) -> Result<UndoRecord>;

    /// Undoes [`keep_awake`](Self::keep_awake) in the same process. Idempotent.
    fn release_awake(&self, rec: &UndoRecord) -> Result<()>;

    /// Undoes an action found open in the journal, possibly left by a process that crashed.
    /// Must cope with `applied == false` (the change may or may not have happened). Idempotent.
    fn restore(&self, action: &PendingAction) -> Result<()>;

    /// Lid open/close events, from OS notifications. `None` when unsupported. Taken once.
    fn lid_events(&self) -> Option<tokio::sync::mpsc::UnboundedReceiver<LidState>> {
        None
    }

    /// Battery/heat change events, from OS notifications. `None` when unsupported. Taken once.
    fn power_events(&self) -> Option<tokio::sync::mpsc::UnboundedReceiver<PowerSnapshot>> {
        None
    }

    /// The current battery/heat reading, if the OS can provide one on demand.
    fn power_snapshot(&self) -> Option<PowerSnapshot> {
        None
    }

    /// Registers the OS-scheduled `vigilia recover` backstop (every `interval`) for a session.
    /// The returned record is journaled; undoing it removes the job.
    fn register_backstop(&self, _program: &Path, _interval: Duration) -> Result<UndoRecord> {
        Err(PlatformError::Unsupported("the recovery backstop"))
    }

    /// Undoes [`register_backstop`](Self::register_backstop) in the same process. Idempotent.
    fn unregister_backstop(&self, _rec: &UndoRecord) -> Result<()> {
        Ok(())
    }

    /// Removes any backstop job, whether or not this process registered it. Idempotent; used
    /// by `vigilia recover` when the journal is clean.
    fn remove_backstop(&self) -> Result<()> {
        Ok(())
    }

    /// Posts the session-change doorbell, so a GUI app that is not connected can wake and
    /// reconnect. Default: no-op.
    fn announce_session_change(&self) {}
}

/// How often the backstop runs during a session.
pub const BACKSTOP_INTERVAL: Duration = Duration::from_secs(300);

/// The launchd label / job name of the backstop.
pub const BACKSTOP_LABEL: &str = "com.vigilia.backstop";

/// The backend for the OS this binary was built for.
pub fn native() -> Arc<dyn Platform> {
    #[cfg(target_os = "macos")]
    return Arc::new(macos::MacPlatform::new());
    #[cfg(not(target_os = "macos"))]
    return Arc::new(stub::StubPlatform);
}

/// The deadline timer for this OS: a GCD wall-clock timer on macOS, a tokio timer elsewhere.
/// Must be called inside a tokio runtime.
pub fn native_timer() -> (Box<dyn DeadlineTimer>, TimerFired) {
    #[cfg(target_os = "macos")]
    {
        let (timer, fired) = macos::GcdTimer::new();
        (Box::new(timer), fired)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let (timer, fired) = TokioTimer::new(Arc::new(vigilia_core::SystemClock));
        (Box::new(timer), fired)
    }
}
