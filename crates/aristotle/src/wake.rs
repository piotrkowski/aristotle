// SPDX-License-Identifier: AGPL-3.0-or-later
//! Coalesced worker notifications for the input loop.

use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::mpsc;
use std::sync::OnceLock;

static WAKE: OnceLock<Result<OwnedFd, i32>> = OnceLock::new();
static SIGNAL_FD: AtomicI32 = AtomicI32::new(-1);

pub struct Sender<T>(Option<mpsc::Sender<T>>);

pub fn channel<T>() -> (Sender<T>, mpsc::Receiver<T>) {
    let (tx, rx) = mpsc::channel();
    (Sender(Some(tx)), rx)
}

impl<T> Sender<T> {
    pub fn send(&self, value: T) -> Result<(), mpsc::SendError<T>> {
        let result = self.0.as_ref().expect("sender is live").send(value);
        notify();
        result
    }
}

impl<T> Clone for Sender<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<T> Drop for Sender<T> {
    fn drop(&mut self) {
        self.0.take();
        notify();
    }
}

pub fn fd() -> io::Result<RawFd> {
    match WAKE.get_or_init(|| {
        // SAFETY: eventfd returns a new descriptor or -1.
        let fd = unsafe { libc::eventfd(0, libc::EFD_NONBLOCK | libc::EFD_CLOEXEC) };
        if fd < 0 {
            Err(io::Error::last_os_error()
                .raw_os_error()
                .unwrap_or(libc::EIO))
        } else {
            // SAFETY: this descriptor has one owner for the process lifetime.
            Ok(unsafe { OwnedFd::from_raw_fd(fd) })
        }
    }) {
        Ok(fd) => {
            let fd = fd.as_raw_fd();
            SIGNAL_FD.store(fd, Ordering::Relaxed);
            Ok(fd)
        }
        Err(errno) => Err(io::Error::from_raw_os_error(*errno)),
    }
}

pub fn notify() {
    if let Ok(fd) = fd() {
        let one = 1u64;
        loop {
            // SAFETY: the pointer covers eight initialized bytes; the descriptor is nonblocking.
            let result = unsafe { libc::write(fd, (&one as *const u64).cast(), 8) };
            if result >= 0 || io::Error::last_os_error().kind() != io::ErrorKind::Interrupted {
                break;
            }
        }
    }
}

/// Async-signal-safe notification after initialization by the event loop.
pub fn signal() {
    let fd = SIGNAL_FD.load(Ordering::Relaxed);
    if fd >= 0 {
        let one = 1u64;
        // SAFETY: errno is thread-local; write is async-signal-safe and the descriptor is nonblocking.
        unsafe {
            let errno = libc::__errno_location();
            let saved = *errno;
            libc::write(fd, (&one as *const u64).cast(), 8);
            *errno = saved;
        }
    }
}

pub fn drain() {
    if let Ok(fd) = fd() {
        let mut count = 0u64;
        // SAFETY: the pointer covers eight writable bytes; one read drains the counter.
        unsafe { libc::read(fd, (&mut count as *mut u64).cast(), 8) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn results_and_disconnection_wake_a_blocked_poll() {
        let fd = fd().unwrap();
        for disconnected in [false, true] {
            drain();
            let (tx, rx) = channel();
            let worker = std::thread::spawn(move || {
                if !disconnected {
                    tx.send(42).unwrap();
                }
            });
            let mut event = libc::pollfd {
                fd,
                events: libc::POLLIN,
                revents: 0,
            };
            // SAFETY: one initialized pollfd with a live descriptor.
            assert_eq!(unsafe { libc::poll(&mut event, 1, 1000) }, 1);
            worker.join().unwrap();
            assert_ne!(event.revents & libc::POLLIN, 0);
            if disconnected {
                assert!(rx.recv().is_err());
            } else {
                assert_eq!(rx.recv().unwrap(), 42);
            }
        }
    }
}
