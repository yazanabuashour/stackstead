use std::{
    fs::{File, OpenOptions},
    io::{Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use fs2::FileExt;

use crate::error::StacksteadError;

const LOCK_WAIT_TIMEOUT: Duration = Duration::from_secs(30);
const LOCK_RETRY_INTERVAL: Duration = Duration::from_millis(50);

fn open_lock(path: &Path, create: bool) -> std::io::Result<File> {
    let mut options = OpenOptions::new();
    options.create(create).read(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;

        options.custom_flags(libc::O_NOFOLLOW);
    }
    options.open(path)
}

fn wait_for_lock(
    file: &File,
    path: &Path,
    kind: &'static str,
    shared: bool,
    timeout: Duration,
) -> anyhow::Result<()> {
    let started = Instant::now();
    loop {
        let result = if shared {
            FileExt::try_lock_shared(file)
        } else {
            FileExt::try_lock_exclusive(file)
        };
        match result {
            Ok(()) => return Ok(()),
            Err(error) if error.kind() != fs2::lock_contended_error().kind() => {
                return Err(error.into());
            }
            Err(_) if started.elapsed() >= timeout => {
                return Err(StacksteadError::LockBusy {
                    kind,
                    path: path.to_path_buf(),
                }
                .into());
            }
            Err(_) => std::thread::sleep(
                LOCK_RETRY_INTERVAL.min(timeout.saturating_sub(started.elapsed())),
            ),
        }
    }
}

pub struct LockGuard {
    file: File,
    unlock_on_drop: bool,
}

impl LockGuard {
    pub fn acquire(path: &Path, kind: &'static str) -> anyhow::Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut file = open_lock(path, true)?;
        wait_for_lock(&file, path, kind, false, LOCK_WAIT_TIMEOUT)?;
        file.set_len(0)?;
        file.seek(SeekFrom::Start(0))?;
        writeln!(
            file,
            "pid={} acquired_at={}",
            std::process::id(),
            chrono::Utc::now()
        )?;
        file.flush()?;
        Ok(Self {
            file,
            unlock_on_drop: true,
        })
    }

    pub fn acquire_existing(path: &Path, kind: &'static str) -> anyhow::Result<Self> {
        Self::open_existing(path, kind, false)
    }

    pub fn acquire_existing_shared(path: &Path, kind: &'static str) -> anyhow::Result<Self> {
        Self::open_existing(path, kind, true)
    }

    fn open_existing(path: &Path, kind: &'static str, shared: bool) -> anyhow::Result<Self> {
        let mut file = open_lock(path, false).map_err(|error| {
            anyhow::anyhow!(
                "cannot acquire {kind} lock at {} because the stackstead no longer exists: {error}",
                path.display()
            )
        })?;
        wait_for_lock(&file, path, kind, shared, LOCK_WAIT_TIMEOUT)?;
        if !shared {
            file.set_len(0)?;
            file.seek(SeekFrom::Start(0))?;
            writeln!(
                file,
                "pid={} acquired_at={}",
                std::process::id(),
                chrono::Utc::now()
            )?;
            file.flush()?;
        }
        Ok(Self {
            file,
            unlock_on_drop: true,
        })
    }

    pub fn can_acquire(path: &Path) -> bool {
        let file = match open_lock(path, false) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return true,
            Err(_) => return false,
        };
        file.try_lock_exclusive().is_ok()
    }

    pub fn inherit_on_exec(&self) -> anyhow::Result<()> {
        #[cfg(unix)]
        {
            crate::supervisor::set_cloexec(&self.file, false)?;
        }
        Ok(())
    }

    #[cfg(unix)]
    pub(crate) fn inherited_identity(&self) -> std::io::Result<(i32, u64, u64)> {
        use std::os::{fd::AsRawFd, unix::fs::MetadataExt};

        let metadata = self.file.metadata()?;
        Ok((self.file.as_raw_fd(), metadata.dev(), metadata.ino()))
    }

    #[cfg(unix)]
    pub(crate) fn close_after_handoff(mut self) {
        self.unlock_on_drop = false;
    }

    pub fn downgrade_to_shared(self) -> anyhow::Result<Self> {
        FileExt::unlock(&self.file)?;
        FileExt::try_lock_shared(&self.file)?;
        Ok(self)
    }
}

impl Drop for LockGuard {
    fn drop(&mut self) {
        if self.unlock_on_drop {
            let _ = self.file.unlock();
        }
    }
}

pub fn project_lock_path(project_state_dir: &Path) -> PathBuf {
    project_state_dir.join("project.lock")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn lock_acquisition_rejects_symlinks_without_modifying_the_target() {
        use std::os::unix::fs::symlink;

        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("target");
        let lock = directory.path().join("lock");
        std::fs::write(&target, b"unchanged").unwrap();
        symlink(&target, &lock).unwrap();

        assert!(LockGuard::acquire(&lock, "stackstead").is_err());
        assert!(LockGuard::acquire_existing(&lock, "stackstead").is_err());
        assert!(LockGuard::acquire_existing_shared(&lock, "stackstead").is_err());
        assert!(!LockGuard::can_acquire(&lock));
        assert_eq!(std::fs::read(&target).unwrap(), b"unchanged");
    }

    #[test]
    fn lock_acquisition_preserves_regular_file_behavior() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("lock");

        drop(LockGuard::acquire(&path, "stackstead").unwrap());
        assert!(LockGuard::can_acquire(&path));
        drop(LockGuard::acquire_existing(&path, "stackstead").unwrap());
        drop(LockGuard::acquire_existing_shared(&path, "stackstead").unwrap());
    }

    #[test]
    fn exclusive_lock_can_be_downgraded_for_shared_run_leases() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("lock");
        let lock = LockGuard::acquire(&path, "stackstead").unwrap();

        let lock = lock.downgrade_to_shared().unwrap();
        drop(LockGuard::acquire_existing_shared(&path, "stackstead").unwrap());
        let contender = open_lock(&path, false).unwrap();
        assert!(
            wait_for_lock(
                &contender,
                &path,
                "stackstead",
                false,
                Duration::from_millis(100)
            )
            .is_err()
        );
        drop(lock);
        drop(LockGuard::acquire_existing(&path, "stackstead").unwrap());
    }

    #[cfg(unix)]
    #[test]
    fn handoff_closes_without_unlocking_the_inherited_file_description() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("lock");
        let lock = LockGuard::acquire(&path, "stackstead").unwrap();
        let inherited = lock.file.try_clone().unwrap();

        lock.close_after_handoff();
        assert!(
            open_lock(&path, false)
                .unwrap()
                .try_lock_exclusive()
                .is_err()
        );

        drop(inherited);
        drop(LockGuard::acquire_existing(&path, "stackstead").unwrap());
    }

    #[test]
    fn bounded_wait_acquires_after_the_contender_releases() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("lock");
        let lock = LockGuard::acquire(&path, "stackstead").unwrap();
        let contender = open_lock(&path, false).unwrap();
        let release = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(100));
            drop(lock);
        });

        wait_for_lock(
            &contender,
            &path,
            "stackstead",
            false,
            Duration::from_secs(1),
        )
        .unwrap();
        release.join().unwrap();
    }

    #[test]
    fn existing_lock_acquisition_never_recreates_destroyed_state() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("destroyed/state/lock");
        assert!(LockGuard::acquire_existing(&path, "stackstead").is_err());
        assert!(!directory.path().join("destroyed").exists());
        assert!(LockGuard::acquire_existing_shared(&path, "stackstead").is_err());
        assert!(!directory.path().join("destroyed").exists());
    }
}
