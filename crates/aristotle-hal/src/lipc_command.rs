// SPDX-License-Identifier: AGPL-3.0-or-later
//! Bounded subprocesses for the stock LIPC property tools.

use std::io::{self, Read};
use std::os::fd::AsRawFd;
use std::process::{Child, Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};

const TIMEOUT: Duration = Duration::from_secs(5);
const MAX_OUTPUT: usize = 4096;

pub(crate) fn run(command: &mut Command) -> io::Result<Output> {
    let child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    finish(child, TIMEOUT)
}

fn finish(mut child: Child, timeout: Duration) -> io::Result<Output> {
    let result = collect(&mut child, timeout);
    if result.is_err() {
        let _ = child.kill();
        let _ = child.wait();
    }
    result
}

fn collect(child: &mut Child, timeout: Duration) -> io::Result<Output> {
    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| io::Error::other("no LIPC stdout"))?;
    let fd = stdout.as_raw_fd();
    // SAFETY: fcntl only changes flags on our owned stdout descriptor.
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    // SAFETY: O_NONBLOCK bounds reads even if another process inherits stdout.
    if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        return Err(io::Error::last_os_error());
    }
    let deadline = Instant::now() + timeout;
    let mut output = Vec::new();
    loop {
        drain(&mut stdout, &mut output)?;
        if let Some(status) = child.try_wait()? {
            drain(&mut stdout, &mut output)?;
            return Ok(Output {
                status,
                stdout: output,
                stderr: Vec::new(),
            });
        }
        let now = Instant::now();
        if now >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "LIPC command exceeded its deadline",
            ));
        }
        thread::sleep((deadline - now).min(Duration::from_millis(10)));
    }
}

fn drain(reader: &mut impl Read, output: &mut Vec<u8>) -> io::Result<()> {
    let mut buffer = [0; 1024];
    loop {
        match reader.read(&mut buffer) {
            Ok(0) => return Ok(()),
            Ok(size) => {
                if output.len() + size > MAX_OUTPUT {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "LIPC output exceeds 4096 bytes",
                    ));
                }
                output.extend_from_slice(&buffer[..size]);
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(()),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collects_output_and_exit_status() {
        let output = run(Command::new("sh").args(["-c", "printf active; exit 7"])).unwrap();
        assert_eq!(output.stdout, b"active");
        assert_eq!(output.status.code(), Some(7));
    }

    #[test]
    fn timeout_kills_and_reaps_the_command() {
        let child = Command::new("sleep")
            .arg("60")
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let pid = child.id();
        let started = Instant::now();
        let error = finish(child, Duration::from_millis(20)).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert!(started.elapsed() < Duration::from_secs(2));
        // SAFETY: WNOHANG only inspects the former child; it cannot block.
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
    fn exited_command_does_not_wait_for_stdout_eof() {
        use std::os::fd::{FromRawFd, OwnedFd};
        let mut fds = [0; 2];
        // SAFETY: pipe2 fills the two-element array on success.
        assert_eq!(unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) }, 0);
        // SAFETY: both descriptors were just created and are owned here.
        let (read, write) = unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) };
        let mut child = Command::new("printf")
            .arg("active")
            .stdout(Stdio::from(write.try_clone().unwrap()))
            .spawn()
            .unwrap();
        child.stdout = Some(read.into());
        let output = finish(child, Duration::from_secs(2)).unwrap();
        drop(write);
        assert_eq!(output.stdout, b"active");
    }

    #[test]
    fn rejects_excessive_output() {
        let error = run(Command::new("yes").arg("invalid property output")).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    }
}
