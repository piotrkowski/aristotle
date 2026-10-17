// SPDX-License-Identifier: AGPL-3.0-or-later
//! A dedicated display thread with latest-wins damage coalescing.
//!
//! # Why
//!
//! Measured on the PW4 (`aristotle-observe bench stream`): the Lab126 EPDC
//! driver effectively runs one update at a time. An update submitted while
//! another is still being displayed makes the submit ioctl block until the
//! earlier one finishes (about 120 ms for A2), whether or not the two regions
//! overlap. Streaming A2 updates therefore cost about two waveform durations
//! each, and any code that submits from its input loop stalls that loop.
//!
//! Neither deeper queues nor tile-sized updates help (both measured; tiles
//! made it far worse, since every tile is another blocking submit).
//!
//! # How
//!
//! Callers draw into the framebuffer as usual and hand the damaged rectangle
//! to [`Display::damage`], which never blocks. A display thread loops: take
//! everything damaged since its last submit as one rectangle, submit it (this
//! is where it blocks while the panel is busy), repeat. Pixels drawn while
//! the panel was busy go out together in the next update. A point therefore
//! waits for at most the rest of the current update plus its own.
//!
//! [`Completions`] records when each update reaches the panel, for latency
//! measurement without a second waiter fighting over markers.

use std::io;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;

use crate::framebuffer::{Framebuffer, Rect, Waveform};
use crate::input::monotonic_us;

/// Reports finished EPDC updates without blocking the caller.
///
/// A helper thread waits on each tracked marker in order and sends it back,
/// with the CLOCK_MONOTONIC time it completed. Only one waiter may wait on a
/// given marker: the driver forgets a marker once it has signalled it.
pub struct Completions {
    submit: Sender<u32>,
    done: Receiver<(u32, u64)>,
}

impl Completions {
    pub fn new(fb: &Framebuffer) -> io::Result<Self> {
        let waiter = fb.waiter()?;
        let (submit, pending) = mpsc::channel::<u32>();
        let (finished, done) = mpsc::channel::<(u32, u64)>();
        thread::Builder::new()
            .name("epdc-completions".into())
            .spawn(move || {
                for marker in pending {
                    // An error means the driver forgot the marker; report it anyway
                    // so nothing waits on it forever.
                    let _ = waiter.wait(marker);
                    if finished.send((marker, monotonic_us())).is_err() {
                        break;
                    }
                }
            })?;
        Ok(Self { submit, done })
    }

    /// Start tracking `marker`.
    pub fn track(&self, marker: u32) {
        let _ = self.submit.send(marker);
    }

    /// A sender that tracks markers from another thread.
    pub fn tracker(&self) -> Sender<u32> {
        self.submit.clone()
    }

    /// `(marker, completed_at_us)` for updates that finished since the last call.
    pub fn drain(&self) -> impl Iterator<Item = (u32, u64)> + '_ {
        self.done.try_iter()
    }
}

/// Damage accumulated since the display thread's last submit.
#[derive(Default)]
struct Pending {
    rect: Option<Rect>,
    waveform: Option<Waveform>,
    /// Drive every pixel of the merged rect, not only changed ones; set if
    /// any merged damage asked for it (see [`Display::damage_full`]).
    full: bool,
    /// Caller-supplied timestamps carried along for latency accounting.
    tags: Vec<u64>,
    /// Submitted updates whose tags have not been reported as shown yet.
    in_flight: std::collections::HashMap<u32, Vec<u64>>,
    /// True while the display thread is inside the submit ioctl.
    submitting: bool,
    shutdown: bool,
}

/// Rank waveforms so merged damage uses the most faithful one requested.
fn fidelity(w: Waveform) -> u32 {
    match w {
        Waveform::A2 => 0,
        Waveform::DU => 1,
        Waveform::GLR16 | Waveform::GLKW16 => 3,
        Waveform::GC16 | Waveform::GCK16 => 4,
        _ => 2,
    }
}

/// See the module documentation.
pub struct Display {
    shared: Arc<(Mutex<Pending>, Condvar)>,
    thread: Option<thread::JoinHandle<()>>,
    completions: Completions,
}

impl Display {
    pub fn new(fb: &Framebuffer) -> io::Result<Self> {
        let mut updater = fb.updater()?;
        let completions = Completions::new(fb)?;
        let tracker = completions.tracker();
        let shared = Arc::new((Mutex::new(Pending::default()), Condvar::new()));
        let worker = Arc::clone(&shared);
        let thread = thread::Builder::new()
            .name("epdc-display".into())
            .spawn(move || {
                let (lock, cvar) = &*worker;
                loop {
                    let (rect, waveform, full, tags) = {
                        let mut p = lock.lock().expect("display lock");
                        while p.rect.is_none() && !p.shutdown {
                            p = cvar.wait(p).expect("display lock");
                        }
                        if p.rect.is_none() && p.shutdown {
                            return;
                        }
                        p.submitting = true;
                        (
                            p.rect.take().unwrap(),
                            p.waveform.take().unwrap_or(Waveform::A2),
                            std::mem::take(&mut p.full),
                            std::mem::take(&mut p.tags),
                        )
                    };
                    // Blocks while the panel finishes the previous update.
                    let marker = updater.update(rect, waveform, full).ok().flatten();
                    let mut p = lock.lock().expect("display lock");
                    p.submitting = false;
                    if let Some(m) = marker {
                        p.in_flight.insert(m, tags);
                        let _ = tracker.send(m);
                    }
                    cvar.notify_all();
                }
            })?;
        Ok(Self {
            shared,
            thread: Some(thread),
            completions,
        })
    }

    /// Queue `rect` for display with `waveform`; never blocks on the panel.
    /// `tags` (for example input timestamps) come back from
    /// [`Display::drain_shown`] once the update carrying them is on the panel.
    pub fn damage(&self, rect: Rect, waveform: Waveform, tags: &[u64]) {
        self.queue(rect, waveform, false, tags);
    }

    /// Like [`Display::damage`], but the update drives every pixel in the
    /// merged rect. Needed to clear ghosting: GC16 flash refreshes, and the
    /// final GL16 pass of a page turn after the fast A2 pass. With a partial
    /// update the driver skips pixels that already hold their target value,
    /// and those are exactly where the previous page's ghost remains.
    pub fn damage_full(&self, rect: Rect, waveform: Waveform, tags: &[u64]) {
        self.queue(rect, waveform, true, tags);
    }

    fn queue(&self, rect: Rect, waveform: Waveform, full: bool, tags: &[u64]) {
        let (lock, cvar) = &*self.shared;
        let mut p = lock.lock().expect("display lock");
        p.full |= full;
        p.rect = Some(p.rect.map_or(rect, |r| r.union(rect)));
        p.waveform = Some(match p.waveform {
            Some(w) if fidelity(w) >= fidelity(waveform) => w,
            _ => waveform,
        });
        p.tags.extend_from_slice(tags);
        cvar.notify_all();
    }

    /// Updates that reached the panel since the last call, as
    /// `(completed_at_us, tags)`.
    pub fn drain_shown(&self) -> Vec<(u64, Vec<u64>)> {
        let done: Vec<(u32, u64)> = self.completions.drain().collect();
        if done.is_empty() {
            return Vec::new();
        }
        let (lock, _) = &*self.shared;
        let mut p = lock.lock().expect("display lock");
        done.into_iter()
            .filter_map(|(m, at)| p.in_flight.remove(&m).map(|tags| (at, tags)))
            .collect()
    }

    /// True while damage is queued, being submitted, or not yet on the panel.
    pub fn busy(&self) -> bool {
        let (lock, _) = &*self.shared;
        let p = lock.lock().expect("display lock");
        p.rect.is_some() || p.submitting || !p.in_flight.is_empty()
    }

    /// Block until everything queued so far has been submitted.
    pub fn sync(&self) {
        let (lock, cvar) = &*self.shared;
        let mut p = lock.lock().expect("display lock");
        while p.rect.is_some() || p.submitting {
            p = cvar.wait(p).expect("display lock");
        }
    }
}

impl Drop for Display {
    fn drop(&mut self) {
        {
            let (lock, cvar) = &*self.shared;
            lock.lock().expect("display lock").shutdown = true;
            cvar.notify_all();
        }
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}
