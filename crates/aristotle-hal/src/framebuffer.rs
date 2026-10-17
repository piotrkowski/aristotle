//! E-ink framebuffer on the i.MX EPDC, using Lab126's Rex update ABI.
//!
//! # Model
//!
//! Drawing and displaying are separate steps on e-ink:
//!
//! 1. Write pixels into the memory-mapped framebuffer ([`Framebuffer::fill`],
//!    [`Framebuffer::line`]). Nothing changes on the panel yet.
//! 2. Ask the EPDC to drive a rectangle to the new content with a chosen
//!    [`Waveform`] ([`Framebuffer::update`]). The call returns in about 1.4 ms;
//!    the panel then takes 144 ms (A2) to 544 ms (GC16) to settle.
//! 3. Optionally block until that update is on the panel ([`Framebuffer::wait`]).
//!
//! Step 2 is less cheap than it looks: the driver runs one update at a time,
//! and submitting while an earlier update is still on the panel blocks until
//! it is done. Interactive code should therefore submit from the display
//! thread in [`crate::display`] and never from its input path. See
//! `docs/eink-rendering.md`.
//!
//! Coordinates are framebuffer pixels in portrait orientation: (0, 0) is the
//! top-left corner and the visible area is 1072 × 1448 on the PW4. Gray levels
//! are 8-bit, with 0 black and 255 white.
//!
//! # ABI
//!
//! The update struct layout comes from FBInk's `eink/mxcfb-kindle.h`
//! (`mxcfb_update_data_rex`, 80 bytes, checked at compile time below). Plato's
//! Kobo struct has a different field order and size and must not be reused.

use std::fs::{File, OpenOptions};
use std::io;
use std::os::fd::AsRawFd;
use std::ptr::NonNull;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

const FBIOGET_VSCREENINFO: libc::c_ulong = 0x4600;
const FBIOGET_FSCREENINFO: libc::c_ulong = 0x4602;
/// `_IOW('F', 0x2E, struct mxcfb_update_data_rex)`, 80-byte payload.
const MXCFB_SEND_UPDATE_REX: libc::c_ulong = 0x4050_462E;
/// `_IOWR('F', 0x2F, struct mxcfb_update_marker_data)`.
const MXCFB_WAIT_FOR_UPDATE_COMPLETE: libc::c_ulong = 0xC008_462F;

const UPDATE_MODE_PARTIAL: u32 = 0;
const UPDATE_MODE_FULL: u32 = 1;
const TEMP_USE_AMBIENT: i32 = 0x1000;
/// `EPDC_FLAG_ENABLE_INVERSION` from FBInk's `mxcfb-kindle.h`: the PXP
/// inverts the update region on its way from the framebuffer to the panel.
/// FBInk and KOReader report it as working on mxcfb Kindles (`canHWInvert`).
const EPDC_FLAG_ENABLE_INVERSION: u32 = 0x01;

/// The update flags for the current inversion state.
fn update_flags(invert: &AtomicBool) -> u32 {
    if invert.load(Ordering::Relaxed) {
        EPDC_FLAG_ENABLE_INVERSION
    } else {
        0
    }
}

/// EPDC waveform mode: how the panel moves each pixel to its new value.
///
/// The PW4 (Rex) driver uses the Zelda numbering from FBInk's
/// `mxcfb-kindle.h`, not the older PW2 one. Using the PW2 value 4 for A2 on
/// this device silently selects REAGL instead, almost four times slower.
/// Measured times (120 × 120 region): A2 144 ms, DU 309 ms, DU4 344 ms,
/// everything else about 544 ms. See `docs/pw4-reference.md`.
///
/// It is a plain number rather than an enum so experiments can try any mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Waveform(pub u32);

impl Waveform {
    /// Clears to white.
    pub const INIT: Self = Self(0x0);
    /// Fast black/white, no flash.
    pub const DU: Self = Self(0x1);
    /// Full 16-level gray with a flash.
    pub const GC16: Self = Self(0x2);
    /// 16-level gray from white, no flash.
    pub const GL16: Self = Self(0x3);
    /// Ghost-compensated text (REAGL).
    pub const GLR16: Self = Self(0x4);
    /// REAGL with dithering.
    pub const GLD16: Self = Self(0x5);
    /// Fastest black/white, for animation and pen strokes.
    pub const A2: Self = Self(0x6);
    /// The stock UI's flashing full refresh on the PW4. FBInk labels it a
    /// night-mode waveform, but the driver log shows stock using it in day
    /// mode for every full clean refresh.
    pub const GCK16: Self = Self(0x8);
    /// The stock reader's page-turn waveform on the PW4, always sent as a
    /// full update of the whole screen. FBInk labels it night mode too.
    /// Clean pages without a flash; about 544 ms.
    pub const GLKW16: Self = Self(0x9);
    /// Let the driver pick from the content.
    pub const AUTO: Self = Self(257);
}

/// A screen rectangle in framebuffer pixels. `#[repr(C)]` because it is also
/// the `mxcfb_rect` passed to the driver, field order included.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct Rect {
    pub top: u32,
    pub left: u32,
    pub width: u32,
    pub height: u32,
}

impl Rect {
    /// Smallest rectangle covering both.
    pub fn union(self, other: Rect) -> Rect {
        let top = self.top.min(other.top);
        let left = self.left.min(other.left);
        let bottom = (self.top + self.height).max(other.top + other.height);
        let right = (self.left + self.width).max(other.left + other.width);
        Rect {
            top,
            left,
            width: right - left,
            height: bottom - top,
        }
    }
}

/// Submits updates without waiting for each to finish, up to `max_in_flight`
/// outstanding; beyond that the oldest is awaited first.
///
/// This is the measured *baseline*, not the recommended path. The Lab126
/// driver handles one update at a time, and the submit ioctl itself blocks
/// while an earlier update is still on the panel, so a `Presenter` used from
/// an input loop still stalls that loop. Interactive code should use
/// [`crate::display::Display`]. Measurements: `docs/eink-rendering.md`.
pub struct Presenter {
    in_flight: std::collections::VecDeque<u32>,
    max_in_flight: usize,
}

impl Presenter {
    pub fn new(max_in_flight: usize) -> Self {
        Self {
            in_flight: Default::default(),
            max_in_flight: max_in_flight.max(1),
        }
    }

    /// Submit `rect`; returns its update marker (`None` if fully off-screen).
    pub fn present(
        &mut self,
        fb: &mut Framebuffer,
        rect: Rect,
        waveform: Waveform,
    ) -> io::Result<Option<u32>> {
        while self.in_flight.len() >= self.max_in_flight {
            let oldest = self.in_flight.pop_front();
            fb.wait(oldest)?;
        }
        let marker = fb.update(rect, waveform, false)?;
        if let Some(m) = marker {
            self.in_flight.push_back(m);
        }
        Ok(marker)
    }

    /// Wait for every submitted update to reach the panel.
    pub fn flush(&mut self, fb: &Framebuffer) -> io::Result<()> {
        while let Some(marker) = self.in_flight.pop_front() {
            fb.wait(Some(marker))?;
        }
        Ok(())
    }
}

/// Accumulates damage between frames so each frame sends one EPDC update.
///
/// Sending one small update per input event made the first touch test lag by
/// tens of seconds: overlapping updates collide in the driver and each waits
/// for the previous one to finish (309 ms per DU update).
#[derive(Default)]
pub struct Damage {
    rect: Option<Rect>,
}

impl Damage {
    pub fn add(&mut self, rect: Rect) {
        self.rect = Some(match self.rect {
            Some(r) => r.union(rect),
            None => rect,
        });
    }

    pub fn take(&mut self) -> Option<Rect> {
        self.rect.take()
    }
}

#[repr(C)]
#[derive(Default)]
struct AltBufferData {
    phys_addr: u32,
    width: u32,
    height: u32,
    alt_update_region: Rect,
}

#[repr(C)]
#[derive(Default)]
struct UpdateDataRex {
    update_region: Rect,
    waveform_mode: u32,
    update_mode: u32,
    update_marker: u32,
    temp: i32,
    flags: u32,
    dither_mode: i32,
    quant_bit: i32,
    alt_buffer_data: AltBufferData,
    hist_bw_waveform_mode: u32,
    hist_gray_waveform_mode: u32,
}

const _: () = assert!(std::mem::size_of::<UpdateDataRex>() == 80);

#[repr(C)]
struct UpdateMarkerData {
    update_marker: u32,
    collision_test: u32,
}

#[repr(C)]
#[derive(Default)]
struct FbBitfield {
    offset: u32,
    length: u32,
    msb_right: u32,
}

#[repr(C)]
#[derive(Default)]
struct VarScreenInfo {
    xres: u32,
    yres: u32,
    xres_virtual: u32,
    yres_virtual: u32,
    xoffset: u32,
    yoffset: u32,
    bits_per_pixel: u32,
    grayscale: u32,
    red: FbBitfield,
    green: FbBitfield,
    blue: FbBitfield,
    transp: FbBitfield,
    nonstd: u32,
    activate: u32,
    height: u32,
    width: u32,
    accel_flags: u32,
    pixclock: u32,
    left_margin: u32,
    right_margin: u32,
    upper_margin: u32,
    lower_margin: u32,
    hsync_len: u32,
    vsync_len: u32,
    sync: u32,
    vmode: u32,
    rotate: u32,
    colorspace: u32,
    reserved: [u32; 4],
}

#[repr(C)]
struct FixScreenInfo {
    id: [u8; 16],
    smem_start: libc::c_ulong,
    smem_len: u32,
    kind: u32,
    type_aux: u32,
    visual: u32,
    xpanstep: u16,
    ypanstep: u16,
    ywrapstep: u16,
    line_length: u32,
    mmio_start: libc::c_ulong,
    mmio_len: u32,
    accel: u32,
    capabilities: u16,
    reserved: [u16; 2],
}

/// Submits EPDC updates from another thread through its own file descriptor.
/// The pixels are whatever is in the shared framebuffer mapping when the
/// driver processes the update, so drawing stays on the owner of
/// [`Framebuffer`] while this handle only issues the (blocking) ioctl.
pub struct Updater {
    file: File,
    width: u32,
    height: u32,
    next_marker: u32,
    invert: Arc<AtomicBool>,
}

impl Updater {
    /// Count this handle's update markers from `base`, so two updaters on
    /// two threads never hand out the same marker (a waiter would then wake
    /// for the wrong update). The default base is `0x8000_0000`.
    pub fn with_marker_base(mut self, base: u32) -> Self {
        self.next_marker = base;
        self
    }

    /// Same contract as [`Framebuffer::update`]. Blocks while the driver is
    /// still busy with an earlier update (about one waveform duration,
    /// measured: 120 ms for A2).
    ///
    /// `full` drives every pixel in `rect`, not only the changed ones. That is
    /// what clears ghosting: a partial update leaves pixels whose value did
    /// not change alone, including the faint remains of earlier content.
    pub fn update(
        &mut self,
        rect: Rect,
        waveform: Waveform,
        full: bool,
    ) -> io::Result<Option<u32>> {
        let right = rect.left.saturating_add(rect.width).min(self.width);
        let bottom = rect.top.saturating_add(rect.height).min(self.height);
        if rect.left >= right || rect.top >= bottom {
            return Ok(None);
        }
        let rect = Rect {
            top: rect.top,
            left: rect.left,
            width: right - rect.left,
            height: bottom - rect.top,
        };
        // Keep markers disjoint from the owner's, which counts up from 1.
        let marker = self.next_marker;
        self.next_marker = self.next_marker.wrapping_add(1).max(0x8000_0000);
        let mut data = UpdateDataRex {
            update_region: rect,
            waveform_mode: waveform.0,
            update_mode: if full {
                UPDATE_MODE_FULL
            } else {
                UPDATE_MODE_PARTIAL
            },
            update_marker: marker,
            temp: TEMP_USE_AMBIENT,
            flags: update_flags(&self.invert),
            ..Default::default()
        };
        ioctl(&self.file, MXCFB_SEND_UPDATE_REX, &mut data)?;
        Ok(Some(marker))
    }
}

/// Waits for update completion on its own file descriptor; `Send`, so it can
/// live on a measurement thread.
pub struct UpdateWaiter {
    file: File,
}

impl UpdateWaiter {
    pub fn wait(&self, marker: u32) -> io::Result<()> {
        let mut data = UpdateMarkerData {
            update_marker: marker,
            collision_test: 0,
        };
        ioctl(&self.file, MXCFB_WAIT_FOR_UPDATE_COMPLETE, &mut data)
    }
}

/// Geometry as reported by the driver. Never assume `stride == width`.
#[derive(Clone, Copy, Debug)]
pub struct Geometry {
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    pub bits_per_pixel: u32,
    pub x_offset: u32,
    pub y_offset: u32,
    pub rotate: u32,
}

/// The memory-mapped `/dev/fb0` plus the EPDC update interface.
///
/// Not `Send`: it owns a raw mapping. Use [`Framebuffer::waiter`] to wait on
/// updates from another thread.
pub struct Framebuffer {
    file: File,
    map: NonNull<u8>,
    map_len: usize,
    geometry: Geometry,
    id: String,
    next_marker: u32,
    /// Hardware inversion for every update, shared with [`Updater`]s.
    invert: Arc<AtomicBool>,
}

fn ioctl<T>(file: &File, request: libc::c_ulong, arg: *mut T) -> io::Result<()> {
    // SAFETY: `arg` points to a properly sized, initialised #[repr(C)] value
    // matching `request`, and the fd stays open for the call.
    let rc = unsafe { libc::ioctl(file.as_raw_fd(), request as _, arg) };
    if rc < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

impl Framebuffer {
    /// Open and map the framebuffer (normally `/dev/fb0`). Fails unless the
    /// driver reports 8 bits per pixel, the only format this code draws.
    pub fn open(path: &str) -> io::Result<Self> {
        let file = OpenOptions::new().read(true).write(true).open(path)?;
        let mut var = VarScreenInfo::default();
        ioctl(&file, FBIOGET_VSCREENINFO, &mut var)?;
        // SAFETY: plain-old-data struct; all-zero is a valid bit pattern.
        let mut fix: FixScreenInfo = unsafe { std::mem::zeroed() };
        ioctl(&file, FBIOGET_FSCREENINFO, &mut fix)?;
        if var.bits_per_pixel != 8 {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                format!("expected 8 bpp, driver reports {}", var.bits_per_pixel),
            ));
        }
        let map_len = fix.smem_len as usize;
        // SAFETY: mapping the device's own framebuffer memory with its reported length.
        let ptr = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                map_len,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_SHARED,
                file.as_raw_fd(),
                0,
            )
        };
        if ptr == libc::MAP_FAILED {
            return Err(io::Error::last_os_error());
        }
        let id_len = fix.id.iter().position(|&b| b == 0).unwrap_or(fix.id.len());
        Ok(Self {
            file,
            map: NonNull::new(ptr.cast()).expect("mmap returned null"),
            map_len,
            geometry: Geometry {
                width: var.xres,
                height: var.yres,
                stride: fix.line_length,
                bits_per_pixel: var.bits_per_pixel,
                x_offset: var.xoffset,
                y_offset: var.yoffset,
                rotate: var.rotate,
            },
            id: String::from_utf8_lossy(&fix.id[..id_len]).into_owned(),
            next_marker: 1,
            invert: Arc::new(AtomicBool::new(false)),
        })
    }

    /// A handle that can wait for update markers on another thread, to measure
    /// when pixels actually reach the panel without blocking the render loop.
    pub fn waiter(&self) -> io::Result<UpdateWaiter> {
        Ok(UpdateWaiter {
            file: self.file.try_clone()?,
        })
    }

    /// A handle that submits updates from another thread; see [`Updater`].
    pub fn updater(&self) -> io::Result<Updater> {
        Ok(Updater {
            file: self.file.try_clone()?,
            width: self.geometry.width,
            height: self.geometry.height,
            next_marker: 0x8000_0000,
            invert: self.invert.clone(),
        })
    }

    /// Hardware night mode: from now on every update, from this handle and
    /// from every [`Updater`] made from it, asks the PXP to invert the region
    /// on its way to the panel (`EPDC_FLAG_ENABLE_INVERSION`). The
    /// framebuffer keeps day pixels, so screenshots of `/dev/fb0` show the
    /// day picture. The inversion only reaches the panel with the next
    /// update of each region, so follow a switch with a full update of the
    /// whole screen.
    pub fn set_hw_invert(&self, on: bool) {
        self.invert.store(on, Ordering::Relaxed);
    }

    pub fn hw_invert(&self) -> bool {
        self.invert.load(Ordering::Relaxed)
    }

    pub fn geometry(&self) -> Geometry {
        self.geometry
    }

    /// Driver identifier, `mxc_epdc_fb` on the PW4.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Fill a rectangle with one gray level (0 = black, 255 = white).
    pub fn fill(&mut self, rect: Rect, gray: u8) {
        let g = self.geometry;
        let bottom = (rect.top + rect.height).min(g.height);
        let right = (rect.left + rect.width).min(g.width);
        if rect.left >= right {
            return;
        }
        for y in rect.top..bottom {
            let row = ((y + g.y_offset) * g.stride + rect.left + g.x_offset) as usize;
            let len = (right - rect.left) as usize;
            if row + len > self.map_len {
                break;
            }
            // SAFETY: bounds checked against the mapping length above.
            unsafe { std::ptr::write_bytes(self.map.as_ptr().add(row), gray, len) };
        }
    }

    /// Copy an 8-bit image into the framebuffer at `rect`. `src` holds
    /// `rect.height` rows of `src_stride` bytes; the first `rect.width` bytes
    /// of each row are used. Clipped to the screen. One memcpy per row, so a
    /// full page takes well under a millisecond.
    pub fn blit(&mut self, src: &[u8], src_stride: usize, rect: Rect) {
        self.blit_rows(src, src_stride, rect, false);
    }

    /// [`Framebuffer::blit`] with every pixel inverted (`255 - v`), for
    /// software night mode. The inversion is fused into the copy, which is
    /// bound by memory bandwidth, so it costs about the same as a plain blit.
    pub fn blit_inverted(&mut self, src: &[u8], src_stride: usize, rect: Rect) {
        self.blit_rows(src, src_stride, rect, true);
    }

    fn blit_rows(&mut self, src: &[u8], src_stride: usize, rect: Rect, invert: bool) {
        let Some(clipped) = self.clip(rect) else {
            return;
        };
        let g = self.geometry;
        let width = clipped.width as usize;
        for row in 0..clipped.height as usize {
            let src_off = (clipped.top - rect.top) as usize * src_stride
                + row * src_stride
                + (clipped.left - rect.left) as usize;
            let dst_off = ((clipped.top as usize + row + g.y_offset as usize) * g.stride as usize)
                + clipped.left as usize
                + g.x_offset as usize;
            if src_off + width > src.len() || dst_off + width > self.map_len {
                break;
            }
            let from = &src[src_off..src_off + width];
            // SAFETY: the destination range was bounds-checked just above,
            // and the mapping is not aliased by any Rust reference.
            let to =
                unsafe { std::slice::from_raw_parts_mut(self.map.as_ptr().add(dst_off), width) };
            if invert {
                invert_into(to, from);
            } else {
                to.copy_from_slice(from);
            }
        }
    }

    /// Clip `rect` to the visible screen. The driver rejects regions that
    /// extend past the panel with EINVAL.
    pub fn clip(&self, rect: Rect) -> Option<Rect> {
        let g = self.geometry;
        let right = (rect.left.saturating_add(rect.width)).min(g.width);
        let bottom = (rect.top.saturating_add(rect.height)).min(g.height);
        (rect.left < right && rect.top < bottom).then(|| Rect {
            top: rect.top,
            left: rect.left,
            width: right - rect.left,
            height: bottom - rect.top,
        })
    }

    /// Ask the EPDC to refresh `rect` (clipped to the screen). Returns the
    /// update marker, or `None` when nothing of `rect` is visible. Takes about
    /// 1 ms when the panel is idle, but blocks for the rest of the current
    /// update when it is not.
    ///
    /// `full` selects a full update, where every pixel in `rect` is driven
    /// even if unchanged (with GC16 this is the flashing "clean" refresh).
    /// Partial updates only drive pixels whose value changed. Temperature
    /// compensation uses the panel's ambient sensor.
    pub fn update(
        &mut self,
        rect: Rect,
        waveform: Waveform,
        full: bool,
    ) -> io::Result<Option<u32>> {
        let Some(rect) = self.clip(rect) else {
            return Ok(None);
        };
        let marker = self.next_marker;
        self.next_marker = self.next_marker.wrapping_add(1).max(1);
        let mut data = UpdateDataRex {
            update_region: rect,
            waveform_mode: waveform.0,
            update_mode: if full {
                UPDATE_MODE_FULL
            } else {
                UPDATE_MODE_PARTIAL
            },
            update_marker: marker,
            temp: TEMP_USE_AMBIENT,
            flags: update_flags(&self.invert),
            ..Default::default()
        };
        ioctl(&self.file, MXCFB_SEND_UPDATE_REX, &mut data)?;
        Ok(Some(marker))
    }

    /// Block until the update with `marker` has been displayed.
    pub fn wait(&self, marker: Option<u32>) -> io::Result<()> {
        let Some(marker) = marker else { return Ok(()) };
        let mut data = UpdateMarkerData {
            update_marker: marker,
            collision_test: 0,
        };
        ioctl(&self.file, MXCFB_WAIT_FOR_UPDATE_COMPLETE, &mut data)
    }

    /// Draw a thick line by stamping squares of half-width `r` along it, spaced
    /// `r / 2` apart so the stroke has no gaps. Returns the damaged rectangle
    /// for the caller to pass to [`Framebuffer::update`] or a [`Presenter`].
    pub fn line(&mut self, from: (u32, u32), to: (u32, u32), r: u32, gray: u8) -> Rect {
        let (x0, y0) = (from.0 as i64, from.1 as i64);
        let (x1, y1) = (to.0 as i64, to.1 as i64);
        let steps = (x1 - x0).abs().max((y1 - y0).abs()).max(1);
        let stride = (r as i64 / 2).max(1);
        let mut damage: Option<Rect> = None;
        let mut i = 0;
        loop {
            let t = i.min(steps);
            let x = (x0 + (x1 - x0) * t / steps) as u32;
            let y = (y0 + (y1 - y0) * t / steps) as u32;
            let dot = Rect {
                top: y.saturating_sub(r),
                left: x.saturating_sub(r),
                width: 2 * r,
                height: 2 * r,
            };
            self.fill(dot, gray);
            damage = Some(damage.map_or(dot, |d| d.union(dot)));
            if t == steps {
                break;
            }
            i += stride;
        }
        damage.expect("at least one dot")
    }

    /// The whole visible screen.
    pub fn full_screen(&self) -> Rect {
        Rect {
            top: 0,
            left: 0,
            width: self.geometry.width,
            height: self.geometry.height,
        }
    }
}

/// `dst[i] = 255 - src[i]`; a plain loop the compiler vectorizes (NEON on
/// the PW4).
pub fn invert_into(dst: &mut [u8], src: &[u8]) {
    for (d, s) in dst.iter_mut().zip(src) {
        *d = !*s;
    }
}

impl Drop for Framebuffer {
    fn drop(&mut self) {
        // SAFETY: unmapping exactly the region mapped in `open`.
        unsafe { libc::munmap(self.map.as_ptr().cast(), self.map_len) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inversion_flag_follows_the_switch() {
        let invert = AtomicBool::new(false);
        assert_eq!(update_flags(&invert), 0);
        invert.store(true, Ordering::Relaxed);
        assert_eq!(update_flags(&invert), EPDC_FLAG_ENABLE_INVERSION);
    }

    #[test]
    fn invert_into_maps_every_level() {
        let src: Vec<u8> = (0..=255).collect();
        let mut dst = vec![0; 256];
        invert_into(&mut dst, &src);
        assert!(dst.iter().enumerate().all(|(i, &v)| v as usize == 255 - i));
    }
}
