//! Single-instance OS advisory file lock for `binance-botd`.

use std::fmt;
use std::fs::{File, OpenOptions, TryLockError};
use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

/// Exclusive OS advisory lock held for the lifetime of the daemon process.
///
/// Releasing (dropping) this guard closes the underlying file descriptor,
/// immediately releasing the OS lock without unlinking the file.
#[derive(Debug)]
pub struct ProcessLock {
    _file: File,
    path: PathBuf,
}

/// Errors returned while acquiring [`ProcessLock`].
#[derive(Debug)]
pub enum ProcessLockError {
    /// Another running process already holds an exclusive lock on `path`.
    AlreadyLocked(PathBuf),
    /// An underlying filesystem or OS locking error occurred.
    Io {
        /// Path associated with the failing I/O operation.
        path: PathBuf,
        /// Underlying OS I/O error.
        source: std::io::Error,
    },
}

impl fmt::Display for ProcessLockError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AlreadyLocked(path) => {
                write!(f, "process lock already held at {}", path.display())
            }
            Self::Io { path, source } => {
                write!(f, "process lock I/O error at {}: {source}", path.display())
            }
        }
    }
}

impl std::error::Error for ProcessLockError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::AlreadyLocked(_) => None,
            Self::Io { source, .. } => Some(source),
        }
    }
}

impl ProcessLock {
    /// Acquire an exclusive non-blocking OS file lock at `path`, creating missing
    /// parent directories and recording only the current process ID in the file.
    ///
    /// # Errors
    ///
    /// Returns [`ProcessLockError::AlreadyLocked`] if another process holds the
    /// lock, or [`ProcessLockError::Io`] if directory creation, file opening,
    /// locking, or writing the PID fails.
    pub fn acquire(path: &Path) -> Result<Self, ProcessLockError> {
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent).map_err(|source| ProcessLockError::Io {
                path: parent.to_path_buf(),
                source,
            })?;
        }

        let mut file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(path)
            .map_err(|source| ProcessLockError::Io {
                path: path.to_path_buf(),
                source,
            })?;

        match file.try_lock() {
            Ok(()) => {}
            Err(TryLockError::WouldBlock) => {
                return Err(ProcessLockError::AlreadyLocked(path.to_path_buf()));
            }
            Err(TryLockError::Error(source)) => {
                return Err(ProcessLockError::Io {
                    path: path.to_path_buf(),
                    source,
                });
            }
        }

        // Truncate and rewind only after holding the exclusive lock so a second
        // instance never clobbers the active holder's PID file.
        file.set_len(0).map_err(|source| ProcessLockError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        file.seek(SeekFrom::Start(0))
            .map_err(|source| ProcessLockError::Io {
                path: path.to_path_buf(),
                source,
            })?;
        writeln!(file, "{}", std::process::id()).map_err(|source| ProcessLockError::Io {
            path: path.to_path_buf(),
            source,
        })?;

        Ok(Self {
            _file: file,
            path: path.to_path_buf(),
        })
    }

    /// Return the path of the held lock file.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

#[cfg(test)]
mod tests {
    use super::{ProcessLock, ProcessLockError};
    use tempfile::tempdir;

    #[test]
    fn exclusive_lock_blocks_second_instance_and_releases_on_drop() {
        let dir = tempdir().expect("tempdir");
        let lock_path = dir.path().join("binance-botd.live.lock");

        let first = ProcessLock::acquire(&lock_path).expect("first lock");
        let second = ProcessLock::acquire(&lock_path);
        assert!(matches!(second, Err(ProcessLockError::AlreadyLocked(_))));

        // Review Focus #5: dropping releases lock immediately.
        drop(first);
        let third = ProcessLock::acquire(&lock_path).expect("re-acquire after drop");
        assert_eq!(third.path(), lock_path.as_path());
    }

    #[test]
    fn acquire_creates_missing_parent_dirs_and_writes_pid_only() {
        let dir = tempdir().expect("tempdir");
        let lock_path = dir
            .path()
            .join("nested")
            .join("state")
            .join("binance-botd.paper.lock");

        let lock = ProcessLock::acquire(&lock_path).expect("acquire with nested dirs");
        let contents = std::fs::read_to_string(lock.path()).expect("read lock file");
        assert_eq!(contents.trim(), std::process::id().to_string());
    }
}
