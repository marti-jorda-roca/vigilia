//! The instance lock. The service holds it for its whole life, and `vigilia recover` (step 2)
//! takes the same lock, so only one process changes OS state at a time. It also settles the race
//! when two clients each start a service: the second one finds the lock taken and exits.

use std::fs::{File, OpenOptions, TryLockError};
use std::io::{self, Write};
use std::path::Path;

/// Released when dropped, or by the OS when the process dies.
#[derive(Debug)]
pub struct InstanceLock {
    _file: File,
}

/// `Ok(None)` when another process holds the lock.
pub fn try_acquire(path: &Path) -> io::Result<Option<InstanceLock>> {
    let mut file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(path)?;
    match file.try_lock() {
        Ok(()) => {}
        Err(TryLockError::WouldBlock) => return Ok(None),
        Err(TryLockError::Error(e)) => return Err(e),
    }
    // The pid is only for people debugging; the lock itself is what matters.
    file.set_len(0)?;
    writeln!(file, "{}", std::process::id())?;
    Ok(Some(InstanceLock { _file: file }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn second_acquire_fails_until_the_first_is_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vigilia.lock");
        let first = try_acquire(&path).unwrap().expect("first lock");
        assert!(try_acquire(&path).unwrap().is_none());
        drop(first);
        assert!(try_acquire(&path).unwrap().is_some());
    }
}
