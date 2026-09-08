use std::{
    io,
    os::unix::process::CommandExt as _,
    process::{Child, Command, ExitStatus, Output},
    sync::mpsc,
    thread::{self, JoinHandle},
    time::Instant,
};

use anyhow::Context as _;
use rustix::process::{Pid, Signal};

use super::process::{observe, require_waitable_children};

mod drain;
#[cfg(target_os = "macos")]
mod macos;

pub(super) fn output(command: &mut Command, deadline: Instant) -> anyhow::Result<Output> {
    check_deadline(deadline)?;
    require_waitable_children()?;
    command.process_group(0);
    let mut target = Target::new(command.spawn()?);
    let (send, receive) = mpsc::channel();
    let notification = Notify {
        send: send.clone(),
        event: Completion::Output,
    };
    let mut drain = drain::Drain::start(&mut target.child, deadline, notification)?;
    target.start_observer(send)?;
    let completion = wait_for_exit(&receive, &mut drain, deadline);
    // Even normal completion must terminate pipe-holding descendants. NOWAIT retains
    // the original leader until all group signals have finished, preventing ID reuse.
    let status = target.finish()?;
    let (stdout, stderr) = match completion? {
        Some(streams) => streams,
        None => drain.finish()?,
    };
    check_deadline(deadline)?;
    Ok(Output {
        status,
        stdout,
        stderr,
    })
}

pub(super) fn status(
    command: &mut Command,
    deadline: Instant,
) -> anyhow::Result<Option<ExitStatus>> {
    if Instant::now() >= deadline {
        return Ok(None);
    }
    require_waitable_children()?;
    command.process_group(0);
    let mut target = Target::new(command.spawn()?);
    let (send, receive) = mpsc::channel();
    target.start_observer(send)?;
    let completed = match receive.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
        Ok(Completion::Exit) => true,
        Ok(Completion::Output) => {
            anyhow::bail!("unexpected output notification for a status-only command")
        }
        Err(mpsc::RecvTimeoutError::Timeout) => false,
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            anyhow::bail!("command exit observer disconnected")
        }
    };
    let status = target.finish()?;
    Ok((completed && Instant::now() < deadline).then_some(status))
}

fn wait_for_exit(
    receive: &mpsc::Receiver<Completion>,
    drain: &mut drain::Drain,
    deadline: Instant,
) -> anyhow::Result<Option<drain::Streams>> {
    let mut streams = None;
    loop {
        check_deadline(deadline)?;
        match receive.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Ok(Completion::Exit) => return Ok(streams),
            Ok(Completion::Output) => streams = Some(drain.finish()?),
            Err(mpsc::RecvTimeoutError::Timeout) => anyhow::bail!("command deadline expired"),
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                anyhow::bail!("command observers disconnected")
            }
        }
    }
}

#[derive(Clone, Copy)]
enum Completion {
    Exit,
    Output,
}

struct Notify {
    send: mpsc::Sender<Completion>,
    event: Completion,
}

impl Drop for Notify {
    fn drop(&mut self) {
        // Notify on errors and unwinding too. The receiving thread joins for the result;
        // it may already have closed the channel while unwinding its own error path.
        let _notification_result = self.send.send(self.event);
    }
}

fn check_deadline(deadline: Instant) -> anyhow::Result<()> {
    if Instant::now() >= deadline {
        anyhow::bail!("command deadline expired");
    }
    Ok(())
}

struct Target {
    child: Child,
    observer: Option<JoinHandle<io::Result<()>>>,
    signalable: bool,
    reaped: bool,
}

impl Target {
    const fn new(child: Child) -> Self {
        Self {
            child,
            observer: None,
            signalable: true,
            reaped: false,
        }
    }

    fn start_observer(&mut self, send: mpsc::Sender<Completion>) -> io::Result<()> {
        let pid = Pid::from_child(&self.child);
        let notification = Notify {
            send,
            event: Completion::Exit,
        };
        self.observer = Some(thread::Builder::new().name("command-exit".into()).spawn(
            move || {
                let _notification = notification;
                observe(pid, false).map(|_exited| ())
            },
        )?);
        Ok(())
    }

    fn finish(&mut self) -> anyhow::Result<ExitStatus> {
        let signaling = self.terminate();
        if let Err(error) = &signaling {
            tracing::error!(%error, "cannot terminate captured command; retaining its wait and observer until exit");
        }
        // Terminate before joining, including setup failures and unwinding. The observer
        // never consumes status. No consuming wait may run concurrently with it.
        let observed = self.observer.take().map_or(Ok(()), |observer| {
            observer
                .join()
                .map_err(|_panic| anyhow::anyhow!("command exit observer panicked"))?
                .context("could not observe command exit")
        });
        let status = self.child.wait();
        self.reaped = match &status {
            Ok(_status) => true,
            Err(error) => error.raw_os_error() == Some(libc::ECHILD),
        };
        signaling?;
        observed?;
        status.context("could not reap captured command")
    }

    fn terminate(&mut self) -> io::Result<()> {
        if !self.signalable {
            return Ok(());
        }
        // Close signaling permanently before any path can reach a consuming wait.
        self.signalable = false;
        let pid = Pid::from_child(&self.child);
        observe(pid, true)?;
        let group = absent_is_ok(rustix::process::kill_process_group(pid, Signal::KILL));
        #[cfg(target_os = "macos")]
        let group = macos::check_group_signal(pid, group);
        // A child can leave its original group. Never follow it into another group;
        // the retained direct-child identity remains safe to signal individually.
        let direct = absent_is_ok(rustix::process::kill_process(pid, Signal::KILL));
        group.and(direct)
    }
}

impl Drop for Target {
    fn drop(&mut self) {
        if !self.reaped
            && let Err(error) = self.finish()
        {
            tracing::error!(%error, "captured command cleanup failed");
        }
    }
}

fn absent_is_ok(result: rustix::io::Result<()>) -> io::Result<()> {
    match result {
        Ok(()) | Err(rustix::io::Errno::SRCH) => Ok(()),
        Err(error) => Err(error.into()),
    }
}

#[cfg(test)]
#[path = "captured_tests.rs"]
mod tests;
