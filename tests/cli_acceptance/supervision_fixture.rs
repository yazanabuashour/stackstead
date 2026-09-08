use super::{Path, PathBuf, ProcessCommand, TestResultErrorExt, TestResultExt, fs};
use fs2::FileExt as _;
use std::{
    io,
    os::{fd::RawFd, unix::process::CommandExt as _},
    process::{Child, ChildStdin, ExitStatus, Stdio},
    thread,
    time::Duration,
};

// Receipt: run.rs already uses 100 attempts at 20 ms for process readiness/cleanup.
const ATTEMPTS: usize = 100;
const RETRY_DELAY: Duration = Duration::from_millis(20);

pub(super) struct ChildGuard {
    child: Option<Child>,
    input: Option<ChildStdin>,
}

impl ChildGuard {
    pub(super) fn spawn(command: &mut ProcessCommand) -> anyhow::Result<Self> {
        let mut child = command
            .stdin(Stdio::piped())
            .spawn()
            .test_context("spawn supervisor fixture")?;
        let input = child.stdin.take();
        Ok(Self {
            child: Some(child),
            input,
        })
    }

    #[cfg(target_os = "linux")]
    pub(super) fn release_input(&mut self) {
        drop(self.input.take());
    }

    pub(super) fn signal(&self, signal: rustix::process::Signal) -> anyhow::Result<()> {
        let child = self
            .child
            .as_ref()
            .test_context("child is still owned and unreaped")?;
        rustix::process::kill_process(rustix::process::Pid::from_child(child), signal)
            .test_context("signal owned fixture child")
    }

    pub(super) fn wait(&mut self) -> anyhow::Result<ExitStatus> {
        for _ in 0..ATTEMPTS {
            if let Some(status) = self.child.as_mut().test()?.try_wait().test()? {
                // Keep stdin open after wrapper reap: only control EOF may cancel the target.
                drop(self.child.take());
                return Ok(status);
            }
            thread::sleep(RETRY_DELAY);
        }
        anyhow::bail!("fixture child did not finish within the existing acceptance tripwire")
    }
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        // Both fixture shells can finish on stdin EOF even if supervision fails.
        drop(self.input.take());
        if let Some(mut child) = self.child.take() {
            if let Err(error) = child.kill() {
                eprintln!("could not kill owned fixture child: {error}");
            }
            if let Err(error) = child.wait() {
                eprintln!("could not reap owned fixture child: {error}");
            }
        }
    }
}

pub(super) struct GroupFixture {
    directory: PathBuf,
    script: PathBuf,
}

impl GroupFixture {
    pub(super) fn new(directory: &Path) -> anyhow::Result<Self> {
        let script = directory.join("original-group.sh");
        fs::write(&script, include_str!("supervision_target.sh")).test()?;
        Ok(Self {
            directory: directory.to_path_buf(),
            script,
        })
    }

    pub(super) fn target(&self, command: &mut ProcessCommand) {
        command.arg("sh").arg(&self.script).arg(&self.directory);
    }

    pub(super) fn wait_ready(&self) {
        for name in ["leader-ready", "member-ready"] {
            assert!(
                super::wait_for_file(&self.directory.join(name), ATTEMPTS, RETRY_DELAY),
                "original-group fixture did not publish {name}"
            );
        }
    }

    pub(super) fn wait_for_cleanup_lease(&self, contender: &fs::File) -> anyhow::Result<()> {
        for _ in 0..ATTEMPTS {
            match contender.try_lock_exclusive() {
                Ok(()) => {
                    // Do not wait for receipts after acquiring: cleanup must precede release.
                    for name in ["member-term", "leader-term"] {
                        assert!(
                            self.directory.join(name).exists(),
                            "the exact lease became exclusive before {name}"
                        );
                    }
                    return Ok(());
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    thread::sleep(RETRY_DELAY);
                }
                Err(error) => return Err(error.into()),
            }
        }
        anyhow::bail!(
            "cleanup did not release the exact lease within the existing acceptance tripwire"
        )
    }
}

pub(super) fn contended_lease(path: &Path) -> anyhow::Result<fs::File> {
    let contender = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .test()?;
    let error = contender.try_lock_exclusive().test_err()?;
    assert_eq!(
        error.kind(),
        io::ErrorKind::WouldBlock,
        "run did not hold its exact lease"
    );
    Ok(contender)
}

#[expect(
    unsafe_code,
    reason = "register only async-signal-safe fcntl operations in the forked child"
)]
pub(super) fn inherit_descriptors(command: &mut ProcessCommand, descriptors: Vec<RawFd>) {
    // SAFETY: The closure only iterates preallocated integers and calls fcntl.
    // The caller keeps descriptor owners alive through spawn. Parent flags never change.
    unsafe {
        command.pre_exec(move || {
            for descriptor in &descriptors {
                clear_cloexec(*descriptor)?;
            }
            Ok(())
        });
    }
}

#[expect(
    unsafe_code,
    reason = "fcntl reads and writes descriptor flags without dereferencing pointers"
)]
fn clear_cloexec(descriptor: RawFd) -> io::Result<()> {
    // SAFETY: F_GETFD accepts an integer descriptor and does not access caller memory.
    let flags = unsafe { libc::fcntl(descriptor, libc::F_GETFD) };
    if flags < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: F_SETFD accepts only the descriptor and integer flags, with no pointers.
    if unsafe { libc::fcntl(descriptor, libc::F_SETFD, flags & !libc::FD_CLOEXEC) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}
