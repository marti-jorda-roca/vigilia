//! `vigilia recover`: the backstop entry point. Also run once at login.
//!
//! Takes the instance lock, replays unfinished journal entries and restores sleep. When
//! the service is alive it holds the lock, so recover finds nothing to do and leaves; when
//! the journal is clean, any leftover backstop job is removed.

use std::io;
use std::sync::Arc;

use vigilia_core::{Journal, Paths};
use vigilia_platform::Platform;

use crate::lock;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RecoverOutcome {
    /// Actions replayed and undone.
    pub undone: usize,
    /// Actions whose undo failed; they stay in the journal for the next attempt.
    pub failed: usize,
    /// Another process (the service) holds the lock and owns the state.
    pub already_running: bool,
}

/// Replays the journal at `paths` through `platform`. Never fails on a held lock.
pub fn recover(paths: &Paths, platform: Arc<dyn Platform>) -> io::Result<RecoverOutcome> {
    let Some(_lock) = lock::try_acquire(&paths.lock)? else {
        tracing::info!("another instance holds the lock; nothing to recover");
        return Ok(already_running());
    };
    replay(paths, &platform)
}

/// Recovers like [`recover`], then deletes the journal, under the same lock. For
/// `vigilia uninstall`: once the sudoers rule is gone, a later replay could not restore
/// `disablesleep`, so actions whose undo failed are dropped rather than kept for a retry.
pub fn recover_and_clear(paths: &Paths, platform: Arc<dyn Platform>) -> io::Result<RecoverOutcome> {
    let Some(_lock) = lock::try_acquire(&paths.lock)? else {
        return Ok(already_running());
    };
    let outcome = replay(paths, &platform)?;
    match std::fs::remove_file(&paths.journal) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
        _ => {}
    }
    Ok(outcome)
}

fn already_running() -> RecoverOutcome {
    RecoverOutcome {
        already_running: true,
        ..RecoverOutcome::default()
    }
}

/// Replays the journal; the caller holds the lock.
fn replay(paths: &Paths, platform: &Arc<dyn Platform>) -> io::Result<RecoverOutcome> {
    let mut journal = Journal::open(&paths.journal)?;
    let report = journal.replay(|action| platform.restore(action))?;
    for action in &report.undone {
        tracing::info!(id = action.id, kind = ?action.kind, "recovered action");
    }
    for (action, error) in &report.failed {
        tracing::error!(id = action.id, kind = ?action.kind, %error, "recovery failed");
    }
    if report.failed.is_empty() {
        // Clean journal: a stale backstop (e.g. left by a crash after its journal entry was
        // already replayed) has no reason to exist.
        if let Err(e) = platform.remove_backstop() {
            tracing::warn!(error = %e, "could not remove the backstop job");
        }
    }
    Ok(RecoverOutcome {
        undone: report.undone.len(),
        failed: report.failed.len(),
        already_running: false,
    })
}

/// `vigilia recover` as the CLI runs it: the user's paths and the native platform.
pub fn recover_blocking() -> Result<RecoverOutcome, crate::DaemonError> {
    let paths = Paths::for_user()?;
    paths.create_dirs()?;
    Ok(recover(&paths, vigilia_platform::native())?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::convert::Infallible;
    use vigilia_core::{ActionKind, UndoRecord};
    use vigilia_platform::FakePlatform;

    #[test]
    fn replays_leftovers_and_clears_the_backstop() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::in_dir(dir.path().to_path_buf());
        paths.create_dirs().unwrap();
        let platform = FakePlatform::new();
        {
            let mut journal = Journal::open(&paths.journal).unwrap();
            journal
                .apply(ActionKind::KeepAwake, None, || {
                    Ok::<_, Infallible>(UndoRecord::LidClosedAwake {
                        pid: 999_999,
                        assertion_id: 7,
                        sleep_was_disabled: false,
                    })
                })
                .unwrap();
            platform
                .register_backstop(
                    std::path::Path::new("vigilia"),
                    std::time::Duration::from_secs(300),
                )
                .unwrap();
            // Dropped without undo: the previous service was killed mid-session.
        }
        let outcome = recover(&paths, Arc::new(platform.clone())).unwrap();
        assert_eq!(outcome.undone, 1);
        assert_eq!(outcome.failed, 0);
        assert!(!outcome.already_running);
        assert!(Journal::open(&paths.journal).unwrap().pending().is_empty());
        assert!(!platform.sleep_disabled());
        assert!(!platform.backstop_registered());
    }

    #[test]
    fn clearing_deletes_the_journal_even_when_an_undo_fails() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::in_dir(dir.path().to_path_buf());
        paths.create_dirs().unwrap();
        let platform = FakePlatform::new();
        Journal::open(&paths.journal)
            .unwrap()
            .apply(ActionKind::KeepAwake, None, || {
                Ok::<_, Infallible>(UndoRecord::LidClosedAwake {
                    pid: 999_999,
                    assertion_id: 7,
                    sleep_was_disabled: false,
                })
            })
            .unwrap();
        platform.fail_release(true);
        let outcome = recover_and_clear(&paths, Arc::new(platform)).unwrap();
        assert_eq!(outcome.failed, 1);
        assert!(!paths.journal.exists());
    }

    #[test]
    fn clearing_leaves_a_running_service_alone() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::in_dir(dir.path().to_path_buf());
        paths.create_dirs().unwrap();
        Journal::open(&paths.journal).unwrap();
        let _held = lock::try_acquire(&paths.lock).unwrap().unwrap();
        let outcome = recover_and_clear(&paths, Arc::new(FakePlatform::new())).unwrap();
        assert!(outcome.already_running);
        assert!(paths.journal.exists());
    }

    #[test]
    fn a_held_lock_means_the_service_owns_the_state() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::in_dir(dir.path().to_path_buf());
        paths.create_dirs().unwrap();
        let _held = lock::try_acquire(&paths.lock).unwrap().unwrap();
        let outcome = recover(&paths, Arc::new(FakePlatform::new())).unwrap();
        assert!(outcome.already_running);
        assert_eq!(outcome.undone, 0);
    }

    #[test]
    fn nothing_to_do_is_success() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::in_dir(dir.path().to_path_buf());
        paths.create_dirs().unwrap();
        let outcome = recover(&paths, Arc::new(FakePlatform::new())).unwrap();
        assert_eq!(outcome, RecoverOutcome::default());
    }
}
