#[cfg(unix)]
use std::{
    ffi::OsString,
    io::Read,
    os::{fd::AsFd, unix::process::CommandExt},
    process::Command,
    time::{Duration, Instant},
};

#[cfg(unix)]
pub(crate) const ARGUMENT: &str = "__stackstead_run_supervisor_v1";
#[cfg(unix)]
const GRACE: Duration = Duration::from_millis(500);

#[cfg(unix)]
pub(crate) fn run_if_requested() -> Option<i32> {
    (std::env::args_os().nth(1).as_deref() == Some(std::ffi::OsStr::new(ARGUMENT))).then(|| {
        run().unwrap_or_else(|error| {
            eprintln!("error: private run supervisor failed: {error:#}");
            1
        })
    })
}

#[cfg(not(unix))]
pub(crate) fn run_if_requested() -> Option<i32> {
    None
}

#[cfg(unix)]
fn run() -> anyhow::Result<i32> {
    let mut arguments = std::env::args_os().skip(2);
    let control_fd = argument_number::<i32>(&mut arguments, "control descriptor")?;
    let lease_fd = argument_number::<i32>(&mut arguments, "lease descriptor")?;
    let lease_dev = argument_number::<libc::dev_t>(&mut arguments, "lease device")?;
    let lease_ino = argument_number::<libc::ino_t>(&mut arguments, "lease inode")?;
    if control_fd < 3 || lease_fd < 3 || control_fd == lease_fd {
        anyhow::bail!("private descriptors are invalid");
    }
    if arguments.next().as_deref() != Some(std::ffi::OsStr::new("--")) {
        anyhow::bail!("private supervisor argument boundary is missing");
    }
    let program = arguments
        .next()
        .ok_or_else(|| anyhow::anyhow!("private supervisor target is missing"))?;
    let args = arguments.collect::<Vec<OsString>>();
    let control_fd = take_inherited_fd(control_fd)?;
    let lease_fd = take_inherited_fd(lease_fd)?;
    validate_lease(&lease_fd, lease_dev, lease_ino)?;
    set_cloexec(&lease_fd, true)?;
    set_cloexec(&control_fd, true)?;
    #[cfg(target_os = "linux")]
    set_subreaper()?;
    let mut control = std::os::unix::net::UnixStream::from(control_fd);
    control.set_nonblocking(true)?;

    let mut command = Command::new(program);
    command.args(args).process_group(0);
    let child = command.spawn()?;
    let group = rustix::process::Pid::from_child(&child);
    let mut target = TargetGuard {
        child,
        group,
        armed: true,
    };

    loop {
        if let Some(status) = target.child.try_wait()? {
            target.finish()?;
            return Ok(crate::agent::exit_code(status));
        }
        let mut byte = [0_u8; 1];
        match control.read(&mut byte) {
            Ok(0) => {
                target.cancel()?;
                return Ok(143);
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error.into()),
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

#[cfg(unix)]
fn argument_number<T>(
    arguments: &mut impl Iterator<Item = OsString>,
    label: &str,
) -> anyhow::Result<T>
where
    T: std::str::FromStr,
    T::Err: std::error::Error + Send + Sync + 'static,
{
    arguments
        .next()
        .ok_or_else(|| anyhow::anyhow!("private supervisor {label} is missing"))?
        .into_string()
        .map_err(|_| anyhow::anyhow!("private supervisor {label} is not UTF-8"))?
        .parse()
        .map_err(Into::into)
}

#[cfg(unix)]
struct TargetGuard {
    child: std::process::Child,
    group: rustix::process::Pid,
    armed: bool,
}

#[cfg(unix)]
impl TargetGuard {
    fn cancel(&mut self) -> std::io::Result<()> {
        cancel_target(&mut self.child, self.group)?;
        cleanup_adopted_children()?;
        self.armed = false;
        Ok(())
    }

    fn finish(&mut self) -> std::io::Result<()> {
        cleanup_group(self.group)?;
        cleanup_adopted_children()?;
        self.armed = false;
        Ok(())
    }
}

#[cfg(unix)]
impl Drop for TargetGuard {
    fn drop(&mut self) {
        if self.armed {
            let _ = cancel_target(&mut self.child, self.group);
            let _ = cleanup_adopted_children();
        }
    }
}

#[cfg(unix)]
fn validate_lease(
    fd: &impl AsFd,
    expected_dev: libc::dev_t,
    expected_ino: libc::ino_t,
) -> anyhow::Result<()> {
    let metadata = rustix::fs::fstat(fd)?;
    if metadata.st_dev != expected_dev || metadata.st_ino != expected_ino {
        anyhow::bail!("private run lease identity changed during handoff");
    }
    Ok(())
}

#[cfg(unix)]
pub(crate) fn set_cloexec(fd: &impl AsFd, enabled: bool) -> std::io::Result<()> {
    let mut flags = rustix::io::fcntl_getfd(fd)?;
    if enabled {
        flags.insert(rustix::io::FdFlags::CLOEXEC);
    } else {
        flags.remove(rustix::io::FdFlags::CLOEXEC);
    }
    rustix::io::fcntl_setfd(fd, flags).map_err(Into::into)
}

#[cfg(unix)]
#[expect(
    unsafe_code,
    reason = "inherited raw descriptors cross exec without a Rust owner"
)]
fn take_inherited_fd(raw_fd: std::os::fd::RawFd) -> std::io::Result<std::os::fd::OwnedFd> {
    use std::os::fd::FromRawFd;

    // SAFETY: F_GETFD only passes the integer descriptor to the kernel and
    // does not dereference a caller-provided pointer. It is used here to prove
    // that the untrusted private argument names an open descriptor before the
    // descriptor is adopted.
    if unsafe { libc::fcntl(raw_fd, libc::F_GETFD) } < 0 {
        return Err(std::io::Error::last_os_error());
    }

    // SAFETY: The successful F_GETFD above proves the descriptor is open.
    // Private supervisor descriptors are distinct, at least 3, inherited
    // across exec, and adopted during single-threaded process startup, so no
    // other Rust value owns or can concurrently close this descriptor.
    Ok(unsafe { std::os::fd::OwnedFd::from_raw_fd(raw_fd) })
}

#[cfg(unix)]
fn signal_group(
    group: rustix::process::Pid,
    signal: rustix::process::Signal,
) -> std::io::Result<()> {
    match rustix::process::kill_process_group(group, signal) {
        Ok(()) | Err(rustix::io::Errno::SRCH) => Ok(()),
        Err(error) => Err(error.into()),
    }
}

#[cfg(unix)]
fn cancel_target(
    child: &mut std::process::Child,
    group: rustix::process::Pid,
) -> std::io::Result<()> {
    signal_group(group, rustix::process::Signal::TERM)?;
    let deadline = Instant::now() + GRACE;
    while Instant::now() < deadline {
        match child.try_wait() {
            Ok(Some(_)) => {
                signal_group(group, rustix::process::Signal::KILL)?;
                return Ok(());
            }
            Ok(None) => {}
            Err(_) => break,
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    signal_group(group, rustix::process::Signal::KILL)?;
    child.wait().map(|_| ())
}

#[cfg(unix)]
fn cleanup_group(group: rustix::process::Pid) -> std::io::Result<()> {
    match rustix::process::kill_process_group(group, rustix::process::Signal::TERM) {
        Ok(()) => {}
        Err(rustix::io::Errno::SRCH) => return Ok(()),
        Err(error) => return Err(error.into()),
    }
    std::thread::sleep(GRACE);
    signal_group(group, rustix::process::Signal::KILL)
}

#[cfg(target_os = "linux")]
fn set_subreaper() -> std::io::Result<()> {
    rustix::process::set_child_subreaper(Some(rustix::process::Pid::INIT)).map_err(Into::into)
}

#[cfg(target_os = "linux")]
fn cleanup_adopted_children() -> std::io::Result<()> {
    let path = format!("/proc/self/task/{}/children", std::process::id());
    for _ in 0..25 {
        reap_children()?;
        let children = std::fs::read_to_string(&path)?
            .split_whitespace()
            .map(str::parse::<i32>)
            .collect::<Result<Vec<_>, _>>()
            .map_err(std::io::Error::other)?;
        if children.is_empty() {
            return Ok(());
        }
        for child in children {
            let child = rustix::process::Pid::from_raw(child)
                .ok_or_else(|| std::io::Error::other("adopted child PID is zero"))?;
            match rustix::process::kill_process(child, rustix::process::Signal::KILL) {
                Ok(()) | Err(rustix::io::Errno::SRCH) => {}
                Err(error) => return Err(error.into()),
            }
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    reap_children()?;
    if std::fs::read_to_string(path)?
        .split_whitespace()
        .next()
        .is_some()
    {
        Err(std::io::Error::other(
            "could not reap all descendants of the interrupted run",
        ))
    } else {
        Ok(())
    }
}

#[cfg(not(target_os = "linux"))]
fn cleanup_adopted_children() -> std::io::Result<()> {
    Ok(())
}

#[cfg(target_os = "linux")]
fn reap_children() -> std::io::Result<()> {
    loop {
        match rustix::process::wait(rustix::process::WaitOptions::NOHANG) {
            Ok(Some(_)) => continue,
            Ok(None) | Err(rustix::io::Errno::CHILD) => return Ok(()),
            Err(error) => return Err(error.into()),
        }
    }
}
