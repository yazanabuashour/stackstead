use std::{
    io::{self, Write as _},
    os::unix::net::UnixStream,
    process::{Child, Command, ExitStatus, Stdio},
};

use rustix::process::{Pid, getpid};

use super::Observer;
use crate::{
    command::observe_child_exit as observe,
    test_support::{TestResultErrorExt as _, TestResultExt as _},
};

#[test]
fn exited_before_registration_beats_pending_eof_and_stays_latched() -> anyhow::Result<()> {
    let mut target = DirectChild::spawn()?;
    let pid = Pid::from_child(&target.child);
    target.release()?;
    assert!(observe(pid, false).test()?);

    let (mut control, parent) = UnixStream::pair().test()?;
    drop(parent);
    target.observer = Some(Observer::start(pid).test()?);
    assert!(
        target
            .observer
            .as_mut()
            .test()?
            .wait(&mut control, pid)
            .test()?
    );
    target.observer.as_mut().test()?.join().test()?;

    // An open control peer leaves only the latched observer completion ready.
    let (mut control, _parent) = UnixStream::pair().test()?;
    assert!(
        target
            .observer
            .as_mut()
            .test()?
            .wait(&mut control, pid)
            .test()?
    );
    assert!(observe(pid, true).test()?);
    assert_eq!(target.reap().test()?.code(), Some(23));
    Ok(())
}

#[test]
fn control_eof_commits_cancellation_before_the_direct_child_exits() -> anyhow::Result<()> {
    let mut target = DirectChild::spawn()?;
    let pid = Pid::from_child(&target.child);
    target.observer = Some(Observer::start(pid).test()?);
    let (mut control, parent) = UnixStream::pair().test()?;
    drop(parent);

    let observed_exit = target
        .observer
        .as_mut()
        .test()?
        .wait(&mut control, pid)
        .test()?;
    assert!(!observe(pid, true).test()?);
    target.release()?;
    target.observer.as_mut().test()?.join().test()?;

    // The caller retains the cancellation decision even after exit is observable.
    assert!(!observed_exit);
    assert!(observe(pid, true).test()?);
    assert_eq!(target.reap().test()?.code(), Some(23));
    Ok(())
}

#[test]
fn observer_failure_is_not_successful_completion() -> anyhow::Result<()> {
    // This process is not its own child. Keep control open so wait must join
    // the failed observer rather than decide from a control EOF probe.
    let (mut control, _parent) = UnixStream::pair().test()?;
    let pid = getpid();
    let mut observer = Observer::start(pid).test()?;
    let result = observer.wait(&mut control, pid);
    let joined = observer.join();
    let error = result.test_err()?;
    joined.test()?;
    assert_eq!(error.raw_os_error(), Some(libc::ECHILD));
    Ok(())
}

// Never use supervisor TargetGuard here: its broad adoption reap can consume
// children belonging to other tests in this process.
struct DirectChild {
    child: Child,
    observer: Option<Observer>,
    reaped: bool,
}

impl DirectChild {
    fn spawn() -> anyhow::Result<Self> {
        let child = Command::new("sh")
            .args(["-c", "read token; exit 23"])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .test()?;
        Ok(Self {
            child,
            observer: None,
            reaped: false,
        })
    }

    fn release(&mut self) -> anyhow::Result<()> {
        self.child
            .stdin
            .as_mut()
            .test()?
            .write_all(b"continue\n")
            .test()
    }

    fn reap(&mut self) -> io::Result<ExitStatus> {
        if let Some(observer) = &mut self.observer {
            observer.join()?;
        }
        let status = self.child.wait();
        self.reaped = status.is_ok()
            || status
                .as_ref()
                .is_err_and(|error| error.raw_os_error() == Some(libc::ECHILD));
        status
    }
}

impl Drop for DirectChild {
    fn drop(&mut self) {
        if self.reaped {
            return;
        }
        // EOF also releases the shell if termination fails. No external command
        // or descendant can retain this barrier. Only a still-owned child may
        // receive a signal, and no consuming wait may precede the observer join.
        drop(self.child.stdin.take());
        match observe(Pid::from_child(&self.child), true) {
            Ok(false) => {
                if let Err(error) = self.child.kill() {
                    eprintln!("test direct-child termination failed: {error}");
                }
            }
            Ok(true) => {}
            Err(error) => eprintln!("test direct-child ownership check failed: {error}"),
        }
        if let Some(observer) = &mut self.observer
            && let Err(error) = observer.join()
        {
            eprintln!("test observer cleanup failed: {error}");
        }
        if let Err(error) = self.child.wait() {
            eprintln!("test direct-child reap failed: {error}");
        }
    }
}
