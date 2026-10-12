//! Time sources. Sessions use both clocks: wall time for `ends_at`, monotonic time so a system
//! clock change can't extend a session.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

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

/// A clock moved by hand, for tests. Clones share the same time.
#[derive(Debug, Clone)]
pub struct ManualClock(Arc<Mutex<Now>>);

impl ManualClock {
    pub fn new(wall: DateTime<Utc>) -> Self {
        Self(Arc::new(Mutex::new(Now {
            wall,
            mono: Instant::now(),
        })))
    }

    /// Moves both clocks forward, as real time passing does.
    pub fn advance(&self, by: Duration) {
        let mut now = self.0.lock().unwrap();
        now.wall += by;
        now.mono += by;
    }

    /// Moves only the wall clock, as a user or NTP changing the system time does.
    pub fn jump_wall(&self, by: chrono::TimeDelta) {
        self.0.lock().unwrap().wall += by;
    }
}

impl Clock for ManualClock {
    fn now(&self) -> Now {
        *self.0.lock().unwrap()
    }
}
