use super::{
    ChildGuard, Path, ProcessCommand, Stdio, TestResultErrorExt, TestResultExt, UnixStream,
    contended_lease, fs, inherit_descriptors, private_command, wait_for_file,
};
use fs2::FileExt as _;
use std::{
    io,
    os::{fd::AsRawFd as _, unix::fs::MetadataExt as _, unix::process::CommandExt as _},
    thread,
    time::Duration,
};

// Receipt: supervision_fixture.rs uses this existing readiness/cleanup tripwire.
const ATTEMPTS: usize = 100;
const RETRY_DELAY: Duration = Duration::from_millis(20);

#[test]
fn direct_child_termination_failure_retains_supervision_and_exact_lease() -> anyhow::Result<()> {
    let directory = tempfile::tempdir().test()?;
    let ready = directory.path().join("target-ready");
    let stderr_path = directory.path().join("supervisor.stderr");
    let lease_path = directory.path().join("run.lock");
    let lease = fs::File::create(&lease_path).test()?;
    fs2::FileExt::lock_shared(&lease).test()?;
    let metadata = lease.metadata().test()?;
    let (parent, control) = UnixStream::pair().test()?;
    let mut command = private_command(
        control.as_raw_fd(),
        lease.as_raw_fd(),
        (metadata.dev(), metadata.ino()),
    );
    command
        .args([
            "sh",
            "-c",
            "set -eu; : > \"$1\"; IFS= read -r release || :",
            "direct-child-failure",
        ])
        .arg(&ready)
        .stderr(Stdio::from(fs::File::create(&stderr_path).test()?));
    inherit_descriptors(&mut command, vec![control.as_raw_fd(), lease.as_raw_fd()]);
    deny_native_kill(&mut command)?;
    let mut supervisor = ChildGuard::spawn(&mut command)?;
    drop(control);
    // Do not unlock the shared open file description handed to the supervisor.
    drop(lease);
    assert!(
        wait_for_file(&ready, ATTEMPTS, RETRY_DELAY),
        "cooperative direct child did not publish readiness"
    );
    let contender = contended_lease(&lease_path)?;

    drop(parent);
    wait_for_retention_diagnostic(&stderr_path)?;
    let error = contender.try_lock_exclusive().test_err()?;
    assert_eq!(
        error.kind(),
        io::ErrorKind::WouldBlock,
        "direct-child termination failure released the exact lease before stdin EOF"
    );

    supervisor.release_input();
    assert_eq!(supervisor.wait()?.code(), Some(1));
    contender.try_lock_exclusive().test_context(
        "supervisor must release the exact lease after the cooperative direct child exits",
    )?;
    Ok(())
}

fn wait_for_retention_diagnostic(path: &Path) -> anyhow::Result<()> {
    let mut stderr = String::new();
    for _ in 0..ATTEMPTS {
        stderr = fs::read_to_string(path).test()?;
        if stderr.lines().any(|line| {
            line.contains("error: cannot terminate direct run child ")
                && line.contains("retaining run lease and supervision until it exits")
        }) {
            return Ok(());
        }
        thread::sleep(RETRY_DELAY);
    }
    anyhow::bail!(
        "direct-child failure did not report lease retention within the existing acceptance tripwire; stderr:\n{stderr}"
    )
}

#[expect(
    unsafe_code,
    reason = "register a child-only seccomp setup using preallocated instructions and prctl"
)]
fn deny_native_kill(command: &mut ProcessCommand) -> anyhow::Result<()> {
    // This is fault injection for native Rustix kill/group-kill, not a sandbox.
    // The cooperative native shell needs no signals or privilege elevation.
    let filter = [
        libc::sock_filter {
            code: u16::try_from(libc::BPF_LD | libc::BPF_W | libc::BPF_ABS).test()?,
            jt: 0,
            jf: 0,
            k: u32::try_from(std::mem::offset_of!(libc::seccomp_data, nr)).test()?,
        },
        libc::sock_filter {
            code: u16::try_from(libc::BPF_JMP | libc::BPF_JEQ | libc::BPF_K).test()?,
            jt: 0,
            jf: 1,
            k: u32::try_from(libc::SYS_kill).test()?,
        },
        libc::sock_filter {
            code: u16::try_from(libc::BPF_RET | libc::BPF_K).test()?,
            jt: 0,
            jf: 0,
            k: libc::SECCOMP_RET_ERRNO | u32::try_from(libc::EPERM).test()?,
        },
        libc::sock_filter {
            code: u16::try_from(libc::BPF_RET | libc::BPF_K).test()?,
            jt: 0,
            jf: 0,
            k: libc::SECCOMP_RET_ALLOW,
        },
    ];
    let length = u16::try_from(filter.len()).test()?;
    // SAFETY: After fork the closure only borrows its captured array, builds a
    // stack descriptor, and calls prctl through install_filter. It neither
    // allocates nor locks. The parent never installs either process policy.
    unsafe {
        command.pre_exec(move || install_filter(&filter, length));
    }
    Ok(())
}

#[expect(
    unsafe_code,
    reason = "prctl installs a seccomp filter only in the freshly forked fixture child"
)]
fn install_filter(filter: &[libc::sock_filter], length: u16) -> io::Result<()> {
    let program = libc::sock_fprog {
        len: length,
        filter: filter.as_ptr().cast_mut(),
    };
    // SAFETY: This prctl operation takes integer arguments and touches no caller
    // memory. It prevents this fixture and its target from gaining privileges.
    if unsafe {
        libc::prctl(
            libc::PR_SET_NO_NEW_PRIVS,
            1_usize,
            0_usize,
            0_usize,
            0_usize,
        )
    } < 0
    {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: The descriptor and its preallocated instructions remain alive
    // throughout prctl, which copies them without modifying them. length comes
    // from this array's checked length. No allocation or locks occur after fork.
    if unsafe {
        libc::prctl(
            libc::PR_SET_SECCOMP,
            libc::c_ulong::from(libc::SECCOMP_MODE_FILTER),
            &raw const program,
            0_usize,
            0_usize,
        )
    } < 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}
