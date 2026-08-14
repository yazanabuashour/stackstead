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

#[derive(Debug)]
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
            drop(self.file.unlock());
        }
    }
}

pub fn project_lock_path(project_state_dir: &Path) -> PathBuf {
    project_state_dir.join("project.lock")
}

#[cfg(test)]
mod tests;
