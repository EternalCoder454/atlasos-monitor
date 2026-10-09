//! End Task, Kill, Stop and Continue.
//!
//! Pids are reused: the kernel wraps at `pid_max`, and a row the user clicked
//! a while ago may name a different process by now. So an action takes the
//! pid *and* the start time the table showed, opens a pidfd, and only then
//! checks the start time. A pidfd stays bound to the process it was opened
//! on, so if the start time still matches after it is open, the signal goes
//! to that process and no other, even if the pid is reused a moment later.
//!
//! No privilege (DESIGN.md, Privilege): another user's process is "not
//! allowed", and nothing escalates.

use std::fmt;
use std::io;

use rustix::io::Errno;
use rustix::process::{Pid, PidfdFlags, Signal, pidfd_open, pidfd_send_signal};

use super::parse::parse_stat;

/// Something to do to a process.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Action {
    /// Ask it to exit, letting it save and clean up (`SIGTERM`).
    End,
    /// Stop it at once, with no chance to save anything (`SIGKILL`).
    Kill,
    /// Freeze it (`SIGSTOP`).
    Stop,
    /// Let a stopped process run again (`SIGCONT`).
    Continue,
}

impl Action {
    fn signal(self) -> Signal {
        match self {
            Action::End => Signal::TERM,
            Action::Kill => Signal::KILL,
            Action::Stop => Signal::STOP,
            Action::Continue => Signal::CONT,
        }
    }
}

/// Why an action wasn't carried out.
#[derive(Debug)]
pub enum ActionError {
    /// The process has exited (or its pid now names another process).
    Gone,
    /// It belongs to another user, or to the system.
    NotAllowed,
    Failed(io::Error),
}

impl From<Errno> for ActionError {
    fn from(e: Errno) -> Self {
        match e {
            Errno::SRCH => ActionError::Gone,
            Errno::PERM | Errno::ACCESS => ActionError::NotAllowed,
            e => ActionError::Failed(e.into()),
        }
    }
}

impl fmt::Display for ActionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ActionError::Gone => f.write_str("the process has exited"),
            ActionError::NotAllowed => f.write_str("not allowed"),
            ActionError::Failed(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for ActionError {}

/// Carries out `action` on the process `pid` that started at `start_time`
/// (clock ticks since boot, as [`super::Proc::start_time`] has it).
pub fn act(pid: u32, start_time: u64, action: Action) -> Result<(), ActionError> {
    // Only ever one process, by its pid: never pid 0 (our process group),
    // -1 (everything we may signal) or a negative one (a group), which the
    // kernel's `kill` takes for those; a `u32` above `i32::MAX` is the
    // negative of one, and is refused rather than wrapped. Nor PID 1, whose
    // signals are the service manager's to take (it ignores `SIGKILL`; for
    // root `SIGTERM` would restart it): the row is shown, not actionable.
    let raw = i32::try_from(pid)
        .ok()
        .filter(|&p| p > 1)
        .and_then(Pid::from_raw)
        .ok_or(if pid == 1 {
            ActionError::NotAllowed
        } else {
            ActionError::Gone
        })?;
    // Opening a pidfd checks no permission, so a failure other than "no
    // such process" (a seccomp filter's EPERM, an old kernel's ENOSYS) is
    // not the user being refused.
    let pidfd = pidfd_open(raw, PidfdFlags::empty()).map_err(|e| match e {
        Errno::SRCH => ActionError::Gone,
        e => ActionError::Failed(e.into()),
    })?;
    // The state isn't checked: a leader that has exited shows as a zombie
    // while its other threads run on, and those a signal still reaches.
    let stat = std::fs::read(format!("/proc/{pid}/stat")).map_err(|_| ActionError::Gone)?;
    if parse_stat(&stat).map(|st| st.start_time) != Some(start_time) {
        return Err(ActionError::Gone);
    }
    pidfd_send_signal(&pidfd, action.signal())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::process::ExitStatusExt;
    use std::process::{Child, Command};
    use std::time::{Duration, Instant};

    use crate::process::start_time;

    fn sleeper() -> (Child, u32, u64) {
        let child = Command::new("sleep").arg("30").spawn().unwrap();
        let pid = child.id();
        let start = start_time(pid).unwrap();
        (child, pid, start)
    }

    /// The state letter from `/proc/<pid>/stat`, waiting up to a second for
    /// it to become `want`: a signal is delivered, not acted on, at once.
    fn wait_for_state(pid: u32, want: u8) -> u8 {
        let deadline = Instant::now() + Duration::from_secs(1);
        loop {
            let stat = std::fs::read(format!("/proc/{pid}/stat")).unwrap();
            let state = crate::process::parse_stat(&stat).unwrap().state;
            if state == want || Instant::now() > deadline {
                return state;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn stop_continue_end() {
        let (mut child, pid, start) = sleeper();
        act(pid, start, Action::Stop).unwrap();
        assert_eq!(wait_for_state(pid, b'T'), b'T');
        act(pid, start, Action::Continue).unwrap();
        assert_eq!(wait_for_state(pid, b'S'), b'S');
        act(pid, start, Action::End).unwrap();
        assert_eq!(child.wait().unwrap().signal(), Some(libc::SIGTERM));
    }

    #[test]
    fn kill() {
        let (mut child, pid, start) = sleeper();
        act(pid, start, Action::Kill).unwrap();
        assert_eq!(child.wait().unwrap().signal(), Some(libc::SIGKILL));
    }

    /// A start time that doesn't match is another process given the same
    /// pid: it must be left alone.
    #[test]
    fn a_reused_pid_is_left_alone() {
        let (mut child, pid, start) = sleeper();
        assert!(matches!(
            act(pid, start + 1, Action::Kill),
            Err(ActionError::Gone)
        ));
        assert!(
            child.try_wait().unwrap().is_none(),
            "the wrong process was killed"
        );
        child.kill().unwrap();
        child.wait().unwrap();
    }

    #[test]
    fn a_process_that_is_gone() {
        let (mut child, pid, start) = sleeper();
        child.kill().unwrap();
        child.wait().unwrap();
        assert!(matches!(
            act(pid, start, Action::End),
            Err(ActionError::Gone)
        ));
        assert!(matches!(act(0, 0, Action::End), Err(ActionError::Gone)));
        assert!(matches!(
            act(u32::MAX, 0, Action::End),
            Err(ActionError::Gone)
        ));
    }

    #[test]
    fn errors_map_to_what_the_user_is_told() {
        assert!(matches!(ActionError::from(Errno::SRCH), ActionError::Gone));
        assert!(matches!(
            ActionError::from(Errno::PERM),
            ActionError::NotAllowed
        ));
        assert!(matches!(
            ActionError::from(Errno::INVAL),
            ActionError::Failed(_)
        ));
        assert_eq!(ActionError::NotAllowed.to_string(), "not allowed");
    }
}
