//! Starting the real daemon binary from a test without ever leaving one
//! behind.
//!
//! A daemon that outlives its test is a live tracker: it reads `/dev/input`,
//! and it keeps every file descriptor it was handed, including a build
//! wrapper's lock. Three things stop that, each for a different way out:
//!
//! - [`Daemon`] kills and reaps the child when it is dropped, so a failed
//!   assertion anywhere in the test cleans up.
//! - The child asks the kernel for SIGKILL when the thread that started it
//!   ends. That covers the test binary being killed, where no destructor
//!   runs. The request is kept across `execve`, so it still holds after the
//!   daemon has replaced itself to pick up `OMP_THREAD_LIMIT`.
//! - Every descriptor from 3 up is marked close-on-exec in the child, so
//!   the daemon holds nothing it inherited by accident.

use std::os::unix::process::CommandExt;
use std::process::{Child, Command};

/// A started daemon. Dropping it kills the process and waits for it.
pub struct Daemon {
    pub child: Child,
}

impl Drop for Daemon {
    fn drop(&mut self) {
        // Both fail harmlessly when the test already waited for the exit.
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Spawn `cmd` (the daemon binary, with its arguments, environment and stdio
/// already set) with the protections above.
pub fn spawn_daemon(mut cmd: Command) -> Daemon {
    let starter = std::process::id() as libc::pid_t;
    // SAFETY: the closure runs in the forked child before exec and makes
    // only async-signal-safe calls (`prctl`, `getppid`, `close_range`,
    // `getrlimit`, `fcntl`) and allocates nothing.
    unsafe {
        cmd.pre_exec(move || {
            if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL as libc::c_ulong) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            // The starter may have died between fork and prctl. Then the
            // signal was never armed and nobody is left to clean up.
            if libc::getppid() != starter {
                return Err(std::io::Error::from_raw_os_error(libc::ESRCH));
            }
            close_inherited_fds_on_exec();
            Ok(())
        });
    }
    Daemon {
        child: cmd.spawn().expect("spawn daemon"),
    }
}

/// Mark every descriptor from 3 up close-on-exec. They are not closed here:
/// one of them is the pipe `Command` uses to report a failed exec, and it is
/// already close-on-exec.
///
/// # Safety
/// Call only in a forked child before exec.
unsafe fn close_inherited_fds_on_exec() {
    // SAFETY: `close_range` takes three integers and touches only this
    // process's descriptor table.
    let marked = unsafe {
        libc::syscall(
            libc::SYS_close_range,
            3 as libc::c_uint,
            libc::c_uint::MAX,
            libc::CLOSE_RANGE_CLOEXEC,
        )
    };
    if marked == 0 {
        return;
    }
    // Kernels before 5.11 have no CLOSE_RANGE_CLOEXEC: set the flag on each
    // descriptor up to the soft limit, capped so a huge limit stays quick.
    let mut limit = libc::rlimit {
        rlim_cur: 1024,
        rlim_max: 1024,
    };
    // SAFETY: `getrlimit` writes the one struct passed by pointer.
    unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut limit) };
    let top = limit.rlim_cur.min(65_536) as libc::c_int;
    for fd in 3..top {
        // SAFETY: `fcntl(F_SETFD)` on a descriptor that is not open fails
        // with EBADF and changes nothing.
        unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) };
    }
}
