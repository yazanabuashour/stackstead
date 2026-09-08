use std::io;

use rustix::process::Pid;

use crate::command::process::leader_is_alone;

pub(super) fn check_group_signal(pid: Pid, result: io::Result<()>) -> io::Result<()> {
    let Err(error) = result else {
        return Ok(());
    };
    // XNU's explicit-group kill filters zombies, then UNIX03 returns EPERM if
    // nobody was signalable. Recheck exit AFTER the error: the leader may have
    // exited since terminate's first NOWAIT check. Never excuse other members.
    if error.raw_os_error() == Some(libc::EPERM)
        && super::observe(pid, true)?
        && leader_is_alone(pid)?
    {
        return Ok(());
    }
    Err(error)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{TestResultErrorExt as _, TestResultExt as _};

    #[test]
    fn native_lone_leader_requires_exit_and_stays_waitable_through_signaling() -> anyhow::Result<()>
    {
        use std::{
            io::Write as _,
            os::unix::process::CommandExt as _,
            process::{Command, Stdio},
        };

        use super::super::{Target, observe, require_waitable_children};

        require_waitable_children().test()?;
        let child = Command::new("sh")
            .args(["-c", "read token; exit 23"])
            .process_group(0)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .test()?;
        let mut target = Target::new(child);
        let pid = Pid::from_child(&target.child);
        assert!(!observe(pid, true).test()?);
        let error =
            check_group_signal(pid, Err(io::Error::from_raw_os_error(libc::EPERM))).test_err()?;
        assert_eq!(error.raw_os_error(), Some(libc::EPERM));
        target
            .child
            .stdin
            .as_mut()
            .test()?
            .write_all(b"exit\n")
            .test()?;
        assert!(observe(pid, false).test()?);
        assert!(leader_is_alone(pid).test()?);
        check_group_signal(pid, Err(io::Error::from_raw_os_error(libc::EPERM))).test()?;
        let error =
            check_group_signal(pid, Err(io::Error::from_raw_os_error(libc::EACCES))).test_err()?;
        assert_eq!(error.raw_os_error(), Some(libc::EACCES));
        target.terminate().test()?;
        assert!(observe(pid, true).test()?);
        assert_eq!(target.finish().test()?.code(), Some(23));
        assert_eq!(
            observe(pid, true).test_err()?.raw_os_error(),
            Some(libc::ECHILD)
        );
        Ok(())
    }
}
