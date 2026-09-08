use std::{
    io,
    os::unix::net::UnixStream,
    process::{Child, ExitStatus},
    time::Instant,
};

use rustix::process::{Pid, Signal};

use super::{GRACE, cleanup, wait};
use crate::command::observe_child_exit as observe;

pub(super) struct TargetGuard {
    child: Child,
    observer: Option<wait::Observer>,
    signalable: bool,
    settled: bool,
}

impl TargetGuard {
    pub(super) const fn new(child: Child) -> Self {
        Self {
            child,
            observer: None,
            signalable: true,
            settled: false,
        }
    }

    pub(super) fn start_observer(&mut self) -> io::Result<()> {
        if self.observer.is_some() || !self.signalable {
            return Err(io::Error::other("run exit observer cannot be restarted"));
        }
        self.observer = Some(wait::Observer::start(Pid::from_child(&self.child))?);
        Ok(())
    }

    pub(super) fn wait(&mut self, control: &mut UnixStream) -> io::Result<bool> {
        let pid = Pid::from_child(&self.child);
        let result = self
            .observer
            .as_mut()
            .ok_or_else(|| io::Error::other("run exit observer is missing"))?
            .wait(control, pid);
        if result
            .as_ref()
            .is_err_and(|error| error.raw_os_error() == Some(libc::ECHILD))
        {
            self.signalable = false;
        }
        result
    }

    pub(super) fn finish(&mut self, cancelled: bool) -> io::Result<ExitStatus> {
        let preparation = self.graceful(cancelled);
        self.complete(preparation)
    }

    fn graceful(&mut self, cancelled: bool) -> io::Result<bool> {
        if !self.signalable {
            return Err(io::Error::other("run numeric signaling is already closed"));
        }
        let pid = Pid::from_child(&self.child);
        let exited = observe(pid, true)?;
        if exited && cleanup::leader_is_alone(pid)? {
            return Ok(true);
        }
        cleanup::signal_group(pid, Signal::TERM)?;
        if cancelled && !exited {
            // The direct child may have left the original group. Do not follow
            // it into another group, which could include unrelated processes.
            cleanup::signal_child(pid, Signal::TERM)?;
        }
        let deadline = Instant::now()
            .checked_add(GRACE)
            .ok_or_else(|| io::Error::other("supervisor grace period exceeds Instant range"))?;
        if cancelled {
            if !exited {
                self.observer
                    .as_mut()
                    .ok_or_else(|| io::Error::other("run exit observer is missing"))?
                    .wait_until(deadline)?;
            }
        } else {
            wait::grace_until(deadline)?;
        }
        Ok(false)
    }

    fn complete(&mut self, preparation: io::Result<bool>) -> io::Result<ExitStatus> {
        if let Err(error) = &preparation {
            if error.raw_os_error() == Some(libc::ECHILD) {
                self.signalable = false;
            }
            eprintln!("error: run group grace cleanup failed: {error}");
        }
        let skip_group = preparation.as_ref().is_ok_and(|alone| *alone);
        let forced = self.kill_and_close(skip_group);
        if let Err(error) = &forced {
            eprintln!("error: run final signaling failed: {error}");
        }
        // Error cleanup must kill before a potentially blocking join. The lease
        // remains in run's scope throughout joining, reaping, and adoption cleanup.
        let status = self.settle();
        if let Err(error) = &status {
            eprintln!("error: run child cleanup failed: {error}");
        }
        preparation?;
        forced?;
        status
    }

    fn kill_and_close(&mut self, skip_group: bool) -> io::Result<()> {
        if !self.signalable {
            return Ok(());
        }
        // This is the final signal batch. Close guard authority BEFORE anything
        // can fail; neither consuming waits nor Drop can reopen numeric signaling.
        self.signalable = false;
        let pid = Pid::from_child(&self.child);
        let exited = observe(pid, true).inspect_err(|error| {
            if error.raw_os_error() != Some(libc::ECHILD) {
                eprintln!(
                    "error: cannot verify direct run child {} for termination; retaining run lease and supervision until it exits: {error}",
                    pid.as_raw_pid()
                );
            }
        })?;
        if skip_group && exited {
            return Ok(());
        }
        let group = cleanup::signal_group(pid, Signal::KILL);
        if group
            .as_ref()
            .is_err_and(|error| error.raw_os_error() == Some(libc::ECHILD))
        {
            return group;
        }
        // A failed group signal must not prevent exact-child termination. The
        // retained direct-child identity is safe even if its original group is gone.
        let direct = if exited {
            Ok(())
        } else {
            cleanup::signal_child(pid, Signal::KILL)
        };
        if let Err(error) = &direct {
            eprintln!(
                "error: cannot terminate direct run child {}; retaining run lease and supervision until it exits: {error}",
                pid.as_raw_pid()
            );
        }
        group.and(direct)
    }

    fn settle(&mut self) -> io::Result<ExitStatus> {
        let observed = self.observer.as_mut().map_or(Ok(()), wait::Observer::join);
        if let Err(error) = &observed {
            eprintln!("error: could not observe direct run child exit: {error}");
        }
        let status = self.child.wait();
        if let Err(error) = &status {
            eprintln!("error: could not reap direct run child: {error}");
        }
        // An unsuccessful wait must not hand a still-pinned leader to wait(-1).
        let adopted = if status.is_ok()
            || status
                .as_ref()
                .is_err_and(|error| error.raw_os_error() == Some(libc::ECHILD))
        {
            cleanup::cleanup_adopted_children()
        } else {
            Ok(())
        };
        if let Err(error) = &adopted {
            eprintln!("error: run adopted-child cleanup failed: {error}");
        }
        // Keep the existing best-effort Drop retry after adoption cleanup fails.
        // That retry cannot reopen group signaling, and uses the same bounded loop.
        // Only waiting for the direct child may hold supervision indefinitely.
        self.settled = adopted.is_ok();
        observed?;
        let status = status?;
        adopted?;
        Ok(status)
    }
}

impl Drop for TargetGuard {
    fn drop(&mut self) {
        if !self.settled
            && let Err(error) = self.complete(Ok(false))
        {
            eprintln!("error: run supervisor fallback cleanup failed: {error}");
        }
    }
}
