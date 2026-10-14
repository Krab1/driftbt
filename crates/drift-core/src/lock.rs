//! One program at a time per data folder: the desktop app and the headless server must never run on the same
//! session and settings files. The OS drops the lock when the process dies, however it dies.

use std::fs::{File, OpenOptions, TryLockError};
use std::path::Path;

pub struct DataDirLock {
    _file: File,
}

impl DataDirLock {
    /// `Ok(None)` means another Drift already holds this folder.
    pub fn acquire(dir: &Path) -> std::io::Result<Option<Self>> {
        std::fs::create_dir_all(dir)?;
        let file = OpenOptions::new().create(true).write(true).truncate(false).open(dir.join("drift.lock"))?;
        match file.try_lock() {
            Ok(()) => Ok(Some(Self { _file: file })),
            Err(TryLockError::WouldBlock) => Ok(None),
            Err(TryLockError::Error(e)) => Err(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_second_holder_is_refused_until_the_first_lets_go() {
        let dir = std::env::temp_dir().join(format!("drift-lock-{}", std::process::id()));
        let first = DataDirLock::acquire(&dir).unwrap();
        assert!(first.is_some());
        assert!(DataDirLock::acquire(&dir).unwrap().is_none(), "second instance must be refused");
        drop(first);
        assert!(DataDirLock::acquire(&dir).unwrap().is_some(), "free again after the first exits");
        std::fs::remove_dir_all(dir).ok();
    }
}
