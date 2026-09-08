use std::io;

use anyhow::Context as _;
use rustix::process::{Pid, WaitId, WaitIdOptions, waitid};

#[cfg(any(target_os = "macos", test))]
mod macos;
#[cfg(target_os = "macos")]
pub use macos::leader_is_alone;

// Callers retain the direct child and exclude consuming waiters until every
// numeric process-group signal has finished. NOWAIT alone cannot enforce that.
pub fn observe(pid: Pid, nonblocking: bool) -> io::Result<bool> {
    let mut options = WaitIdOptions::EXITED | WaitIdOptions::NOWAIT;
    if nonblocking {
        options |= WaitIdOptions::NOHANG;
    }
    loop {
        match waitid(WaitId::Pid(pid), options) {
            Ok(status) => return Ok(status.is_some()),
            Err(rustix::io::Errno::INTR) => {}
            Err(error) => return Err(error.into()),
        }
    }
}

#[expect(
    unsafe_code,
    reason = "query the inherited SIGCHLD policy without changing process-global signal state"
)]
pub fn require_waitable_children() -> anyhow::Result<()> {
    let mut action = std::mem::MaybeUninit::<libc::sigaction>::uninit();
    // SAFETY: action points to writable sigaction storage; a null new action only queries.
    if unsafe { libc::sigaction(libc::SIGCHLD, std::ptr::null(), action.as_mut_ptr()) } != 0 {
        return Err(io::Error::last_os_error()).context("cannot check child waitability");
    }
    // SAFETY: a successful sigaction query initialized the complete output value.
    let action = unsafe { action.assume_init() };
    // Custom handlers can reap as well. Callers must keep the default policy
    // while an owned child remains subject to non-reaping observation.
    if action.sa_sigaction != libc::SIG_DFL || action.sa_flags & libc::SA_NOCLDWAIT != 0 {
        anyhow::bail!("supervised commands require default SIGCHLD without SA_NOCLDWAIT");
    }
    Ok(())
}
