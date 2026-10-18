// SPDX-License-Identifier: AGPL-3.0-or-later
//! Sleep the stock way: the app follows `powerd` instead of holding it off.
//!
//! While the app runs, blanket's screensaver module is unloaded, so nothing
//! draws over it, and `preventScreenSaver` is 0, so the power button works.
//! powerd's `goingToScreenSaver` (power button, cover, its idle timer, or
//! our own timer pressing the button through `powerButton`) makes the app
//! save and draw its screensaver card; powerd then suspends the CPU a minute
//! later with the card on the panel. `outOfScreenSaver` (the button again)
//! brings the screen back. A touch on the card asks powerd to wake as well.
//!
//! The app's "sleep after" setting can exceed powerd's own idle timer (T1,
//! 600 s on our unit [dev]), so the app restarts T1 after touches (at most
//! once a minute) and every five minutes while awake, and presses the
//! button itself when its own time is up. "Never" keeps restarting T1.
//!
//! When powerd does not react, the app falls back to its own screensaver without suspend.
//! A touch wakes that fallback; the ignored button's cause needs device validation.
//! Firmware drive_mode_state=1 means off, not a USB-network suspend blocker.

use std::os::fd::RawFd;
use std::time::{Duration, Instant};

use aristotle_hal::powerd::{self as hal, Lipc, PowerEvent, PowerEvents};
use aristotle_hal::screensaver::Screensaver;

use crate::perf;

/// How long to wait for `goingToScreenSaver` after pressing the button for
/// our own timer.
const SLEEP_WAIT: Duration = Duration::from_secs(5);
/// How long a touch on powerd's screensaver waits for `outOfScreenSaver`.
/// powerd drops presses closer than 2 s to the previous one [dev].
const WAKE_WAIT: Duration = Duration::from_secs(3);
/// T1 restarts: after a touch at most this often...
const PING_TOUCHED: Duration = Duration::from_secs(60);
/// ... and at least this often while awake (T1 is 600 s on our unit).
const PING_ALWAYS: Duration = Duration::from_secs(300);

/// What the loop should do after [`Sleep::tick`].
#[derive(Debug, PartialEq, Eq)]
pub enum Action {
    None,
    /// Show the screensaver without powerd (it did not react).
    Sleep,
    /// Leave the screensaver without powerd (it did not react).
    Wake,
}

pub struct Sleep {
    events: Option<PowerEvents>,
    lipc: Lipc,
    /// Restores stock screensaver state on every exit path.
    screensaver: Option<Screensaver>,
    last_ping: Instant,
    touched: bool,
    /// The button pressed for our own timer, waiting for powerd.
    asked_at: Option<Instant>,
    /// The screensaver on the panel is powerd's (it will suspend).
    powerd_asleep: bool,
    /// A touch asked powerd to leave its screensaver.
    wake_asked_at: Option<Instant>,
    /// After a wake powerd did not confirm: check its state then.
    check_at: Option<Instant>,
}

impl Sleep {
    /// Follow powerd and take the screensaver over from blanket. Off the
    /// device, without `lipc-wait-event`, or unless `ARISTOTLE_POWERD=1`
    /// is set, hold `preventScreenSaver` instead, as earlier versions did:
    /// suspending through powerd is not yet verified on the device, so it
    /// is opt-in for now.
    pub fn start() -> std::io::Result<Sleep> {
        let wanted = std::env::var_os("ARISTOTLE_POWERD").is_some_and(|v| v == "1");
        let listen = if wanted {
            PowerEvents::listen()
        } else {
            Err(std::io::Error::other("ARISTOTLE_POWERD=1 not set"))
        };
        let (events, screensaver) = match listen {
            Ok(ev) => match Screensaver::acquire(true) {
                Ok(state) => {
                    perf!("sleep: following powerd's events");
                    (Some(ev), state)
                }
                Err(error) => {
                    eprintln!("aristotle: can't take over powerd's screensaver: {error}");
                    drop(ev);
                    (None, Screensaver::acquire(false)?)
                }
            },
            Err(e) => {
                eprintln!("aristotle: no powerd events ({e}); holding off its screensaver");
                (None, Screensaver::acquire(false)?)
            }
        };
        Ok(Sleep {
            events,
            lipc: Lipc::start(),
            screensaver: Some(screensaver),
            // Restart T1 on the first tick. (Early in a boot the monotonic
            // clock may be below PING_ALWAYS.)
            last_ping: Instant::now()
                .checked_sub(PING_ALWAYS)
                .unwrap_or_else(Instant::now),
            touched: false,
            asked_at: None,
            powerd_asleep: false,
            wake_asked_at: None,
            check_at: None,
        })
    }

    /// A Sleep for the host preview: no powerd, nothing changed.
    pub fn none() -> Sleep {
        Sleep {
            events: None,
            lipc: Lipc::start(),
            screensaver: None,
            last_ping: Instant::now(),
            touched: false,
            asked_at: None,
            powerd_asleep: false,
            wake_asked_at: None,
            check_at: None,
        }
    }

    fn managed(&self) -> bool {
        self.events.is_some()
    }

    /// The descriptor to poll with the touchscreen.
    pub fn fd(&self) -> Option<RawFd> {
        self.events.as_ref().map(|e| e.fd())
    }

    /// powerd's events since the last call.
    pub fn events(&self) -> Vec<PowerEvent> {
        self.events.as_ref().map(|e| e.drain()).unwrap_or_default()
    }

    /// Whether the button was pressed for our timer and powerd has not
    /// answered yet.
    pub fn sleep_pending(&self) -> bool {
        self.asked_at.is_some()
    }

    /// A touch while awake.
    pub fn touched(&mut self) {
        self.touched = true;
    }

    /// Our own idle time is up. True: show the screensaver now (no powerd);
    /// false: wait for `goingToScreenSaver`.
    pub fn request_sleep(&mut self, now: Instant) -> bool {
        if !self.managed() {
            return true;
        }
        if self.asked_at.is_none() {
            perf!("sleep: idle time up, pressing powerd's button");
            self.lipc.power_button();
            self.asked_at = Some(now);
        }
        false
    }

    /// The screensaver is on the panel; `by_powerd` when powerd put it
    /// there.
    pub fn asleep(&mut self, by_powerd: bool) {
        self.powerd_asleep = by_powerd;
        self.asked_at = None;
        self.wake_asked_at = None;
    }

    /// powerd went to its screensaver over one of ours: it may suspend now.
    pub fn powerd_took_over(&mut self) {
        self.powerd_asleep = true;
    }

    /// A touch on the screensaver. True: wake now (the screensaver is our
    /// own); false: powerd was asked, wait for `outOfScreenSaver`.
    pub fn touch_asleep(&mut self, now: Instant) -> bool {
        if !self.powerd_asleep {
            return true;
        }
        if self.wake_asked_at.is_none() {
            self.lipc.power_button();
            self.wake_asked_at = Some(now);
        }
        false
    }

    /// The screen is back; `by_powerd` after `outOfScreenSaver`.
    pub fn awake(&mut self, by_powerd: bool, now: Instant) {
        if self.powerd_asleep && !by_powerd {
            // powerd may still be in its screensaver and suspend us.
            self.check_at = Some(now + Duration::from_millis(2500));
        }
        self.powerd_asleep = false;
        self.wake_asked_at = None;
        self.asked_at = None;
        self.touched = true;
    }

    /// powerd is about to suspend while the app is awake: it missed a wake.
    pub fn resync(&mut self) {
        perf!("sleep: powerd wants to suspend while awake; pressing its button");
        self.lipc.power_button();
    }

    /// Work due now: restarting powerd's idle timer while awake, and the
    /// fallbacks when powerd does not answer.
    pub fn tick(&mut self, now: Instant, asleep: bool) -> Action {
        if !self.managed() {
            return Action::None;
        }
        if asleep {
            if self
                .wake_asked_at
                .is_some_and(|t| now.duration_since(t) >= WAKE_WAIT)
            {
                perf!("sleep: powerd did not wake; waking without it");
                return Action::Wake;
            }
            return Action::None;
        }
        if self
            .asked_at
            .is_some_and(|t| now.duration_since(t) >= SLEEP_WAIT)
        {
            self.asked_at = None;
            perf!("sleep: powerd ignored the button; screensaver without suspend");
            return Action::Sleep;
        }
        if self.check_at.is_some_and(|t| now >= t) {
            self.check_at = None;
            match hal::state() {
                Ok(s) if s != "active" => {
                    perf!("sleep: powerd still in state {s}; pressing its button");
                    self.lipc.power_button();
                }
                _ => {}
            }
        }
        let since = now.duration_since(self.last_ping);
        if (self.touched && since >= PING_TOUCHED) || since >= PING_ALWAYS {
            self.lipc.touch_idle_timer();
            self.last_ping = now;
            self.touched = false;
        }
        Action::None
    }

    /// The next time [`tick`](Self::tick) has work.
    pub fn deadline(&self, asleep: bool) -> Option<Instant> {
        if !self.managed() {
            return None;
        }
        if asleep {
            return self.wake_asked_at.map(|t| t + WAKE_WAIT);
        }
        let ping = self.last_ping
            + if self.touched {
                PING_TOUCHED
            } else {
                PING_ALWAYS
            };
        [
            self.asked_at.map(|t| t + SLEEP_WAIT),
            self.check_at,
            Some(ping),
        ]
        .into_iter()
        .flatten()
        .min()
    }

    /// Give the screensaver back to the stock system: blanket's module, and
    /// `preventScreenSaver` as it was.
    pub fn restore(&mut self) {
        self.events.take();
        self.lipc.shutdown();
        if let Some(state) = &mut self.screensaver {
            state.restore();
        }
    }
}

impl Drop for Sleep {
    fn drop(&mut self) {
        self.restore();
    }
}
