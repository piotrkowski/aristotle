// SPDX-License-Identifier: AGPL-3.0-or-later
//! Follow stock powerd by default; ARISTOTLE_POWERD=0 selects the fallback.
//!
//! Successful takeover unloads blanket's screensaver and releases preventScreenSaver.
//! Firmware readiness checks still apply to the power button.
//! powerd's `goingToScreenSaver` (power button, cover, its idle timer, or
//! our own timer pressing the button through `powerButton`) makes the app
//! save and draw its screensaver card; powerd then suspends the CPU a minute
//! later with the card on the panel. `outOfScreenSaver` (the button again)
//! brings the screen back. Touch does not wake managed sleep.
//!
//! The app's "sleep after" setting can exceed powerd's own idle timer (T1,
//! 600 s on our unit [dev]), so the app restarts T1 after touches (at most
//! once a minute) and every five minutes while awake, and presses the
//! button itself when its own time is up. "Never" keeps restarting T1.
//!
//! When powerd does not react, the app falls back to its own screensaver without suspend.
//! A touch wakes that fallback; managed sleep supplies KAF readiness after takeover.
//! Firmware drive_mode_state=1 means off, not a USB-network suspend blocker.

use std::os::fd::RawFd;
use std::time::{Duration, Instant};

use aristotle_hal::powerd::{self as hal, Lipc, PowerEvent, PowerEvents};
use aristotle_hal::screensaver::Screensaver;

use crate::perf;

/// How long to wait for `goingToScreenSaver` after pressing the button for
/// our own timer.
const SLEEP_WAIT: Duration = Duration::from_secs(5);
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
    /// After a wake powerd did not confirm: check its state then.
    check_at: Option<Instant>,
    resync_sent: bool,
}

impl Sleep {
    /// Follow powerd unless explicitly disabled; a failed takeover aborts startup.
    /// ARISTOTLE_POWERD=0 retains the non-suspending screensaver fallback.
    pub fn start() -> std::io::Result<Sleep> {
        let wanted = enabled(std::env::var_os("ARISTOTLE_POWERD").as_deref());
        let (events, screensaver) = if wanted {
            let events = PowerEvents::listen()?;
            let screensaver = Screensaver::acquire(true)?;
            perf!("sleep: following powerd's events");
            (Some(events), screensaver)
        } else {
            eprintln!(
                "aristotle: ARISTOTLE_POWERD=0 disables CPU suspend; touch wakes the fallback"
            );
            (None, Screensaver::acquire(false)?)
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
            check_at: None,
            resync_sent: false,
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
            check_at: None,
            resync_sent: false,
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
        self.resync_sent = false;
        self.powerd_asleep = by_powerd;
        self.asked_at = None;
    }

    /// powerd went to its screensaver over one of ours: it may suspend now.
    pub fn powerd_took_over(&mut self) {
        self.powerd_asleep = true;
    }

    /// Touch wakes only the non-suspending fallback.
    pub fn touch_asleep(&mut self, _now: Instant) -> bool {
        !self.powerd_asleep
    }

    /// The screen is back; `by_powerd` after `outOfScreenSaver`.
    pub fn awake(&mut self, by_powerd: bool, now: Instant) {
        if self.powerd_asleep && !by_powerd {
            // powerd may still be in its screensaver and suspend us.
            self.check_at = Some(now + Duration::from_millis(2500));
        }
        self.powerd_asleep = false;
        self.asked_at = None;
        self.touched = true;
    }

    /// powerd is about to suspend while the app is awake: it missed a wake.
    pub fn resync(&mut self) {
        if self.resync_sent {
            return;
        }
        self.resync_sent = true;
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
            return None;
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

fn enabled(value: Option<&std::ffi::OsStr>) -> bool {
    value != Some(std::ffi::OsStr::new("0"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn managed_sleep_is_default_with_an_explicit_zero_opt_out() {
        assert!(enabled(None));
        assert!(enabled(Some(std::ffi::OsStr::new("1"))));
        assert!(!enabled(Some(std::ffi::OsStr::new("0"))));
    }
}
