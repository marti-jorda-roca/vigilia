//! OS backends behind one [`Platform`] trait.
//!
//! Each OS gets a module; OSes that don't support something yet report it through
//! [`Capabilities`] and return [`PlatformError::Unsupported`]. Later steps add trait methods
//! with default bodies that return `Unsupported`, so a new feature never forces every backend
//! to change at once.

use std::sync::Arc;

use vigilia_core::{PendingAction, UndoRecord};
pub use vigilia_protocol::Capabilities;

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
}

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
