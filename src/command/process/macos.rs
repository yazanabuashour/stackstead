use std::io;

#[cfg(target_os = "macos")]
use rustix::process::Pid;

#[cfg(target_os = "macos")]
#[expect(
    unsafe_code,
    reason = "libc exposes the macOS presence query but Rustix has no safe wrapper"
)]
pub fn leader_is_alone(pid: Pid) -> io::Result<bool> {
    // Caller owns this unreaped, exit-observed child through the last signal.
    // Two slots prove existence of another member, not a process-count limit:
    // one can be the leader; two distinct PIDs must include somebody else.
    let mut pids: [libc::pid_t; 2] = [0; 2];
    let bytes = libc::c_int::try_from(std::mem::size_of_val(&pids)).map_err(io::Error::other)?;
    // SAFETY: pids provides initialized, aligned writable storage matching bytes.
    // libproc borrows it only for this call. Input size is bytes; output is a PID
    // count, including zombies. No pointer or enumerated PID escapes this helper.
    let count =
        unsafe { libc::proc_listpgrppids(pid.as_raw_pid(), pids.as_mut_ptr().cast(), bytes) };
    classify_members(pid.as_raw_pid(), count, pids)
}

fn classify_members(
    leader: libc::pid_t,
    count: libc::c_int,
    pids: [libc::pid_t; 2],
) -> io::Result<bool> {
    match (count, pids) {
        (1, [pid, _]) if leader > 0 && pid > 0 => Ok(pid == leader),
        (2, [first, second]) if leader > 0 && first > 0 && second > 0 && first != second => {
            Ok(false)
        }
        // libproc also maps syscall failures to zero. Never report absence or
        // attach stale errno to zero, malformed counts, or malformed entries.
        _ => Err(io::Error::other(
            "proc_listpgrppids returned an unknown or invalid membership snapshot",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{TestResultErrorExt as _, TestResultExt as _};

    #[test]
    fn membership_uses_returned_pid_count_and_only_the_written_prefix() -> anyhow::Result<()> {
        for (count, pids, alone) in [
            (1, [41, 0], true),
            (1, [41, -1], true),
            (1, [41, 41], true),
            (1, [42, 0], false),
            (2, [41, 42], false),
            (2, [42, 41], false),
            (2, [42, 43], false),
        ] {
            assert_eq!(classify_members(41, count, pids).test()?, alone);
        }
        Ok(())
    }

    #[test]
    fn unknown_membership_never_proves_a_lone_leader() -> anyhow::Result<()> {
        for (leader, count, pids) in [
            (41, 0, [41, 0]),
            (41, -1, [41, 0]),
            (41, libc::c_int::MIN, [41, 0]),
            (41, 3, [41, 42]),
            (41, 8, [41, 42]),
            (41, libc::c_int::MAX, [41, 42]),
            (41, 1, [0, 41]),
            (41, 1, [-1, 41]),
            (41, 2, [41, 0]),
            (41, 2, [-1, 41]),
            (41, 2, [41, 41]),
            (41, 2, [42, 42]),
            (0, 1, [41, 0]),
            (-1, 1, [41, 0]),
        ] {
            let error = classify_members(leader, count, pids).test_err()?;
            assert_eq!(error.kind(), io::ErrorKind::Other);
        }
        Ok(())
    }
}
