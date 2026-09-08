use std::{
    io::{self, Read as _},
    os::unix::net::UnixStream,
    thread::{self, JoinHandle},
    time::Instant,
};

use rustix::{
    event::{PollFd, PollFlags, Timespec, poll},
    process::Pid,
};

use crate::command::observe_child_exit as observe;

#[cfg(test)]
#[path = "wait_tests.rs"]
mod tests;

pub(super) struct Observer {
    completion: UnixStream,
    thread: Option<JoinHandle<io::Result<()>>>,
}

impl Observer {
    pub(super) fn start(pid: Pid) -> io::Result<Self> {
        let (completion, notification) = UnixStream::pair()?;
        let thread = thread::Builder::new()
            .name("run-exit".into())
            .spawn(move || {
                // Closing this endpoint latches completion on success, error, or unwind.
                // The observer neither consumes status nor owns any inherited descriptor.
                let _notification = notification;
                if observe(pid, false)? {
                    Ok(())
                } else {
                    Err(io::Error::other(
                        "blocking run observer returned without exit",
                    ))
                }
            })?;
        Ok(Self {
            completion,
            thread: Some(thread),
        })
    }

    pub(super) fn wait(&mut self, control: &mut UnixStream, pid: Pid) -> io::Result<bool> {
        loop {
            let events = {
                let mut fds = [
                    PollFd::new(control, PollFlags::IN),
                    PollFd::new(&self.completion, PollFlags::IN),
                ];
                poll_ready(&mut fds, None)?;
                fds.map(|fd| fd.revents())
            };
            let [parent, completion] = events;
            // The completion endpoint has no writer of bytes: IN/HUP means EOF.
            // Joining verifies the result, rather than interpreting EOF as success.
            if readable(completion) {
                self.join()?;
                return Ok(true);
            }
            if readable(parent) && matches!(read_control(control)?, Control::Closed) {
                // An exit observed at the cancellation decision wins coincident EOF.
                // A false result commits cancellation even if the child exits next.
                return observe(pid, true);
            }
        }
    }

    pub(super) fn wait_until(&mut self, deadline: Instant) -> io::Result<()> {
        let mut fds = [PollFd::new(&self.completion, PollFlags::IN)];
        if poll_ready(&mut fds, Some(deadline))? {
            self.join()?;
        }
        Ok(())
    }

    pub(super) fn join(&mut self) -> io::Result<()> {
        self.thread.take().map_or(Ok(()), |thread| {
            thread
                .join()
                .map_err(|_panic| io::Error::other("run exit observer panicked"))?
        })
    }
}

pub(super) fn parent_closed(control: &mut UnixStream) -> io::Result<bool> {
    loop {
        match read_control(control)? {
            Control::Closed => return Ok(true),
            Control::Open => return Ok(false),
            Control::Data => {}
        }
    }
}

enum Control {
    Closed,
    Open,
    Data,
}

fn read_control(control: &mut UnixStream) -> io::Result<Control> {
    let mut byte = [0_u8; 1];
    loop {
        match control.read(&mut byte) {
            Ok(0) => return Ok(Control::Closed),
            Ok(_) => return Ok(Control::Data),
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(Control::Open),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
}

fn readable(events: PollFlags) -> bool {
    events.intersects(PollFlags::IN | PollFlags::HUP)
}

pub(super) fn grace_until(deadline: Instant) -> io::Result<()> {
    poll_ready(&mut [], Some(deadline)).map(|_ready| ())
}

fn poll_ready(fds: &mut [PollFd<'_>], deadline: Option<Instant>) -> io::Result<bool> {
    loop {
        let now = Instant::now();
        if deadline.is_some_and(|end| now >= end) {
            return Ok(false);
        }
        let timeout = deadline
            .map(|end| Timespec::try_from(end.saturating_duration_since(now)))
            .transpose()
            .map_err(io::Error::other)?;
        match poll(fds, timeout.as_ref()) {
            Ok(0) | Err(rustix::io::Errno::INTR) => continue,
            Ok(_) => {}
            Err(error) => return Err(error.into()),
        }
        for fd in fds.iter() {
            if fd.revents().intersects(PollFlags::ERR | PollFlags::NVAL) {
                return Err(io::Error::other("run supervision socket poll failed"));
            }
        }
        return Ok(true);
    }
}
