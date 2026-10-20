//! The few system calls std does not wrap, in one place: waiting for the
//! game while signals arrive, priorities, CPU pinning and file locks. Every
//! `unsafe` block of seg is in this file.

#![allow(unsafe_code)]

use std::fs::File;
use std::io;
use std::os::fd::AsRawFd;
use std::sync::atomic::{AtomicI32, Ordering};
use std::time::{Duration, Instant};

/// The game's process id, for the signal handler to forward to.
static CHILD: AtomicI32 = AtomicI32::new(0);
/// The first signal that asked seg to stop (0: none yet).
static SIGNALLED: AtomicI32 = AtomicI32::new(0);

extern "C" fn on_signal(sig: libc::c_int) {
    // Only async-signal-safe work: atomics and kill(2).
    let _ = SIGNALLED.compare_exchange(0, sig, Ordering::SeqCst, Ordering::SeqCst);
    let child = CHILD.load(Ordering::SeqCst);
    // Ctrl+C already reached the game through the terminal; the others
    // were sent to seg alone.
    if child > 0 && sig != libc::SIGINT {
        // SAFETY: kill is async-signal-safe and takes plain integers.
        unsafe { libc::kill(child, sig) };
    }
}

/// Tell the signal handler where to forward signals.
pub fn set_child(pid: u32) {
    CHILD.store(pid as i32, Ordering::SeqCst);
}

/// Make INT, TERM, HUP and QUIT stop the wait (and reach the game) instead
/// of killing seg before it has put the system back.
pub fn install_signal_handlers() -> io::Result<()> {
    for sig in [libc::SIGINT, libc::SIGTERM, libc::SIGHUP, libc::SIGQUIT] {
        // SAFETY: a zeroed sigaction is valid; the handler is an
        // `extern "C" fn(c_int)` that only does async-signal-safe work.
        let rc = unsafe {
            let mut action: libc::sigaction = std::mem::zeroed();
            action.sa_sigaction = on_signal as extern "C" fn(libc::c_int) as usize;
            // No SA_RESTART: waitpid must return so the loop sees the flag.
            action.sa_flags = 0;
            libc::sigemptyset(&mut action.sa_mask);
            libc::sigaction(sig, &action, std::ptr::null_mut())
        };
        if rc != 0 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}

/// Start `command` with the given process settings applied between fork and
/// exec, so the game and everything it starts inherit them. Failures to
/// apply a setting (no right to a negative nice value, say) are ignored:
/// the game should start slower rather than not at all.
pub fn spawn_prepared(
    command: &mut std::process::Command,
    nice: Option<i32>,
    io_priority: bool,
    cpus: Option<Vec<usize>>,
) -> io::Result<std::process::Child> {
    use std::os::unix::process::CommandExt;
    // SAFETY: the closure runs in the forked child before exec and only makes
    // system calls (setpriority, ioprio_set, sched_setaffinity); it does not
    // allocate or take locks.
    unsafe {
        command.pre_exec(move || {
            if let Some(n) = nice {
                let _ = set_nice(n);
            }
            if io_priority {
                let _ = set_ioprio_best_effort_top();
            }
            if let Some(cpus) = &cpus {
                let _ = set_affinity(cpus);
            }
            Ok(())
        });
    }
    command.spawn()
}

pub fn pending_signal() -> Option<i32> {
    match SIGNALLED.load(Ordering::SeqCst) {
        0 => None,
        sig => Some(sig),
    }
}

pub fn send_signal(pid: u32, sig: i32) {
    // SAFETY: kill with a process id and a signal number.
    unsafe { libc::kill(pid as i32, sig) };
}

/// The exit code of a wait status, as a shell reports it: the code, or 128
/// plus the signal that ended the process.
pub fn exit_code(status: i32) -> i32 {
    if libc::WIFEXITED(status) {
        libc::WEXITSTATUS(status)
    } else if libc::WIFSIGNALED(status) {
        128 + libc::WTERMSIG(status)
    } else {
        1
    }
}

/// Wait for `pid` to end while signals may arrive. After one does, the
/// game gets 5 seconds to leave, then SIGTERM, then at 15 SIGKILL, so seg is
/// never stuck behind a game that ignores it. Returns the exit code.
pub fn wait_for(pid: u32) -> io::Result<i32> {
    let mut stopping: Option<Instant> = None;
    let mut terminated = false;
    loop {
        if stopping.is_none() && pending_signal().is_some() {
            stopping = Some(Instant::now());
        }
        let flags = if stopping.is_some() { libc::WNOHANG } else { 0 };
        let mut status = 0;
        // SAFETY: waitpid writes one int through a valid pointer.
        let rc = unsafe { libc::waitpid(pid as i32, &mut status, flags) };
        if rc == pid as i32 {
            return Ok(exit_code(status));
        }
        if rc == -1 {
            let e = io::Error::last_os_error();
            match e.raw_os_error() {
                Some(libc::EINTR) => continue,
                // Somebody else reaped it: nothing left to wait for.
                Some(libc::ECHILD) => return Ok(0),
                _ => return Err(e),
            }
        }
        if let Some(since) = stopping {
            let waited = since.elapsed();
            if waited > Duration::from_secs(5) && !terminated {
                send_signal(pid, libc::SIGTERM);
                terminated = true;
            }
            if waited > Duration::from_secs(15) {
                send_signal(pid, libc::SIGKILL);
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}

/// Ask for the nice value `nice` for this process (and so for what it starts).
/// Negative values need the `nice` limit pam_limits gives the gamemode group.
pub fn set_nice(nice: i32) -> io::Result<()> {
    // SAFETY: setpriority takes plain integers.
    let rc = unsafe { libc::setpriority(libc::PRIO_PROCESS, 0, nice) };
    if rc == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

/// The highest priority within the best-effort I/O class.
pub fn set_ioprio_best_effort_top() -> io::Result<()> {
    const IOPRIO_WHO_PROCESS: libc::c_long = 1;
    const IOPRIO_CLASS_BE: libc::c_long = 2;
    const IOPRIO_CLASS_SHIFT: libc::c_long = 13;
    // SAFETY: ioprio_set(2) takes three plain integers.
    let rc = unsafe {
        libc::syscall(
            libc::SYS_ioprio_set,
            IOPRIO_WHO_PROCESS,
            0 as libc::c_long,
            IOPRIO_CLASS_BE << IOPRIO_CLASS_SHIFT,
        )
    };
    if rc == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

/// Restrict this process (and what it starts) to `cpus`.
pub fn set_affinity(cpus: &[usize]) -> io::Result<()> {
    // SAFETY: cpu_set_t is plain data; CPU_SET only sets bits inside it and
    // is skipped for numbers it cannot hold.
    unsafe {
        let mut set: libc::cpu_set_t = std::mem::zeroed();
        libc::CPU_ZERO(&mut set);
        let capacity = 8 * std::mem::size_of::<libc::cpu_set_t>();
        let mut any = false;
        for &cpu in cpus {
            if cpu < capacity {
                libc::CPU_SET(cpu, &mut set);
                any = true;
            }
        }
        if !any {
            return Err(io::Error::from(io::ErrorKind::InvalidInput));
        }
        if libc::sched_setaffinity(0, std::mem::size_of::<libc::cpu_set_t>(), &set) == 0 {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        }
    }
}

/// An exclusive lock on `file`, released when it is closed.
pub fn lock_exclusive(file: &File) -> io::Result<()> {
    loop {
        // SAFETY: flock on a file descriptor that stays open for the call.
        let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) };
        if rc == 0 {
            return Ok(());
        }
        let e = io::Error::last_os_error();
        if e.raw_os_error() != Some(libc::EINTR) {
            return Err(e);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    #[test]
    fn wait_status_becomes_a_shell_exit_code() {
        let exited = |code: i32| code << 8;
        assert_eq!(exit_code(exited(0)), 0);
        assert_eq!(exit_code(exited(3)), 3);
        // Killed by SIGKILL (9).
        assert_eq!(exit_code(9), 137);
        // Killed by SIGTERM (15).
        assert_eq!(exit_code(15), 143);
    }

    // wait_for reaps the children itself (waitpid), which clippy cannot see.
    #[test]
    #[allow(clippy::zombie_processes)]
    fn waiting_returns_the_childs_exit_code() {
        let child = Command::new("sh").args(["-c", "exit 7"]).spawn().unwrap();
        assert_eq!(wait_for(child.id()).unwrap(), 7);
        let child = Command::new("sh")
            .args(["-c", "kill -9 $$"])
            .spawn()
            .unwrap();
        assert_eq!(wait_for(child.id()).unwrap(), 137);
    }

    #[test]
    fn a_prepared_child_gets_its_nice_value_and_cpus() {
        let mut cmd = std::process::Command::new("sh");
        // nice can always be raised; the child reports its own values.
        cmd.args(["-c", "ps -o ni= -p $$ | tr -d ' ' >&2; exit 3"])
            .stderr(std::process::Stdio::piped());
        let child = spawn_prepared(&mut cmd, Some(5), true, Some(vec![0])).unwrap();
        let out = child.wait_with_output().unwrap();
        assert_eq!(out.status.code(), Some(3));
        assert_eq!(String::from_utf8_lossy(&out.stderr).trim(), "5");
    }

    #[test]
    fn affinity_needs_at_least_one_cpu() {
        assert!(set_affinity(&[]).is_err());
        assert!(set_affinity(&[1_000_000]).is_err());
    }

    #[test]
    fn locks_can_be_taken_twice_in_one_process() {
        // flock is per open file description: a second open blocks, but the
        // same description may lock again.
        let f = File::open("/proc/self/status").unwrap();
        lock_exclusive(&f).unwrap();
        lock_exclusive(&f).unwrap();
    }
}
