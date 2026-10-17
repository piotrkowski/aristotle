// SPDX-License-Identifier: AGPL-3.0-or-later
//! `aristotle-observe bench`: EPDC benchmarks that draw on the panel
//! themselves, with the stock GUI stopped (`takeover.sh start`).
//!
//! ```text
//! aristotle-observe bench latency                    one update per waveform; parallel and queued updates
//! aristotle-observe bench stream [waveforms]         a steady stream of small updates
//! aristotle-observe bench pageturn PAGE.raw [TURNS] [PAUSE_MS] [STRATEGY]
//! ```
//!
//! The results are in `docs/eink-rendering.md` and
//! `docs/native-vs-aristotle.md`.

mod latency;
mod pageturn;
mod stream;

use std::io;

pub const USAGE: &str = "bench latency | bench stream [waveforms] | \
     bench pageturn PAGE.raw [TURNS] [PAUSE_MS] [STRATEGY]";

/// Run the benchmark named by `args[0]` with the rest of `args`.
pub fn run(args: &[String]) -> io::Result<()> {
    let rest = args.get(1..).unwrap_or_default();
    match args.first().map(String::as_str) {
        Some("latency") => latency::run(),
        Some("stream") => stream::run(rest),
        Some("pageturn") => pageturn::run(rest),
        _ => {
            eprintln!("usage: aristotle-observe {USAGE}");
            std::process::exit(2);
        }
    }
}
