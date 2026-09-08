use std::{
    fs::File,
    io::{self, Read as _},
    os::{
        fd::{AsFd as _, OwnedFd},
        unix::net::UnixStream,
    },
    process::Child,
    thread::{self, JoinHandle},
    time::Instant,
};

use anyhow::Context as _;
use rustix::{
    event::{PollFd, PollFlags, Timespec, poll},
    fs::{OFlags, fcntl_getfl, fcntl_setfl},
};

pub(super) type Streams = (Vec<u8>, Vec<u8>);

pub(super) struct Drain {
    cancel: Option<UnixStream>,
    worker: Option<JoinHandle<anyhow::Result<Streams>>>,
}

impl Drain {
    pub(super) fn start(
        child: &mut Child,
        deadline: Instant,
        notification: super::Notify,
    ) -> anyhow::Result<Self> {
        let stdout = child
            .stdout
            .take()
            .context("captured command stdout is missing")?;
        let stderr = child
            .stderr
            .take()
            .context("captured command stderr is missing")?;
        let stdout = Reader::new(OwnedFd::from(stdout))?;
        let stderr = Reader::new(OwnedFd::from(stderr))?;
        let (cancel, receiver) = UnixStream::pair()?;
        let worker = thread::Builder::new()
            .name("command-output".into())
            .spawn(move || {
                let _notification = notification;
                capture(stdout, stderr, &receiver, deadline)
            })?;
        Ok(Self {
            cancel: Some(cancel),
            worker: Some(worker),
        })
    }

    pub(super) fn finish(&mut self) -> anyhow::Result<Streams> {
        let worker = self
            .worker
            .take()
            .context("command output was already joined")?;
        worker
            .join()
            .map_err(|_panic| anyhow::anyhow!("command output reader panicked"))?
    }
}

impl Drop for Drain {
    fn drop(&mut self) {
        // Closing the sole sender wakes poll even when an escaped descendant retains
        // either output pipe. Cancellation never depends on pipe EOF or thread detach.
        drop(self.cancel.take());
        if let Some(worker) = self.worker.take() {
            match worker.join() {
                Ok(Ok(_streams)) => {}
                Ok(Err(error)) => tracing::debug!(%error, "command output reader stopped"),
                Err(_panic) => tracing::error!("command output reader panicked during cleanup"),
            }
        }
    }
}

struct Reader {
    pipe: Option<File>,
    bytes: Vec<u8>,
}

impl Reader {
    fn new(fd: OwnedFd) -> io::Result<Self> {
        fcntl_setfl(&fd, fcntl_getfl(&fd)? | OFlags::NONBLOCK)?;
        Ok(Self {
            pipe: Some(File::from(fd)),
            bytes: Vec::new(),
        })
    }

    fn read(&mut self, events: PollFlags) -> anyhow::Result<()> {
        if events.contains(PollFlags::NVAL) {
            anyhow::bail!("command output descriptor became invalid");
        }
        if events.is_empty() {
            return Ok(());
        }
        if let Some(pipe) = &mut self.pipe {
            // This is only a copy buffer, not an output cap or a timing budget.
            let mut buffer = [0; 8192];
            match pipe.read(&mut buffer) {
                Ok(0) => self.pipe = None,
                Ok(count) => self.bytes.extend(buffer.iter().take(count)),
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                    ) => {}
                Err(error) => return Err(error).context("could not capture command output"),
            }
        }
        if events.contains(PollFlags::ERR) {
            anyhow::bail!("command output pipe reported an error");
        }
        Ok(())
    }
}

fn capture(
    mut stdout: Reader,
    mut stderr: Reader,
    cancel: &UnixStream,
    deadline: Instant,
) -> anyhow::Result<Streams> {
    while stdout.pipe.is_some() || stderr.pipe.is_some() {
        super::check_deadline(deadline)?;
        let remaining = deadline.saturating_duration_since(Instant::now());
        let timeout =
            Timespec::try_from(remaining).context("command output deadline is out of range")?;
        let events = {
            // A closed stream uses the cancellation descriptor with no requested events.
            // Unlike a retained EOF pipe, it cannot cause a healthy loop to spin.
            let out = stdout
                .pipe
                .as_ref()
                .map_or_else(|| cancel.as_fd(), File::as_fd);
            let err = stderr
                .pipe
                .as_ref()
                .map_or_else(|| cancel.as_fd(), File::as_fd);
            let mut fds = [
                PollFd::new(cancel, PollFlags::IN),
                PollFd::from_borrowed_fd(
                    out,
                    if stdout.pipe.is_some() {
                        PollFlags::IN
                    } else {
                        PollFlags::empty()
                    },
                ),
                PollFd::from_borrowed_fd(
                    err,
                    if stderr.pipe.is_some() {
                        PollFlags::IN
                    } else {
                        PollFlags::empty()
                    },
                ),
            ];
            match poll(&mut fds, Some(&timeout)) {
                Ok(_) => {}
                Err(rustix::io::Errno::INTR) => continue,
                Err(error) => return Err(error).context("could not wait for command output"),
            }
            fds.map(|fd| fd.revents())
        };
        let [control, out, err] = events;
        if !control.is_empty() {
            anyhow::bail!("command output capture cancelled");
        }
        stdout.read(out)?;
        stderr.read(err)?;
    }
    Ok((stdout.bytes, stderr.bytes))
}

#[cfg(test)]
#[path = "../drain_tests.rs"]
mod tests;
