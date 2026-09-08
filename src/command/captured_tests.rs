use std::{
    io::{Read as _, Write as _},
    os::{
        fd::OwnedFd,
        unix::{net::UnixStream, process::CommandExt as _},
    },
    process::Stdio,
    time::Duration,
};

use super::*;
use crate::test_support::{TestResultErrorExt as _, TestResultExt as _};

#[test]
fn capture_preserves_both_streams_and_nonzero_exit_status() -> anyhow::Result<()> {
    let mut command = shell("head -c 262144 /dev/zero; head -c 262144 /dev/zero >&2; exit 23");
    let captured = output(&mut command, deadline()?).test()?;
    assert_eq!(captured.status.code(), Some(23));
    assert_eq!(captured.stdout, vec![0; 262_144]);
    assert_eq!(captured.stderr, vec![0; 262_144]);
    Ok(())
}

#[test]
fn expired_deadline_does_not_spawn_a_command() -> anyhow::Result<()> {
    let directory = tempfile::tempdir().test()?;
    let marker = directory.path().join("spawned");
    let mut command = shell("printf spawned > \"$MARKER\"");
    command.env("MARKER", &marker);
    let error = output(&mut command, Instant::now()).test_err()?;
    assert!(error.to_string().contains("deadline expired"));
    assert!(!marker.exists());
    Ok(())
}

#[test]
fn deadline_terminates_a_hung_command_and_its_pipe_holders() -> anyhow::Result<()> {
    let mut command = shell("sleep 30 & wait");
    let deadline = Instant::now()
        .checked_add(Duration::from_millis(100))
        .test()?;
    let error = output(&mut command, deadline).test_err()?;
    assert!(error.to_string().contains("deadline expired"));
    Ok(())
}

#[test]
fn successful_leader_cannot_leave_background_pipe_holders_blocking_capture() -> anyhow::Result<()> {
    let mut command = shell("sleep 30 & printf stdout; printf stderr >&2; exit 0");
    let captured = output(&mut command, deadline()?).test()?;
    assert!(captured.status.success());
    assert_eq!(captured.stdout, b"stdout");
    assert_eq!(captured.stderr, b"stderr");
    Ok(())
}

#[test]
fn cleanup_retains_the_exited_leader_and_does_not_signal_another_group() -> anyhow::Result<()> {
    require_waitable_children().test()?;
    let sentinel = Command::new("sh")
        .args(["-c", "printf ready; read token; printf alive"])
        .process_group(0)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .test()?;
    let mut sentinel = Target::new(sentinel);
    let mut ready = [0; 5];
    sentinel
        .child
        .stdout
        .as_mut()
        .test()?
        .read_exact(&mut ready)
        .test()?;
    assert_eq!(&ready, b"ready");
    let child = shell("exit 23").process_group(0).spawn().test()?;
    let mut target = Target::new(child);
    let pid = Pid::from_child(&target.child);
    assert!(observe(pid, false).test()?);
    // Exit observation still leaves this exact child waitable for the cleanup guard.
    assert!(observe(pid, true).test()?);
    assert_eq!(target.finish().test()?.code(), Some(23));
    drop(target);
    assert_eq!(
        observe(pid, true).test_err()?.raw_os_error(),
        Some(libc::ECHILD)
    );
    sentinel
        .child
        .stdin
        .as_mut()
        .test()?
        .write_all(b"continue\n")
        .test()?;
    let mut alive = [0; 5];
    sentinel
        .child
        .stdout
        .as_mut()
        .test()?
        .read_exact(&mut alive)
        .test()?;
    assert_eq!(&alive, b"alive");
    assert!(observe(Pid::from_child(&sentinel.child), false).test()?);
    assert!(sentinel.finish().test()?.success());
    Ok(())
}

#[test]
fn guard_drop_cleans_descendants_before_observer_setup() -> anyhow::Result<()> {
    require_waitable_children().test()?;
    let (mut reader, writer) = UnixStream::pair().test()?;
    reader
        .set_read_timeout(Some(Duration::from_secs(10)))
        .test()?;
    let child = Command::new("sh")
        .args(["-c", "sleep 30 & printf ready; wait"])
        .process_group(0)
        .stdin(Stdio::null())
        .stdout(Stdio::from(OwnedFd::from(writer)))
        .stderr(Stdio::null())
        .spawn()
        .test()?;
    let target = Target::new(child);
    let mut ready = [0; 5];
    reader.read_exact(&mut ready).test()?;
    assert_eq!(&ready, b"ready");
    drop(target);
    let mut remaining = Vec::new();
    reader.read_to_end(&mut remaining).test()?;
    assert!(remaining.is_empty());
    Ok(())
}

#[test]
fn guard_drop_joins_an_active_observer_after_termination() -> anyhow::Result<()> {
    require_waitable_children().test()?;
    let child = shell("sleep 30 & wait").process_group(0).spawn().test()?;
    let mut target = Target::new(child);
    let pid = Pid::from_child(&target.child);
    let (send, receive) = mpsc::channel();
    target.observer = Some(
        thread::Builder::new()
            .spawn(move || {
                observe(pid, false)?;
                send.send(()).map_err(io::Error::other)?;
                Ok(())
            })
            .test()?,
    );
    drop(target);
    receive.recv().test()?;
    assert_eq!(
        observe(pid, true).test_err()?.raw_os_error(),
        Some(libc::ECHILD)
    );
    Ok(())
}

#[test]
#[expect(
    unsafe_code,
    reason = "change SIGCHLD only in the isolated fixture child before exec"
)]
fn automatic_reaping_policy_is_rejected_before_spawning() -> anyhow::Result<()> {
    let directory = tempfile::tempdir().test()?;
    let marker = directory.path().join("spawned");
    let mut command = Command::new(std::env::current_exe().test()?);
    command
        .args([
            "--exact",
            "command::captured::tests::nonwaitable_fixture",
            "--nocapture",
        ])
        .env("STACKSTEAD_NONWAITABLE_FIXTURE", "1")
        .env("MARKER", &marker);
    let ignore_sigchld = || {
        // SAFETY: SIG_IGN is a valid signal disposition, applied only in the fixture child.
        if unsafe { libc::signal(libc::SIGCHLD, libc::SIG_IGN) } == libc::SIG_ERR {
            return Err(std::io::Error::last_os_error());
        }
        Ok(())
    };
    // SAFETY: the closure uses only async-signal-safe signal setup and errno access.
    unsafe { command.pre_exec(ignore_sigchld) };
    let result = command.output().test()?;
    assert!(
        result.status.success(),
        "automatic-reaping subprocess fixture failed: {result:?}"
    );
    assert!(!marker.exists());
    Ok(())
}

#[test]
fn nonwaitable_fixture() -> anyhow::Result<()> {
    if std::env::var_os("STACKSTEAD_NONWAITABLE_FIXTURE").is_none() {
        return Ok(());
    }
    let mut command = shell("printf spawned > \"$MARKER\"");
    let error = output(&mut command, deadline()?).test_err()?;
    assert!(error.to_string().contains("require default SIGCHLD"));
    Ok(())
}

fn shell(script: &str) -> Command {
    let mut command = Command::new("sh");
    command
        .args(["-c", script])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command
}

fn deadline() -> anyhow::Result<Instant> {
    Instant::now().checked_add(Duration::from_secs(10)).test()
}
