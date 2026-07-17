#[cfg(unix)]
use std::{
    ffi::OsString,
    io::Read,
    os::{fd::FromRawFd, unix::process::CommandExt},
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
    validate_lease(lease_fd, lease_dev, lease_ino)?;
    set_cloexec(lease_fd, true)?;
    set_cloexec(control_fd, true)?;
    #[cfg(target_os = "linux")]
    set_subreaper()?;
    let mut control = unsafe { std::os::unix::net::UnixStream::from_raw_fd(control_fd) };
    control.set_nonblocking(true)?;

    let mut command = Command::new(program);
    command.args(args).process_group(0);
    let mut child = command.spawn()?;
    let group = match i32::try_from(child.id()) {
        Ok(group) => group,
        Err(_) => {
            let _ = child.kill();
            let _ = child.wait();
            anyhow::bail!("target PID exceeds the Unix process ID range");
        }
    };
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
    group: i32,
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
    fd: i32,
    expected_dev: libc::dev_t,
    expected_ino: libc::ino_t,
) -> anyhow::Result<()> {
    use std::mem::MaybeUninit;

    let mut metadata = MaybeUninit::<libc::stat>::zeroed();
    if unsafe { libc::fstat(fd, metadata.as_mut_ptr()) } != 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    let metadata = unsafe { metadata.assume_init() };
    if metadata.st_dev != expected_dev || metadata.st_ino != expected_ino {
        anyhow::bail!("private run lease identity changed during handoff");
    }
    Ok(())
}

#[cfg(unix)]
pub(crate) fn set_cloexec(fd: i32, enabled: bool) -> std::io::Result<()> {
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
    if flags < 0 {
        return Err(std::io::Error::last_os_error());
    }
    let flags = if enabled {
        flags | libc::FD_CLOEXEC
    } else {
        flags & !libc::FD_CLOEXEC
    };
    if unsafe { libc::fcntl(fd, libc::F_SETFD, flags) } < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(unix)]
fn signal_group(group: i32, signal: i32) -> std::io::Result<()> {
    if unsafe { libc::kill(-group, signal) } == 0 {
        return Ok(());
    }
    let error = std::io::Error::last_os_error();
    if error.raw_os_error() == Some(libc::ESRCH) {
        Ok(())
    } else {
        Err(error)
    }
}

#[cfg(unix)]
fn cancel_target(child: &mut std::process::Child, group: i32) -> std::io::Result<()> {
    signal_group(group, libc::SIGTERM)?;
    let deadline = Instant::now() + GRACE;
    while Instant::now() < deadline {
        match child.try_wait() {
            Ok(Some(_)) => {
                signal_group(group, libc::SIGKILL)?;
                return Ok(());
            }
            Ok(None) => {}
            Err(_) => break,
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    signal_group(group, libc::SIGKILL)?;
    child.wait().map(|_| ())
}

#[cfg(unix)]
fn cleanup_group(group: i32) -> std::io::Result<()> {
    if unsafe { libc::kill(-group, 0) } != 0 {
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() == Some(libc::ESRCH) {
            return Ok(());
        }
        return Err(error);
    }
    signal_group(group, libc::SIGTERM)?;
    std::thread::sleep(GRACE);
    signal_group(group, libc::SIGKILL)
}

#[cfg(target_os = "linux")]
fn set_subreaper() -> std::io::Result<()> {
    if unsafe { libc::prctl(libc::PR_SET_CHILD_SUBREAPER, 1, 0, 0, 0) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
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
            if unsafe { libc::kill(child, libc::SIGKILL) } != 0 {
                let error = std::io::Error::last_os_error();
                if error.raw_os_error() != Some(libc::ESRCH) {
                    return Err(error);
                }
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
        let result = unsafe { libc::waitpid(-1, std::ptr::null_mut(), libc::WNOHANG) };
        if result > 0 {
            continue;
        }
        if result == 0 {
            return Ok(());
        }
        let error = std::io::Error::last_os_error();
        return if error.raw_os_error() == Some(libc::ECHILD) {
            Ok(())
        } else {
            Err(error)
        };
    }
}
