// SPDX-License-Identifier: AGPL-3.0-or-later
//! Owned KAF compatibility service for powerd after a framework takeover.

use std::io::{self, Read};
use std::os::fd::AsRawFd;
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

const SCRIPT: &str = include_str!("framework.lua");
const READY: &[u8] = b"aristotle-framework-ready\n";
const STARTUP: Duration = Duration::from_secs(5);

pub(crate) struct Framework {
    child: Child,
}

impl Framework {
    pub(crate) fn acquire() -> io::Result<Option<Self>> {
        let existing = crate::powerd::get_prop("com.lab126.kaf", "frameworkStarted", true);
        let takeover = std::path::Path::new("/var/run/aristotle-takeover").is_file();
        if !needs_shim(existing, takeover)? {
            return Ok(None);
        }
        let mut command = Command::new("/usr/bin/lua");
        command.args(["-e", SCRIPT]);
        let mut guard = Self::start(&mut command, STARTUP)?;
        let registered = crate::powerd::get_prop("com.lab126.kaf", "frameworkStarted", true)?;
        if registered != "1" || guard.child.try_wait()?.is_some() {
            return Err(io::Error::other(
                "KAF helper registration did not remain ready",
            ));
        }
        Ok(Some(guard))
    }

    fn start(command: &mut Command, timeout: Duration) -> io::Result<Self> {
        let parent = std::process::id();
        // SAFETY: the child only makes async-signal-safe system calls before exec.
        unsafe {
            command.pre_exec(move || {
                if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL) < 0 {
                    return Err(io::Error::last_os_error());
                }
                if libc::getppid() != parent as libc::pid_t {
                    return Err(io::Error::from_raw_os_error(libc::ECHILD));
                }
                Ok(())
            });
        }
        let mut guard = Self {
            child: command
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .spawn()?,
        };
        guard.ready(timeout)?;
        Ok(guard)
    }

    fn ready(&mut self, timeout: Duration) -> io::Result<()> {
        let mut stdout = self.child.stdout.take().unwrap();
        let fd = stdout.as_raw_fd();
        // SAFETY: fcntl only changes the flags of our owned pipe descriptor.
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
        // SAFETY: nonblocking reads keep an unresponsive Lua child within the deadline.
        if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
            return Err(io::Error::last_os_error());
        }
        let deadline = Instant::now() + timeout;
        let mut output = Vec::new();
        loop {
            let mut buffer = [0; 64];
            match stdout.read(&mut buffer) {
                Ok(size) => output.extend_from_slice(&buffer[..size]),
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(error),
            }
            if self.child.try_wait()?.is_some() {
                return Err(io::Error::other("KAF helper exited during startup"));
            }
            if output == READY {
                return Ok(());
            }
            if !READY.starts_with(&output) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "unexpected KAF helper readiness response",
                ));
            }
            let now = Instant::now();
            if now >= deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "KAF helper did not become ready",
                ));
            }
            thread::sleep((deadline - now).min(Duration::from_millis(10)));
        }
    }
}

fn needs_shim(existing: io::Result<String>, takeover: bool) -> io::Result<bool> {
    match existing {
        Ok(value) if value.parse::<u32>().is_ok_and(|value| value != 0) => Ok(false),
        Ok(_) => Err(io::Error::other("existing KAF is not ready")),
        Err(error) if !takeover => Err(error),
        Err(_) => Ok(true),
    }
}

impl Drop for Framework {
    fn drop(&mut self) {
        let _ = self.child.kill();
        if let Err(error) = self.child.wait() {
            eprintln!("aristotle: reaping KAF helper: {error}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn command(script: &str) -> Command {
        let mut command = Command::new("sh");
        command.args(["-c", script]);
        command
    }

    fn reaped(pid: u32) {
        // SAFETY: WNOHANG inspects only the child PID and cannot block.
        assert_eq!(
            unsafe { libc::waitpid(pid as i32, std::ptr::null_mut(), libc::WNOHANG) },
            -1
        );
        assert_eq!(
            io::Error::last_os_error().raw_os_error(),
            Some(libc::ECHILD)
        );
    }

    #[test]
    fn existing_kaf_is_never_replaced() {
        assert!(!needs_shim(Ok("1".into()), true).unwrap());
        assert!(needs_shim(Ok("0".into()), true).is_err());
        assert!(needs_shim(Ok("invalid".into()), true).is_err());
        assert!(needs_shim(Err(io::Error::other("absent")), false).is_err());
        assert!(needs_shim(Err(io::Error::other("absent")), true).unwrap());
    }

    #[test]
    fn ready_child_is_owned_until_drop_and_reaped() {
        let guard = Framework::start(
            &mut command("printf 'aristotle-framework-ready\\n'; exec sleep 60"),
            Duration::from_secs(1),
        )
        .unwrap();
        let pid = guard.child.id();
        drop(guard);
        reaped(pid);
    }

    #[test]
    fn timeout_kills_and_reaps_the_unready_child() {
        let mut guard = Framework {
            child: command("exec sleep 60")
                .stdout(Stdio::piped())
                .spawn()
                .unwrap(),
        };
        let pid = guard.child.id();
        let error = guard.ready(Duration::from_millis(20)).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        drop(guard);
        reaped(pid);
    }

    #[test]
    fn name_collision_or_registration_failure_cannot_become_ready() {
        assert!(Framework::start(&mut command("exit 1"), Duration::from_secs(1)).is_err());
        assert!(Framework::start(
            &mut command("printf invalid; exec sleep 60"),
            Duration::from_secs(1)
        )
        .is_err());
    }
}
