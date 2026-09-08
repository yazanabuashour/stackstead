use std::io;

use rustix::process::{Pid, Signal};

pub(super) fn signal_group(group: Pid, signal: Signal) -> io::Result<()> {
    let result = absent_is_ok(rustix::process::kill_process_group(group, signal));
    #[cfg(target_os = "macos")]
    if let Err(error) = &result {
        // XNU can return EPERM for a zombie-only group. Only a fresh exit and
        // proved lone leader excuse it; query errors never certify absence.
        if error.raw_os_error() == Some(libc::EPERM)
            && crate::command::observe_child_exit(group, true)?
            && crate::command::leader_is_alone(group)?
        {
            return Ok(());
        }
    }
    result
}

pub(super) fn signal_child(pid: Pid, signal: Signal) -> io::Result<()> {
    absent_is_ok(rustix::process::kill_process(pid, signal))
}

fn absent_is_ok(result: rustix::io::Result<()>) -> io::Result<()> {
    match result {
        Ok(()) | Err(rustix::io::Errno::SRCH) => Ok(()),
        Err(error) => Err(error.into()),
    }
}

#[cfg(target_os = "linux")]
pub(super) fn leader_is_alone(leader: Pid) -> io::Result<bool> {
    // Main spawned the target and is the sole reaper. After its observed exit,
    // subreaper adoption exposes every remaining descendant tree here.
    Ok(children()?.into_iter().all(|child| child == leader))
}

#[cfg(target_os = "macos")]
pub(super) fn leader_is_alone(leader: Pid) -> io::Result<bool> {
    crate::command::leader_is_alone(leader)
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub(super) const fn leader_is_alone(_leader: Pid) -> io::Result<bool> {
    Ok(false)
}

#[cfg(target_os = "linux")]
pub(super) fn set_subreaper() -> io::Result<()> {
    rustix::process::set_child_subreaper(Some(Pid::INIT)).map_err(Into::into)
}

#[cfg(target_os = "linux")]
fn children() -> io::Result<Vec<Pid>> {
    let path = format!("/proc/self/task/{}/children", std::process::id());
    std::fs::read_to_string(path)?
        .split_whitespace()
        .map(|child| {
            let raw = child.parse::<i32>().map_err(io::Error::other)?;
            if raw <= 0 {
                return Err(io::Error::other("adopted child PID is not positive"));
            }
            Pid::from_raw(raw).ok_or_else(|| io::Error::other("adopted child PID is zero"))
        })
        .collect()
}

#[cfg(target_os = "linux")]
pub(super) fn cleanup_adopted_children() -> io::Result<()> {
    // This broad reap is allowed only after the direct child's consuming wait.
    // Keep the existing retry policy, not an indefinite descendant lease hold.
    for _ in 0..25 {
        reap_children()?;
        let children = children()?;
        if children.is_empty() {
            return Ok(());
        }
        // No concurrent reaper can release these owned child identities between
        // the snapshot and signals. Refresh the snapshot after every reap pass.
        for child in children {
            signal_child(child, Signal::KILL)?;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    reap_children()?;
    if children()?.is_empty() {
        Ok(())
    } else {
        Err(io::Error::other(
            "could not reap all descendants of the interrupted run",
        ))
    }
}

#[cfg(not(target_os = "linux"))]
pub(super) const fn cleanup_adopted_children() -> io::Result<()> {
    Ok(())
}

#[cfg(target_os = "linux")]
fn reap_children() -> io::Result<()> {
    loop {
        match rustix::process::wait(rustix::process::WaitOptions::NOHANG) {
            Ok(Some(_)) => {}
            Ok(None) | Err(rustix::io::Errno::CHILD) => return Ok(()),
            Err(error) => return Err(error.into()),
        }
    }
}
