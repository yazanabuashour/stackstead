#[cfg(unix)]
use std::{
    ffi::OsString,
    os::{fd::AsFd, unix::process::CommandExt as _},
    process::Command,
    time::Duration,
};

#[cfg(unix)]
mod cleanup;
#[cfg(unix)]
mod target;
#[cfg(unix)]
mod wait;

#[cfg(unix)]
pub const ARGUMENT: &str = "__stackstead_run_supervisor_v1";
#[cfg(unix)]
const GRACE: Duration = Duration::from_millis(500);

#[cfg(unix)]
pub fn run_if_requested() -> Option<i32> {
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
    crate::command::require_waitable_children()?;
    #[cfg(target_os = "linux")]
    cleanup::set_subreaper()?;
    let mut control = std::os::unix::net::UnixStream::from(control_fd);
    control.set_nonblocking(true)?;
    if wait::parent_closed(&mut control)? {
        return Ok(143);
    }

    let mut command = Command::new(program);
    command.args(args).process_group(0);
    // Guard the child before the completion pair or observer thread can fail.
    let mut target = target::TargetGuard::new(command.spawn()?);
    target.start_observer()?;
    let cancelled = !target.wait(&mut control)?;
    let status = target.finish(cancelled)?;
    if cancelled {
        Ok(143)
    } else {
        Ok(crate::agent::exit_code(status))
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
        .map_err(|value| {
            anyhow::anyhow!(
                "private supervisor {label} is not UTF-8: {}",
                value.to_string_lossy()
            )
        })?
        .parse()
        .map_err(Into::into)
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
pub fn set_cloexec(fd: &impl AsFd, enabled: bool) -> std::io::Result<()> {
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
