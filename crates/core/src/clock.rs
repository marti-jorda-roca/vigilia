//! Time sources. Sessions use both clocks: wall time for `ends_at`, monotonic time so a system
//! clock change can't extend a session.

use std::time::Instant;

use chrono::{DateTime, Utc};

/// One reading of both clocks, taken together.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Now {
    pub wall: DateTime<Utc>,
    pub mono: Instant,
}

pub trait Clock: Send + Sync {
    fn now(&self) -> Now;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Now {
        Now {
            wall: Utc::now(),
            mono: Instant::now(),
        }
    }
}
