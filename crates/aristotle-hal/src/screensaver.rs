// SPDX-License-Identifier: AGPL-3.0-or-later
//! Reversible ownership of powerd's screensaver and blanket's drawing module.

use std::io;

use crate::{power, powerd};

/// Restore the saved power policy on explicit release or on an error path.
pub struct Screensaver {
    state: State<Device>,
}

impl Screensaver {
    /// Unload blanket before allowing powerd to suspend, or hold suspend off.
    pub fn acquire(follow_powerd: bool) -> io::Result<Self> {
        Ok(Self {
            state: State::acquire(Device, follow_powerd)?,
        })
    }

    pub fn restore(&mut self) {
        self.state.restore();
    }
}

trait Control {
    fn prevented(&mut self) -> io::Result<bool>;
    fn prevent(&mut self, value: bool) -> io::Result<()>;
    fn blanket(&mut self, load: bool) -> io::Result<()>;
}

struct Device;

impl Control for Device {
    fn prevented(&mut self) -> io::Result<bool> {
        power::screensaver_prevented()
    }

    fn prevent(&mut self, value: bool) -> io::Result<()> {
        power::prevent_screensaver(value)
    }

    fn blanket(&mut self, load: bool) -> io::Result<()> {
        powerd::blanket(load, "screensaver")
    }
}

struct State<C: Control> {
    control: C,
    prevent_was: bool,
    restore_prevent: bool,
    reload: bool,
}

impl<C: Control> State<C> {
    fn acquire(mut control: C, follow_powerd: bool) -> io::Result<Self> {
        let prevent_was = control.prevented()?;
        let mut state = Self {
            control,
            prevent_was,
            restore_prevent: false,
            reload: false,
        };
        if follow_powerd {
            // A failed command may still have changed the module state.
            state.reload = true;
            state.control.blanket(false)?;
        }
        let prevent = !follow_powerd;
        if prevent_was != prevent {
            state.restore_prevent = true;
            state.control.prevent(prevent)?;
        }
        Ok(state)
    }

    fn restore(&mut self) {
        if self.reload {
            match self.control.blanket(true) {
                Ok(()) => self.reload = false,
                Err(error) => eprintln!("aristotle: restoring stock screensaver: {error}"),
            }
        }
        if self.restore_prevent {
            match self.control.prevent(self.prevent_was) {
                Ok(()) => self.restore_prevent = false,
                Err(error) => eprintln!("aristotle: restoring preventScreenSaver: {error}"),
            }
        }
    }
}

impl<C: Control> Drop for State<C> {
    fn drop(&mut self) {
        self.restore();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    #[derive(Clone)]
    struct Fake(Rc<RefCell<Calls>>);

    struct Calls {
        prevented: bool,
        log: Vec<&'static str>,
        fail: Option<&'static str>,
    }

    impl Fake {
        fn new(prevented: bool, fail: Option<&'static str>) -> Self {
            Self(Rc::new(RefCell::new(Calls {
                prevented,
                log: Vec::new(),
                fail,
            })))
        }

        fn call(&self, name: &'static str) -> io::Result<()> {
            let mut state = self.0.borrow_mut();
            state.log.push(name);
            if state.fail == Some(name) {
                state.fail = None;
                return Err(io::Error::other(name));
            }
            Ok(())
        }

        fn log(&self) -> Vec<&'static str> {
            self.0.borrow().log.clone()
        }
    }

    impl Control for Fake {
        fn prevented(&mut self) -> io::Result<bool> {
            self.call("read")?;
            Ok(self.0.borrow().prevented)
        }

        fn prevent(&mut self, value: bool) -> io::Result<()> {
            self.call(if value { "hold" } else { "release" })
        }

        fn blanket(&mut self, load: bool) -> io::Result<()> {
            self.call(if load { "load" } else { "unload" })
        }
    }

    #[test]
    fn managed_takeover_unloads_before_releasing_suspend() {
        let control = Fake::new(true, None);
        let state = State::acquire(control.clone(), true).unwrap();
        assert_eq!(control.log(), ["read", "unload", "release"]);
        drop(state);
        assert_eq!(control.log(), ["read", "unload", "release", "load", "hold"]);
    }

    #[test]
    fn unknown_original_policy_is_not_changed() {
        let control = Fake::new(false, Some("read"));
        assert!(State::acquire(control.clone(), true).is_err());
        assert_eq!(control.log(), ["read"]);
    }

    #[test]
    fn failed_unload_does_not_release_suspend() {
        let control = Fake::new(true, Some("unload"));
        assert!(State::acquire(control.clone(), true).is_err());
        assert_eq!(control.log(), ["read", "unload", "load"]);
    }

    #[test]
    fn failed_release_rolls_back_both_changes() {
        let control = Fake::new(true, Some("release"));
        assert!(State::acquire(control.clone(), true).is_err());
        assert_eq!(control.log(), ["read", "unload", "release", "load", "hold"]);
    }

    #[test]
    fn fallback_restores_policy_without_loading_blanket() {
        let control = Fake::new(false, None);
        drop(State::acquire(control.clone(), false).unwrap());
        assert_eq!(control.log(), ["read", "hold", "release"]);
    }

    #[test]
    fn failed_hold_restores_the_previous_policy() {
        let control = Fake::new(false, Some("hold"));
        assert!(State::acquire(control.clone(), false).is_err());
        assert_eq!(control.log(), ["read", "hold", "release"]);
    }

    #[test]
    fn startup_error_restores_managed_policy() {
        fn startup(control: Fake) -> io::Result<()> {
            let _state = State::acquire(control, true)?;
            Err(io::Error::other("reader startup failed"))
        }
        let control = Fake::new(true, None);
        assert!(startup(control.clone()).is_err());
        assert_eq!(control.log(), ["read", "unload", "release", "load", "hold"]);
    }

    #[test]
    fn unchanged_policy_needs_no_write() {
        let control = Fake::new(true, None);
        drop(State::acquire(control.clone(), false).unwrap());
        assert_eq!(control.log(), ["read"]);
    }

    #[test]
    fn explicit_restore_is_idempotent() {
        let control = Fake::new(false, None);
        let mut state = State::acquire(control.clone(), true).unwrap();
        state.restore();
        state.restore();
        drop(state);
        assert_eq!(control.log(), ["read", "unload", "load"]);
    }

    #[test]
    fn drop_retries_failed_restoration() {
        let control = Fake::new(true, None);
        let mut state = State::acquire(control.clone(), true).unwrap();
        control.0.borrow_mut().fail = Some("load");
        state.restore();
        drop(state);
        assert_eq!(
            control.log(),
            ["read", "unload", "release", "load", "hold", "load"]
        );
    }
}
