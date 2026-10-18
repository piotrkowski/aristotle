// SPDX-License-Identifier: AGPL-3.0-or-later
//! Acquisition limits and evidence for the final quiet window.

use crate::{kmsg::Update, SETTLE_US};

pub const DRAIN_BUDGET_US: u64 = 10_000_000;

#[derive(Clone, Copy, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct End {
    pub captured_until_us: Option<u64>,
    #[serde(default)]
    pub drain_timed_out: bool,
}

#[derive(Debug, PartialEq)]
pub enum Drain {
    Waiting,
    Complete,
    TimedOut,
}

pub fn drain(
    now: u64,
    started: u64,
    release: u64,
    updates: &[Update],
    contact_open: bool,
) -> Drain {
    let mut deadline = release.saturating_add(SETTLE_US);
    let mut pending = contact_open;
    for update in updates {
        pending |= update.end_us.is_none();
        deadline = deadline.max(
            update
                .end_us
                .unwrap_or(update.start_us)
                .saturating_add(SETTLE_US),
        );
    }
    if !pending && now >= deadline {
        Drain::Complete
    } else if now.saturating_sub(started) >= DRAIN_BUDGET_US {
        Drain::TimedOut
    } else {
        Drain::Waiting
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn update(start: u64, end: Option<u64>) -> Update {
        Update {
            start_us: start,
            end_us: end,
            ..Default::default()
        }
    }

    #[test]
    fn waits_for_the_actual_rolling_quiet_window() {
        let mut updates = vec![
            update(100_000, Some(700_000)),
            update(1_300_000, Some(1_900_000)),
        ];
        assert_eq!(drain(2_000_000, 0, 0, &updates, false), Drain::Waiting);
        updates.push(update(2_500_000, Some(3_000_000)));
        assert_eq!(drain(3_400_000, 0, 0, &updates, false), Drain::Waiting);
        assert_eq!(drain(4_500_000, 0, 0, &updates, false), Drain::Complete);
    }

    #[test]
    fn pending_updates_and_contacts_wait_until_the_bounded_timeout() {
        let updates = [update(100_000, None)];
        assert_eq!(drain(3_000_000, 0, 0, &updates, false), Drain::Waiting);
        assert_eq!(
            drain(DRAIN_BUDGET_US, 0, 0, &updates, false),
            Drain::TimedOut
        );
        assert_eq!(drain(DRAIN_BUDGET_US, 0, 0, &[], true), Drain::TimedOut);
    }

    #[test]
    fn continuing_completed_updates_also_exhaust_the_budget() {
        let updates = [update(9_000_000, Some(9_500_000))];
        assert_eq!(
            drain(DRAIN_BUDGET_US, 0, 0, &updates, false),
            Drain::TimedOut
        );
    }
}
