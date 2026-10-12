//! The service's single deadline timer.
//!
//! The service keeps exactly one armed deadline: the session end, or the idle-exit time when no
//! session runs. Firings arrive on a [`TimerFired`] channel; the service then re-checks its state
//! against the clock, so a late or stale firing is harmless.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use chrono::{DateTime, Utc};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use vigilia_core::{Clock, Now};

/// Receives one `()` per firing.
pub type TimerFired = mpsc::UnboundedReceiver<()>;

pub trait DeadlineTimer: Send {
    /// Fires once at `at` (wall clock), replacing any armed deadline. `leeway` is how late the OS
    /// may fire it so it can group wake-ups with other work.
    fn arm(&mut self, at: DateTime<Utc>, leeway: Duration);

    /// Cancels the armed deadline, if any.
    fn disarm(&mut self);
}

/// A tokio-based timer, used where the OS has no better one, and in tests with paused time.
/// It ignores leeway.
pub struct TokioTimer {
    clock: Arc<dyn Clock>,
    tx: mpsc::UnboundedSender<()>,
    task: Option<JoinHandle<()>>,
    fires: Arc<AtomicUsize>,
}

impl TokioTimer {
    pub fn new(clock: Arc<dyn Clock>) -> (Self, TimerFired) {
        let (tx, rx) = mpsc::unbounded_channel();
        let timer = Self {
            clock,
            tx,
            task: None,
            fires: Arc::default(),
        };
        (timer, rx)
    }

    /// Counts firings, for energy tests.
    pub fn fire_counter(&self) -> Arc<AtomicUsize> {
        self.fires.clone()
    }
}

impl DeadlineTimer for TokioTimer {
    fn arm(&mut self, at: DateTime<Utc>, _leeway: Duration) {
        self.disarm();
        let delay = (at - self.clock.now().wall)
            .to_std()
            .unwrap_or(Duration::ZERO);
        let tx = self.tx.clone();
        let fires = self.fires.clone();
        self.task = Some(tokio::spawn(async move {
            tokio::time::sleep(delay).await;
            fires.fetch_add(1, Ordering::Relaxed);
            let _ = tx.send(());
        }));
    }

    fn disarm(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

impl Drop for TokioTimer {
    fn drop(&mut self) {
        self.disarm();
    }
}

/// A clock that follows tokio's time, so tests with `tokio::time::pause` move it too.
#[derive(Debug, Clone)]
pub struct TokioClock {
    wall_at_start: DateTime<Utc>,
    start: tokio::time::Instant,
}

impl TokioClock {
    pub fn new(wall_at_start: DateTime<Utc>) -> Self {
        Self {
            wall_at_start,
            start: tokio::time::Instant::now(),
        }
    }
}

impl Clock for TokioClock {
    fn now(&self) -> Now {
        let mono = tokio::time::Instant::now();
        Now {
            wall: self.wall_at_start + (mono - self.start),
            mono: mono.into_std(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn fires_once_at_the_deadline_and_rearming_replaces_it() {
        let clock = Arc::new(TokioClock::new("2026-10-12T00:00:00Z".parse().unwrap()));
        let (mut timer, mut fired) = TokioTimer::new(clock.clone());
        let fires = timer.fire_counter();
        let start = clock.now().wall;

        timer.arm(start + Duration::from_secs(60), Duration::ZERO);
        timer.arm(start + Duration::from_secs(120), Duration::ZERO);
        fired.recv().await.unwrap();
        assert_eq!(clock.now().wall, start + Duration::from_secs(120));
        assert_eq!(fires.load(Ordering::Relaxed), 1);

        timer.arm(start + Duration::from_secs(600), Duration::ZERO);
        timer.disarm();
        tokio::time::sleep(Duration::from_secs(3_600)).await;
        assert!(fired.try_recv().is_err());
        assert_eq!(fires.load(Ordering::Relaxed), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn past_deadlines_fire_immediately() {
        let clock = Arc::new(TokioClock::new("2026-10-12T00:00:00Z".parse().unwrap()));
        let (mut timer, mut fired) = TokioTimer::new(clock.clone());
        timer.arm(clock.now().wall - Duration::from_secs(5), Duration::ZERO);
        fired.recv().await.unwrap();
    }
}
