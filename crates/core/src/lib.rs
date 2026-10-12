//! Vigilia's core: the session state machine, the recovery journal, config and paths.
//! Nothing here talks to the OS beyond plain files; OS changes live in `vigilia-platform`.

pub mod clock;
pub mod config;
pub mod journal;
pub mod paths;
pub mod session;

pub use clock::{Clock, Now, SystemClock};
pub use config::Config;
pub use journal::{ActionError, ActionId, ActionKind, Journal, PendingAction, UndoRecord};
pub use paths::Paths;
pub use session::{SessionError, SessionMachine};
