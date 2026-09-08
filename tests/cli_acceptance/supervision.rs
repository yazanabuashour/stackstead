use super::*;
use std::{
    os::{fd::AsRawFd, unix::fs::MetadataExt, unix::net::UnixStream},
    process::Stdio,
};

#[cfg(target_os = "linux")]
#[path = "supervision_fault.rs"]
mod fault;
#[path = "supervision_fixture.rs"]
mod fixture;
use fixture::{ChildGuard, GroupFixture, contended_lease, inherit_descriptors};

const PRIVATE_SUPERVISOR: &str = "__stackstead_run_supervisor_v1";

#[test]
fn private_supervisor_rejects_abandoned_or_invalid_handoffs_before_exec() -> anyhow::Result<()> {
    for case in [
        "pending-eof",
        "reserved-control",
        "reserved-lease",
        "duplicate",
        "closed-control",
        "closed-lease",
        "wrong-lease",
    ] {
        let directory = tempfile::tempdir().test()?;
        let marker = directory.path().join("target-executed");
        let lease = tempfile::tempfile().test()?;
        fs2::FileExt::lock_shared(&lease).test()?;
        let metadata = lease.metadata().test()?;
        let other_lease = tempfile::tempfile().test()?;
        let other_metadata = other_lease.metadata().test()?;
        assert_ne!(
            (metadata.dev(), metadata.ino()),
            (other_metadata.dev(), other_metadata.ino())
        );
        let (parent, control) = UnixStream::pair().test()?;
        let mut parent = Some(parent);
        let mut control_number = control.as_raw_fd();
        let mut lease_number = lease.as_raw_fd();
        let mut identity = (metadata.dev(), metadata.ino());
        let mut inherited = vec![control_number, lease_number];
        match case {
            "pending-eof" => drop(parent.take()),
            "reserved-control" => control_number = 0,
            "reserved-lease" => lease_number = 1,
            "duplicate" => lease_number = control_number,
            "closed-control" => inherited.retain(|fd| *fd != control_number),
            "closed-lease" => inherited.retain(|fd| *fd != lease_number),
            "wrong-lease" => identity = (other_metadata.dev(), other_metadata.ino()),
            _ => anyhow::bail!("unknown handoff fixture {case}"),
        }
        let mut command = private_command(control_number, lease_number, identity);
        command
            .args(["sh", "-c", "printf executed > \"$1\"", "handoff-probe"])
            .arg(&marker);
        inherit_descriptors(&mut command, inherited);
        let mut supervisor = ChildGuard::spawn(&mut command)?;
        drop(control);
        // Closing, not unlocking, preserves the handed-off open file description.
        drop(lease);
        let status = supervisor.wait()?;
        assert_eq!(
            status.code(),
            Some(if case == "pending-eof" { 143 } else { 1 }),
            "unexpected private supervisor result for {case}"
        );
        assert!(!marker.exists(), "{case} executed an unauthorized target");
        drop(parent);
    }
    Ok(())
}

#[test]
fn private_control_eof_cancels_the_original_group_and_releases_its_exact_lease()
-> anyhow::Result<()> {
    let directory = tempfile::tempdir().test()?;
    let fixture = GroupFixture::new(directory.path())?;
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
    fixture.target(&mut command);
    inherit_descriptors(&mut command, vec![control.as_raw_fd(), lease.as_raw_fd()]);
    let mut supervisor = ChildGuard::spawn(&mut command)?;
    drop(control);
    drop(lease);
    fixture.wait_ready();
    let contender = contended_lease(&lease_path)?;

    drop(parent);
    fixture.wait_for_cleanup_lease(&contender)?;
    assert_eq!(supervisor.wait()?.code(), Some(143));
    Ok(())
}

#[test]
fn run_wrapper_signals_cancel_the_original_group_before_lease_release() -> anyhow::Result<()> {
    use rustix::process::Signal;
    use std::os::unix::process::ExitStatusExt as _;

    let project = Project::initialized()?;
    let manifest = project.create("feature-a")?;
    for (name, signal) in [
        ("term", Signal::TERM),
        ("int", Signal::INT),
        ("kill", Signal::KILL),
    ] {
        let directory = project.repo.parent().test()?.join(name);
        fs::create_dir(&directory).test()?;
        let fixture = GroupFixture::new(&directory)?;
        let mut command = ProcessCommand::new(assert_cmd::cargo::cargo_bin!("stackstead"));
        command
            .current_dir(&project.repo)
            .env("XDG_STATE_HOME", test_state_home(&project.repo))
            .args(["run", &manifest.stackstead_id, "--"]);
        fixture.target(&mut command);
        let mut wrapper = ChildGuard::spawn(&mut command)?;
        fixture.wait_ready();
        let contender = contended_lease(&manifest.state_dir.join("run.lock"))?;

        wrapper.signal(signal)?;
        fixture.wait_for_cleanup_lease(&contender)?;
        assert!(
            wrapper.wait()?.signal().is_some(),
            "{name} did not kill the wrapper"
        );
    }
    Ok(())
}

fn private_command(control: i32, lease: i32, identity: (u64, u64)) -> ProcessCommand {
    let mut command = ProcessCommand::new(assert_cmd::cargo::cargo_bin!("stackstead"));
    command
        .arg(PRIVATE_SUPERVISOR)
        .arg(control.to_string())
        .arg(lease.to_string())
        .arg(identity.0.to_string())
        .arg(identity.1.to_string())
        .arg("--")
        .stdout(Stdio::null());
    command
}
