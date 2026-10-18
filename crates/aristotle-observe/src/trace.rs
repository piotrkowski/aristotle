// SPDX-License-Identifier: AGPL-3.0-or-later
//! Portable traces contain no framebuffer or book data.

use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};

use serde::{Deserialize, Serialize};

use crate::capture::{self, End};
use crate::kmsg::Update;
use crate::{report, Interaction};

#[derive(Debug, Serialize, Deserialize)]
pub struct Capture {
    schema_version: u32,
    interactions: Vec<Interaction>,
    updates: Vec<Update>,
    kernel_log_overruns: u64,
    #[serde(flatten)]
    end: End,
}

impl Capture {
    fn horizon_verified(&self) -> Option<bool> {
        self.end.captured_until_us.map(|until| {
            let release = self.interactions.last().map_or(0, |i| i.up_us);
            capture::drain(until, until, release, &self.updates, false) == capture::Drain::Complete
        })
    }

    pub fn new(
        interactions: Vec<Interaction>,
        updates: Vec<Update>,
        kernel_log_overruns: u64,
        end: End,
    ) -> Self {
        Self {
            schema_version: 1,
            interactions,
            updates,
            kernel_log_overruns,
            end,
        }
    }

    pub fn write(&self, path: &str) -> io::Result<()> {
        let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
        serde_json::to_writer_pretty(&mut file, self)?;
        writeln!(file)
    }

    fn read(reader: impl Read) -> io::Result<Self> {
        let capture: Self = serde_json::from_reader(reader)?;
        if capture.schema_version != 1 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "unsupported trace schema",
            ));
        }
        if capture.interactions.iter().any(|i| i.up_us < i.down_us)
            || capture
                .interactions
                .windows(2)
                .any(|p| p[1].down_us < p[0].up_us)
            || capture
                .updates
                .iter()
                .any(|u| u.end_us.is_some_and(|end| end < u.start_us))
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "trace timestamps are not monotonic",
            ));
        }
        if capture.end.drain_timed_out && capture.end.captured_until_us.is_none()
            || capture.end.captured_until_us.is_some_and(|until| {
                capture.interactions.iter().any(|i| i.up_us > until)
                    || capture
                        .updates
                        .iter()
                        .any(|u| u.start_us > until || u.end_us.is_some_and(|end| end > until))
            })
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "capture end precedes recorded events",
            ));
        }
        Ok(capture)
    }
}

pub fn replay(args: &[String]) -> io::Result<()> {
    let (path, json) = match args {
        [path] => (path, false),
        [path, flag] if flag == "--json" => (path, true),
        _ => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "expected replay TRACE [--json]",
            ))
        }
    };
    let capture = Capture::read(File::open(path)?)?;
    let rows = report::measure(
        &capture.interactions,
        &capture.updates,
        capture.kernel_log_overruns,
        capture.end,
    );
    let valid = capture.kernel_log_overruns == 0
        && !capture.end.drain_timed_out
        && capture.horizon_verified() != Some(false)
        && rows.iter().all(|row| row.valid)
        && rows
            .last()
            .is_none_or(|row| row.settlement_verified != Some(false));
    if json {
        let result = serde_json::json!({
            "schema_version": 1,
            "valid": valid,
            "kernel_log_overruns": capture.kernel_log_overruns,
            "captured_until_us": capture.end.captured_until_us,
            "drain_timed_out": capture.end.drain_timed_out,
            "capture_horizon_verified": capture.horizon_verified(),
            "measurements": rows,
        });
        let stdout = io::stdout();
        let mut out = stdout.lock();
        serde_json::to_writer_pretty(&mut out, &result)?;
        writeln!(out)?;
    } else {
        report::print(
            &capture.interactions,
            &capture.updates,
            capture.kernel_log_overruns,
            capture.end,
        );
    }
    if !valid {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "incomplete driver capture",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn capture() -> Capture {
        Capture::new(
            vec![Interaction {
                label: "turn".into(),
                down_us: 100,
                up_us: 200,
                x: 900,
                y: 800,
            }],
            vec![Update {
                start_us: 220,
                end_us: Some(800),
                waveform: "glkw16".into(),
                full: true,
                rect: (0, 0, 1072, 1448),
                taken_ms: Some(1),
            }],
            0,
            End {
                captured_until_us: Some(2_000_000),
                drain_timed_out: false,
            },
        )
    }

    #[test]
    fn round_trip_preserves_measurements_and_driver_evidence() {
        let bytes = serde_json::to_vec(&capture()).unwrap();
        let restored = Capture::read(bytes.as_slice()).unwrap();
        let rows = report::measure(
            &restored.interactions,
            &restored.updates,
            restored.kernel_log_overruns,
            restored.end,
        );
        assert_eq!(rows[0].page_done_us, Some(600));
        assert_eq!(rows[0].updates[0].waveform, "glkw16");
        assert!(rows[0].updates[0].full);
        let report = serde_json::to_value(&rows).unwrap();
        assert_eq!(report[0]["valid"], true);
    }

    #[test]
    fn rejects_unknown_schema_and_impossible_timestamps() {
        for change in 0..3 {
            let mut capture = capture();
            match change {
                0 => capture.schema_version = 2,
                1 => capture.interactions[0].up_us = 99,
                _ => capture.updates[0].end_us = Some(219),
            }
            assert!(Capture::read(serde_json::to_vec(&capture).unwrap().as_slice()).is_err());
        }
    }

    #[test]
    fn preserves_invalid_capture_status_and_null_metrics() {
        let mut capture = capture();
        capture.kernel_log_overruns = 1;
        let restored = Capture::read(serde_json::to_vec(&capture).unwrap().as_slice()).unwrap();
        let rows = report::measure(
            &restored.interactions,
            &restored.updates,
            restored.kernel_log_overruns,
            restored.end,
        );
        let json = serde_json::to_value(&rows).unwrap();
        assert_eq!(json[0]["valid"], false);
        assert!(json[0]["first_done_us"].is_null());
        assert!(json[0]["page_done_us"].is_null());
        assert!(json[0]["settled_us"].is_null());
    }

    #[test]
    fn legacy_traces_preserve_observed_values_without_claiming_settlement() {
        let mut json = serde_json::to_value(capture()).unwrap();
        json.as_object_mut().unwrap().remove("captured_until_us");
        json.as_object_mut().unwrap().remove("drain_timed_out");
        let legacy = Capture::read(serde_json::to_vec(&json).unwrap().as_slice()).unwrap();
        let rows = report::measure(&legacy.interactions, &legacy.updates, 0, legacy.end);
        assert_eq!(rows[0].first_done_us, Some(600));
        assert_eq!(rows[0].settled_us, Some(600));
        assert_eq!(rows[0].settlement_verified, None);
    }

    #[test]
    fn rejects_capture_end_before_raw_events() {
        let mut capture = capture();
        capture.end.captured_until_us = Some(799);
        assert!(Capture::read(serde_json::to_vec(&capture).unwrap().as_slice()).is_err());
    }

    #[test]
    fn final_horizon_covers_unassigned_raw_updates_and_pending_completions() {
        let mut capture = capture();
        capture.updates.push(Update {
            start_us: 3_000_000,
            end_us: Some(3_500_000),
            ..Default::default()
        });
        capture.end.captured_until_us = Some(4_000_000);
        let rows = report::measure(&capture.interactions, &capture.updates, 0, capture.end);
        assert_eq!(rows[0].settlement_verified, Some(true));
        assert_eq!(capture.horizon_verified(), Some(false));
        capture.end.captured_until_us = Some(5_000_000);
        assert_eq!(capture.horizon_verified(), Some(true));
        capture.updates.last_mut().unwrap().end_us = None;
        assert_eq!(capture.horizon_verified(), Some(false));
    }
}
