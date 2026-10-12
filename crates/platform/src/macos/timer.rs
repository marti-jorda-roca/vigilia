//! A Grand Central Dispatch wall-clock timer.
//!
//! `dispatch_walltime` deadlines follow the real time of day, so the timer fires on time after
//! the Mac wakes from a sleep, and the leeway lets the OS group the wake-up with other work.

use std::ffi::c_void;
use std::time::Duration;

use chrono::{DateTime, Utc};
use tokio::sync::mpsc;

use crate::timer::{DeadlineTimer, TimerFired};

#[repr(C)]
struct DispatchSourceType {
    _private: [u8; 0],
}

type DispatchObject = *mut c_void;
type DispatchTime = u64;

const DISPATCH_TIME_FOREVER: DispatchTime = !0;
/// `QOS_CLASS_UTILITY`: low priority, energy-efficient.
const QOS_CLASS_UTILITY: isize = 0x11;

unsafe extern "C" {
    static _dispatch_source_type_timer: DispatchSourceType;
    fn dispatch_get_global_queue(identifier: isize, flags: usize) -> DispatchObject;
    fn dispatch_source_create(
        kind: *const DispatchSourceType,
        handle: usize,
        mask: usize,
        queue: DispatchObject,
    ) -> DispatchObject;
    fn dispatch_source_set_timer(
        source: DispatchObject,
        start: DispatchTime,
        interval: u64,
        leeway: u64,
    );
    fn dispatch_source_set_event_handler_f(
        source: DispatchObject,
        handler: unsafe extern "C" fn(*mut c_void),
    );
    fn dispatch_source_set_cancel_handler_f(
        source: DispatchObject,
        handler: unsafe extern "C" fn(*mut c_void),
    );
    fn dispatch_set_context(object: DispatchObject, context: *mut c_void);
    fn dispatch_resume(object: DispatchObject);
    fn dispatch_source_cancel(source: DispatchObject);
    fn dispatch_release(object: DispatchObject);
    fn dispatch_walltime(when: *const libc::timespec, delta: i64) -> DispatchTime;
}

type Sender = mpsc::UnboundedSender<()>;

unsafe extern "C" fn on_fire(ctx: *mut c_void) {
    // SAFETY: `ctx` is the boxed sender set in `GcdTimer::new`; it lives until the cancel
    // handler runs, which GCD orders after every event handler.
    let tx = unsafe { &*(ctx as *const Sender) };
    let _ = tx.send(());
}

unsafe extern "C" fn on_cancel(ctx: *mut c_void) {
    // SAFETY: runs exactly once, after the last event handler; reclaims the box.
    drop(unsafe { Box::from_raw(ctx as *mut Sender) });
}

pub struct GcdTimer {
    source: DispatchObject,
}

// SAFETY: dispatch objects are thread-safe and reference counted by GCD.
unsafe impl Send for GcdTimer {}

impl GcdTimer {
    pub fn new() -> (Self, TimerFired) {
        let (tx, rx) = mpsc::unbounded_channel();
        let ctx = Box::into_raw(Box::new(tx)) as *mut c_void;
        // SAFETY: standard GCD setup on a global queue. The source starts suspended; we set its
        // context and handlers, park it at "never", and resume it.
        let source = unsafe {
            let queue = dispatch_get_global_queue(QOS_CLASS_UTILITY, 0);
            let source =
                dispatch_source_create(&raw const _dispatch_source_type_timer, 0, 0, queue);
            assert!(!source.is_null(), "dispatch_source_create failed");
            dispatch_set_context(source, ctx);
            dispatch_source_set_event_handler_f(source, on_fire);
            dispatch_source_set_cancel_handler_f(source, on_cancel);
            dispatch_source_set_timer(source, DISPATCH_TIME_FOREVER, DISPATCH_TIME_FOREVER, 0);
            dispatch_resume(source);
            source
        };
        (Self { source }, rx)
    }
}

impl DeadlineTimer for GcdTimer {
    fn arm(&mut self, at: DateTime<Utc>, leeway: Duration) {
        let when = libc::timespec {
            tv_sec: at.timestamp() as libc::time_t,
            tv_nsec: at.timestamp_subsec_nanos() as libc::c_long,
        };
        let leeway = u64::try_from(leeway.as_nanos()).unwrap_or(u64::MAX);
        // SAFETY: `self.source` is a live timer source; a one-shot timer has interval FOREVER.
        unsafe {
            let start = dispatch_walltime(&when, 0);
            dispatch_source_set_timer(self.source, start, DISPATCH_TIME_FOREVER, leeway);
        }
    }

    fn disarm(&mut self) {
        // SAFETY: as in `arm`; a start of FOREVER means it never fires.
        unsafe {
            dispatch_source_set_timer(self.source, DISPATCH_TIME_FOREVER, DISPATCH_TIME_FOREVER, 0)
        };
    }
}

impl Drop for GcdTimer {
    fn drop(&mut self) {
        // SAFETY: cancel runs `on_cancel` (freeing the context) once GCD is done with the
        // source; release drops our reference.
        unsafe {
            dispatch_source_cancel(self.source);
            dispatch_release(self.source);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    #[tokio::test]
    async fn fires_once_within_leeway() {
        let (mut timer, mut fired) = GcdTimer::new();
        let start = Instant::now();
        timer.arm(
            Utc::now() + Duration::from_millis(200),
            Duration::from_millis(50),
        );
        tokio::time::timeout(Duration::from_secs(2), fired.recv())
            .await
            .expect("timer fired")
            .unwrap();
        let elapsed = start.elapsed();
        assert!(elapsed >= Duration::from_millis(190), "{elapsed:?}");
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(fired.try_recv().is_err(), "one-shot timer fired twice");
    }

    #[tokio::test]
    async fn disarm_and_drop() {
        let (mut timer, mut fired) = GcdTimer::new();
        timer.arm(Utc::now() + Duration::from_millis(100), Duration::ZERO);
        timer.disarm();
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(fired.try_recv().is_err());
        drop(timer);
        tokio::time::sleep(Duration::from_millis(50)).await;
        // The cancel handler dropped the sender, closing the channel.
        assert!(fired.recv().await.is_none());
    }
}
