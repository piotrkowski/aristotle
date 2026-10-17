//! Parsing the Lab126 EPDC driver's debug log from `/dev/kmsg`.
//!
//! Writing 1 to `/sys/devices/soc0/soc/2000000.aips-bus/20f4000.epdc/mxc_epdc_debug`
//! makes the stock driver log every update, for example:
//!
//! ```text
//! mxc_epdc_fb: [1] update start marker=1, start time=1792229772592
//! mxc_epdc_fb: [1] waveform=0x2 (gc16) mode=0x1 temp:4096 update region top=0, left=0, width=1072, height=1448 flags=0x0
//! mxc_epdc_fb: [1] update end marker=1, end time=1792229773163, time taken=571 ms
//! ```
//!
//! `/dev/kmsg` records carry the kernel timestamp in microseconds
//! (`prio,seq,usec,flags;message`). That clock is the kernel's local clock,
//! which we calibrate against CLOCK_MONOTONIC by writing our own marker line.

use std::collections::HashMap;

pub const EPDC_DEBUG: &str = "/sys/devices/soc0/soc/2000000.aips-bus/20f4000.epdc/mxc_epdc_debug";

#[derive(Clone, Debug, Default)]
pub struct Update {
    pub start_us: u64,
    pub end_us: Option<u64>,
    pub waveform: String,
    /// `mode=0x1` is a full update.
    pub full: bool,
    pub rect: (u32, u32, u32, u32),
    /// Duration as printed by the driver.
    pub taken_ms: Option<u32>,
}

impl Update {
    pub fn area(&self) -> u64 {
        self.rect.2 as u64 * self.rect.3 as u64
    }

    /// A flashing refresh: GC16 or GCK16 driven as a full update.
    pub fn flashes(&self) -> bool {
        self.full && (self.waveform.starts_with("gc16") || self.waveform.starts_with("gck16"))
    }
}

/// Split one `/dev/kmsg` record into (timestamp_us, message).
pub fn parse_record(record: &str) -> Option<(u64, &str)> {
    let (header, message) = record.split_once(';')?;
    let mut fields = header.split(',');
    fields.next()?; // priority
    fields.next()?; // sequence
    let usec = fields.next()?.parse().ok()?;
    Some((usec, message.trim_end()))
}

fn field<'a>(message: &'a str, key: &str) -> Option<&'a str> {
    let start = message.find(key)? + key.len();
    let rest = &message[start..];
    let end = rest.find([',', ' ']).unwrap_or(rest.len());
    Some(&rest[..end])
}

/// Accumulates driver log lines into [`Update`]s.
#[derive(Default)]
pub struct UpdateLog {
    open: HashMap<i64, Update>,
    pub done: Vec<Update>,
}

impl UpdateLog {
    pub fn feed(&mut self, ts_us: u64, message: &str) {
        // The Lab126 kernel puts the printk time ("[ 5936.322425] ") inside
        // the message text itself, so search instead of matching a prefix.
        let Some(at) = message.find("mxc_epdc_fb: [") else {
            return;
        };
        let rest = &message[at + "mxc_epdc_fb: [".len()..];
        let Some((id, body)) = rest.split_once("] ") else {
            return;
        };
        // Updates without a marker of their own (our partial updates) get a
        // driver-assigned one with the top bit set, printed as negative.
        let Ok(id) = id.parse::<i64>() else { return };
        if body.starts_with("update start") {
            // `[N]` is the driver's own update sequence number, used only to
            // join the start, detail and end lines of one update.
            self.open.insert(
                id,
                Update {
                    start_us: ts_us,
                    ..Default::default()
                },
            );
        } else if body.starts_with("waveform=") {
            if let Some(u) = self.open.get_mut(&id) {
                u.waveform = body
                    .split_once('(')
                    .and_then(|(_, r)| r.split_once(')'))
                    .map(|(name, _)| name.to_string())
                    .unwrap_or_default();
                u.full = field(body, "mode=") == Some("0x1");
                let num = |k| field(body, k).and_then(|v| v.parse().ok()).unwrap_or(0);
                u.rect = (num("top="), num("left="), num("width="), num("height="));
            }
        } else if body.starts_with("update end") {
            if let Some(mut u) = self.open.remove(&id) {
                u.end_us = Some(ts_us);
                u.taken_ms = field(body, "taken=").and_then(|v| v.parse().ok());
                self.done.push(u);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_driver_update() {
        let mut log = UpdateLog::default();
        log.feed(100, "mxc_epdc_fb: [7] update start marker=3, start time=1");
        log.feed(101, "mxc_epdc_fb: [7] waveform=0x2 (gc16) mode=0x1 temp:4096 update region top=0, left=0, width=1072, height=1448 flags=0x0");
        log.feed(
            700,
            "mxc_epdc_fb: [7] update end marker=3, end time=2, time taken=571 ms",
        );
        let u = &log.done[0];
        assert_eq!(
            (u.start_us, u.end_us, u.taken_ms),
            (100, Some(700), Some(571))
        );
        assert_eq!(u.waveform, "gc16");
        assert!(u.full && u.flashes());
        assert_eq!(u.rect, (0, 0, 1072, 1448));
    }

    #[test]
    fn tolerates_the_lab126_time_prefix() {
        let mut log = UpdateLog::default();
        log.feed(
            1,
            "[ 5936.322425] mxc_epdc_fb: [320] update start marker=320, start time=1",
        );
        log.feed(2, "[ 5936.322437] mxc_epdc_fb: [320] waveform=0x101 (auto) mode=0x0 temp:4096 update region top=10, left=951, width=75, height=37 flags=0x0");
        log.feed(3, "[ 5936.771882] mxc_epdc_fb: [320] update end marker=320, end time=2, time taken=449 ms");
        assert_eq!(log.done.len(), 1);
        assert_eq!(log.done[0].rect, (10, 951, 75, 37));
        assert_eq!(log.done[0].waveform, "auto");
    }

    #[test]
    fn joins_updates_with_negative_markers() {
        let mut log = UpdateLog::default();
        log.feed(1, "[ 3145.087017] mxc_epdc_fb: [-2147483573] update start marker=-2147483573, start time=1");
        log.feed(2, "[ 3145.087034] mxc_epdc_fb: [-2147483573] waveform=0x6 (a2) mode=0x0 temp:4096 update region top=65, left=839, width=162, height=106 flags=0x0");
        log.feed(3, "[ 3145.207175] mxc_epdc_fb: [-2147483573] update end marker=2147483723, end time=2, time taken=120 ms");
        assert_eq!(log.done.len(), 1);
        assert_eq!(log.done[0].waveform, "a2");
        assert!(!log.done[0].full);
    }

    #[test]
    fn parses_kmsg_record() {
        assert_eq!(
            parse_record("6,1234,5707920979,-;mxc_epdc_fb: x\n"),
            Some((5707920979, "mxc_epdc_fb: x"))
        );
    }
}
