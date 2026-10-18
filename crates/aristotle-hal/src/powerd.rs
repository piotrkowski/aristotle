// SPDX-License-Identifier: AGPL-3.0-or-later
//! Sleep through the stock power daemon, `powerd`: its screensaver and
//! suspend events, and the few properties an app sets, over LIPC.
//!
//! Aristotle is static musl and cannot link `liblipc.so`, so it runs the
//! stock command-line tools, as KOReader does for the events
//! (`lipc-wait-event -m` in a child, read line by line) [ko]:
//!
//! - [`PowerEvents`] follows `com.lab126.powerd` from a child process and a
//!   reader thread, and exposes a file descriptor to add to the input
//!   loop's `poll`, so an event wakes the loop at once;
//! - [`Lipc`] sets properties from a thread of its own, so the input loop
//!   never waits for a `lipc-set-prop`.
//!
//! The flow, read from powerd's strings and logs on our PW4 [dev]: a press
//! of the power button (or `powerButton 1`, or the idle timer T1, 600 s)
//! moves powerd from `active` to `screenSaver` and emits
//! `goingToScreenSaver`; the app draws its screensaver. About 60 s later
//! (T2) it emits `readyToSuspend` and suspends a few seconds after that;
//! the e-ink picture stays. The next press emits `outOfScreenSaver` and the
//! app redraws. `touchScreenSaverTimeout 1` restarts T1. powerd ignores the
//! button while `preventScreenSaver` is 1, and while it thinks USB drive
//! mode is on, which on our unit includes the USB network [dev].

use std::io::{self, BufRead, BufReader};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

const POWERD: &str = "com.lab126.powerd";
const BLANKET: &str = "com.lab126.blanket";

/// The events [`PowerEvents`] asks for.
const EVENTS: &str = "goingToScreenSaver,outOfScreenSaver,exitingScreenSaver,readyToSuspend,\
                      suspending,wakeupFromSuspend,charging,notCharging";

/// A powerd event. The number is the event's first argument: for the
/// screensaver events the source (1 button wake, 2 button, 4 cover close,
/// 6 cover open [ko]), for `readyToSuspend` the delay in seconds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PowerEvent {
    GoingToScreenSaver(i32),
    OutOfScreenSaver(i32),
    ExitingScreenSaver,
    ReadyToSuspend(i32),
    Suspending,
    WakeupFromSuspend,
    Charging,
    NotCharging,
}

/// One line of `lipc-wait-event` output: the event name, then its
/// arguments separated by spaces. Unknown events are `None`.
pub fn parse_event(line: &str) -> Option<PowerEvent> {
    let mut it = line.split_whitespace();
    let name = it.next()?;
    let arg = it
        .next()
        .and_then(|a| a.trim_matches('"').parse::<i32>().ok())
        .unwrap_or(0);
    Some(match name {
        "goingToScreenSaver" => PowerEvent::GoingToScreenSaver(arg),
        "outOfScreenSaver" => PowerEvent::OutOfScreenSaver(arg),
        "exitingScreenSaver" => PowerEvent::ExitingScreenSaver,
        "readyToSuspend" => PowerEvent::ReadyToSuspend(arg),
        "suspending" => PowerEvent::Suspending,
        "wakeupFromSuspend" => PowerEvent::WakeupFromSuspend,
        "charging" => PowerEvent::Charging,
        "notCharging" => PowerEvent::NotCharging,
        _ => return None,
    })
}

/// powerd's events, from a `lipc-wait-event -m` child that a thread reads.
/// Dropping it stops the child.
pub struct PowerEvents {
    rx: Receiver<PowerEvent>,
    /// Readable whenever an event is queued (one byte per event).
    wake: OwnedFd,
    child: Arc<Mutex<ListenerChild>>,
    stop: Sender<()>,
    worker: Option<thread::JoinHandle<()>>,
}

#[derive(Default)]
struct ListenerChild {
    process: Option<Child>,
    stopped: bool,
}

impl ListenerChild {
    fn reap(&mut self) {
        if let Some(mut child) = self.process.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn spawn_waiter() -> io::Result<Child> {
    Command::new("lipc-wait-event")
        .args(["-m", "-s", "0", POWERD, EVENTS])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
}

impl PowerEvents {
    /// Start listening. Fails off the device (no `lipc-wait-event`). If the
    /// child dies later (lipcd restarting, say), the thread starts another
    /// after a pause.
    pub fn listen() -> io::Result<PowerEvents> {
        Self::listen_with(spawn_waiter, Duration::from_secs(5))
    }

    fn listen_with<F>(mut spawn: F, retry: Duration) -> io::Result<PowerEvents>
    where
        F: FnMut() -> io::Result<Child> + Send + 'static,
    {
        let mut fds = [0; 2];
        // SAFETY: pipe2 fills the two-element array on success.
        if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC | libc::O_NONBLOCK) } != 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: both descriptors were just created and are owned here.
        let (read, write) = unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) };
        let child = Arc::new(Mutex::new(ListenerChild {
            process: Some(spawn()?),
            stopped: false,
        }));
        let (tx, rx) = mpsc::channel();
        let (stop, halt) = mpsc::channel();
        let process = child.clone();
        let worker = match thread::Builder::new()
            .name("powerd-events".into())
            .spawn(move || read_events(tx, write, process, halt, spawn, retry))
        {
            Ok(worker) => worker,
            Err(error) => {
                child.lock().unwrap().reap();
                return Err(error);
            }
        };
        Ok(PowerEvents {
            rx,
            wake: read,
            child,
            stop,
            worker: Some(worker),
        })
    }

    /// The descriptor to poll with the touchscreen.
    pub fn fd(&self) -> RawFd {
        self.wake.as_raw_fd()
    }

    /// The events received since the last call.
    pub fn drain(&self) -> Vec<PowerEvent> {
        let mut buf = [0u8; 64];
        // SAFETY: reads into a local buffer from our own non-blocking pipe.
        while unsafe { libc::read(self.wake.as_raw_fd(), buf.as_mut_ptr().cast(), buf.len()) } > 0 {
        }
        self.rx.try_iter().collect()
    }
}

impl Drop for PowerEvents {
    fn drop(&mut self) {
        {
            let mut child = self.child.lock().unwrap();
            child.stopped = true;
            child.reap();
        }
        let _ = self.stop.send(());
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn read_events<F>(
    tx: Sender<PowerEvent>,
    wake: OwnedFd,
    child: Arc<Mutex<ListenerChild>>,
    stop: Receiver<()>,
    mut spawn: F,
    retry: Duration,
) where
    F: FnMut() -> io::Result<Child>,
{
    loop {
        let stdout = {
            let mut child = child.lock().unwrap();
            child.process.as_mut().and_then(|c| c.stdout.take())
        };
        if let Some(out) = stdout {
            for line in BufReader::new(out).lines() {
                let Ok(line) = line else { break };
                if let Some(ev) = parse_event(&line) {
                    if tx.send(ev).is_err() {
                        child.lock().unwrap().reap();
                        return;
                    }
                    // SAFETY: one byte to our nonblocking wake pipe; EAGAIN means it is already readable.
                    unsafe { libc::write(wake.as_raw_fd(), [1u8].as_ptr().cast(), 1) };
                }
            }
        }
        {
            let mut child = child.lock().unwrap();
            child.reap();
            if child.stopped {
                return;
            }
        }
        if !matches!(
            stop.recv_timeout(retry),
            Err(mpsc::RecvTimeoutError::Timeout)
        ) {
            return;
        }
        let mut child = child.lock().unwrap();
        if child.stopped {
            return;
        }
        match spawn() {
            Ok(process) => child.process = Some(process),
            Err(error) => eprintln!("aristotle: restarting powerd listener: {error}"),
        }
    }
}

/// `lipc-set-prop`, waiting for it. `-i` for integer values.
pub fn set_prop(service: &str, prop: &str, value: &str, int: bool) -> io::Result<()> {
    let mut cmd = Command::new("lipc-set-prop");
    if int {
        cmd.arg("-i");
    }
    let status = cmd
        .args([service, prop, value])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "lipc-set-prop {service} {prop} {value}: {status}"
        )))
    }
}

/// powerd's state (`active`, `screenSaver`, `readyToSuspend`, ...).
pub fn state() -> io::Result<String> {
    let out = Command::new("lipc-get-prop")
        .args(["-s", POWERD, "state"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()?;
    if !out.status.success() {
        return Err(io::Error::other(format!("lipc-get-prop: {}", out.status)));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Load (`true`) or unload a blanket module, such as `screensaver`, the
/// stock screensaver that would draw over the app. Waits.
pub fn blanket(load: bool, module: &str) -> io::Result<()> {
    set_prop(BLANKET, if load { "load" } else { "unload" }, module, false)
}

/// Requests to powerd that need no answer, run in order on a thread of
/// their own.
pub struct Lipc {
    tx: Sender<(String, String, String, bool)>,
}

impl Default for Lipc {
    fn default() -> Self {
        Self::start()
    }
}

impl Lipc {
    pub fn start() -> Lipc {
        let (tx, rx) = mpsc::channel::<(String, String, String, bool)>();
        let _ = thread::Builder::new().name("lipc".into()).spawn(move || {
            for (service, prop, value, int) in rx {
                if let Err(e) = set_prop(&service, &prop, &value, int) {
                    eprintln!("aristotle: {e}");
                }
            }
        });
        Lipc { tx }
    }

    fn send(&self, service: &str, prop: &str, value: &str, int: bool) {
        let _ = self
            .tx
            .send((service.into(), prop.into(), value.into(), int));
    }

    /// Restart powerd's idle timer (T1), as a touch on the stock UI does.
    pub fn touch_idle_timer(&self) {
        self.send(POWERD, "touchScreenSaverTimeout", "1", true);
    }

    /// The same as a press of the power button.
    pub fn power_button(&self) {
        self.send(POWERD, "powerButton", "1", true);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn listener(script: &'static str, retry: Duration) -> PowerEvents {
        PowerEvents::listen_with(
            move || {
                Command::new("sh")
                    .args(["-c", script])
                    .stdout(Stdio::piped())
                    .spawn()
            },
            retry,
        )
        .unwrap()
    }

    fn receive(events: &PowerEvents) -> Vec<PowerEvent> {
        let mut fd = libc::pollfd {
            fd: events.fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: poll receives one initialized descriptor for a bounded wait.
        assert_eq!(unsafe { libc::poll(&mut fd, 1, 2000) }, 1);
        events.drain()
    }

    #[test]
    fn dropping_listener_reaps_its_child() {
        let events = listener(
            "printf 'goingToScreenSaver 2\n'; exec sleep 60",
            Duration::from_secs(5),
        );
        let pid = events.child.lock().unwrap().process.as_ref().unwrap().id();
        assert_eq!(receive(&events), vec![PowerEvent::GoingToScreenSaver(2)]);
        drop(events);
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
    fn listener_restarts_after_exit() {
        let events = listener("printf 'outOfScreenSaver 1\n'", Duration::from_millis(10));
        assert!(receive(&events).contains(&PowerEvent::OutOfScreenSaver(1)));
        assert!(receive(&events).contains(&PowerEvent::OutOfScreenSaver(1)));
    }

    #[test]
    fn listener_retries_a_failed_restart() {
        let mut attempts = 0;
        let events = PowerEvents::listen_with(
            move || {
                let attempt = attempts;
                attempts += 1;
                if attempt == 1 {
                    return Err(io::Error::other("temporary listener failure"));
                }
                Command::new("sh")
                    .args(["-c", "printf 'charging\n'"])
                    .stdout(Stdio::piped())
                    .spawn()
            },
            Duration::from_millis(10),
        )
        .unwrap();
        assert!(receive(&events).contains(&PowerEvent::Charging));
        assert!(receive(&events).contains(&PowerEvent::Charging));
    }

    #[test]
    fn dropping_listener_interrupts_restart_delay() {
        let events = listener("printf 'readyToSuspend 5\n'", Duration::from_secs(60));
        assert_eq!(receive(&events), vec![PowerEvent::ReadyToSuspend(5)]);
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while events.child.lock().unwrap().process.is_some() {
            assert!(std::time::Instant::now() < deadline);
            thread::sleep(Duration::from_millis(1));
        }
        let started = std::time::Instant::now();
        drop(events);
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn missing_listener_does_not_start_a_worker() {
        let result = PowerEvents::listen_with(
            || Err(io::Error::new(io::ErrorKind::NotFound, "test listener")),
            Duration::from_secs(5),
        );
        assert!(matches!(result, Err(e) if e.kind() == io::ErrorKind::NotFound));
    }

    #[test]
    fn events() {
        assert_eq!(
            parse_event("goingToScreenSaver 2"),
            Some(PowerEvent::GoingToScreenSaver(2))
        );
        assert_eq!(
            parse_event("outOfScreenSaver 1\n"),
            Some(PowerEvent::OutOfScreenSaver(1))
        );
        assert_eq!(
            parse_event("readyToSuspend 5"),
            Some(PowerEvent::ReadyToSuspend(5))
        );
        assert_eq!(
            parse_event("exitingScreenSaver"),
            Some(PowerEvent::ExitingScreenSaver)
        );
        assert_eq!(
            parse_event("wakeupFromSuspend 1760000000"),
            Some(PowerEvent::WakeupFromSuspend)
        );
        assert_eq!(parse_event("battLevelChanged 80"), None);
        assert_eq!(parse_event(""), None);
    }
}
