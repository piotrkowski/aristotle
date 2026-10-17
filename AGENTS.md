# AGENTS.md

Repo `aristotle`, product **Aristotle**: full liberation of a Kindle Paperwhite 4, with our own Rust UI framework and reader borrowing from Plato. See `README.md` for the goal and `docs/pw4-reference.md` for the full device reference. This file is the short list of things you would otherwise get wrong.

## Ground rules

- Personal research project on one refurbished test device. Bricking is an accepted risk, but **no recovery path has been verified yet**. Until one exists, do not write to the root filesystem, boot partitions, bootloader, `/mnt/wfm` or `/var/local` without the owner's explicit go-ahead.
- Run device-side experiments as reversible overlays: bind mounts, `/var/tmp` copies, stopped services. A reboot must bring back the stock system.
- Everything in this repo is in English: code, comments, docs, commits.
- `third_party/plato` is an upstream submodule kept as a reference. Don't edit it. When you borrow code, copy it into our crates and keep the AGPL notice and attribution.
- Mark device claims with their source: verified on the device, read from firmware, or borrowed from KOReader or other projects. Don't promote an inference to a fact.

## Host environment

- `/tmp` is a small RAM tmpfs. Keep builds, downloads and probe output under `~/.cache/aristotle/`, and set `CARGO_TARGET_DIR` and `TMPDIR` there.
- Large artifacts live outside git. `docs/pw4-reference.md` lists them under "Host-side artifacts". Read the stock rootfs image without root using `debugfs -R 'cat /etc/...' rootfs.img`.
- RE tools: `tools/fetch-tools.sh` installs pinned Ghidra, JDK 21 and JADX. Then `. ~/.cache/aristotle/tools/env.sh`. The REA MCP server (`re-agents`, Ghidra-backed) is registered at Claude Code's local scope for this directory only, so sessions started here can decompile ARM binaries from the firmware through it. The Python helpers in `probe/host/` need `~/.cache/aristotle/venv` (`python-lzo`, `fdt`).
- A shell hook blocks `rm -rf`, `unlink`, `mv` inside `~/.cache`, and truncating `>` redirects onto existing home paths. Write fresh output directories instead of cleaning up, and use `>>` for logs.
- When the Kindle is plugged in, a udev-triggered highlights exporter mounts it **read-only** at `/run/media/$USER/Kindle`. To write: `udisksctl unmount -b /dev/sda && udisksctl mount -b /dev/sda`.

## Running code on the device

- **Emulator first.** `devloop/emulator/setup.sh` builds katadelos' QEMU fork and creates a stock PW4 5.13.6 instance with KUAL, MRPI and the jailbreak under `~/.cache/aristotle/emulator`. Then `./eink run pw4 --headless --qmp-socket --serial-socket` from `eink-emulator/`: SSH (`ssh -i build/ssh/id_ed25519 -p 2222 root@127.0.0.1`) is up after about 30 s and Home after about 2.5 min. The setup applies `eink-emulator-pw4.patch`, which fixes two things. First, the PW4 board id must start with `P` and carry `15R`, or the guest boots as `yoshi` and hangs. Second, under QEMU the on-screen keyboard is limited to one language, because loading a second `kb` plugin crashes (a QEMU bug, not yet located), and three crashes make the guest reboot and QEMU exit. Try anything that could hang boot, touch the framework or kexec there first. Real e-ink behaviour (ghosting, waveform timing) still needs the device. The host has 2 cores, so the first QEMU build is slow.
- **Dev loop: SSH over the USB cable, no replugging.** KUAL › Aristotle USB network › Start (`devloop/kindle/aristotle-usbnet`) unloads `g_mass_storage`, loads `g_ether` with fixed MACs, brings up `usb0` at `192.168.15.244`, opens `usb0` in the stock iptables firewall, sets `preventScreenSaver` on powerd, and starts KOReader's dropbear (key-only). Then `devloop/host/run.sh [crate]` builds, copies to `/var/tmp`, runs and saves a framebuffer PNG; `ssh -F devloop/host/ssh_config kindle` for a shell. Auth uses a dedicated key, `~/.cache/aristotle/ssh/kindle_ed25519`. Stop, or a reboot, reverts everything.
- One-time host setup: `devloop/host/setup-host-net.sh` (NetworkManager profile matched by MAC, host `192.168.15.201`). Use `devloop/host/install-extensions.sh` whenever the device is in drive mode.
- Eject with `udisksctl unmount -b /dev/sda` only. `udisksctl power-off` removes the device from the USB bus, so the network gadget will not appear until a physical replug.
- Drive mode is one-shot. After ejecting on the host, the device leaves mass-storage mode while the cable stays in (the host sees a 0-byte `sda`), and only a replug re-enters it. While the userstore is exported, the device cannot write `/mnt/us`.
- Fallback without the network: copy a KUAL extension to `/mnt/us/extensions/<id>/` (`config.xml`, `menu.json`, `bin/*.sh`), eject, run it from KUAL (it runs as root), replug, and read the output from `/mnt/us`. `probe/device/aristotle-probe` is the template.
- Show status on screen with `eips` (stock) or FBInk (`/mnt/us/libkh/bin/fbink`).

## Device facts that break naive code

- **ABI.** Stock userspace is **softfp**: soft-float calling convention with VFPv3/NEON instructions, glibc 2.20, `/lib/ld-linux.so.3`, the koxtoolchain `kindlepw2` target. Hard-float *dynamic* binaries, including Plato's upstream `arm-unknown-linux-gnueabihf` build with `libmupdf.so`, will not run against the system libraries. Build **static `armv7-unknown-linux-musleabihf`**, which others report working on PW4 5.13.x. Use `armv7-unknown-linux-gnueabi` only if you must `dlopen` Amazon libraries such as `liblipc.so`.
- **Framebuffer.** It is not "width × height": visible 1072 × 1448, line length 1088 bytes, virtual height 6144, 8 bpp, `rotate=3`. Always use the stride from `FBIOGET_FSCREENINFO` and the offsets from `FBIOGET_VSCREENINFO` (both offsets are 0 on our unit; the "viewVertOrigin=4" that FBInk prints is its own view adjustment).
- **E-ink updates.** They use Lab126's own `MXCFB_SEND_UPDATE` struct and the *Zelda* waveform numbering (A2 = 6, GL16 = 3, REAGL = 4), not Kobo's or the PW2 one. Do not copy Plato's `mxcfb_sys.rs` structs blindly; use FBInk's Kindle headers as the ABI reference. Waveforms come from `/mnt/wfm/waveform_to_use/` and are chosen by temperature.
- **Page turns must be clean, not just fast.** Turn pages exactly like the stock reader: one **full-mode** update of the **whole screen** with waveform **mode 9**, and use mode 8 full for flash refreshes. Partial updates, and A2 previews followed by GL16, left ghosting the owner rejected on real reading. Short demos with near-identical pages hid it. Prerender the next page during the panel refresh. Details: `docs/native-vs-aristotle.md`.
- **One update at a time.** The driver blocks `MXCFB_SEND_UPDATE` itself while an earlier update is still on the panel, whatever the regions. Never submit from an input loop: draw, then hand the damage to `aristotle_hal::display::Display`, whose thread coalesces everything drawn meanwhile into the next update. Deeper queues and per-tile updates were measured and are worse (`docs/eink-rendering.md`).
- **Measure, don't guess.** UI latency work uses record and replay: `aristotle-pen-live record=...` captures real input with kernel timestamps; `aristotle-pen-bench` replays it and reports event-to-panel latency percentiles. A reference recording is kept at `~/.cache/aristotle/pen-session-1.rec`.
- **Touch.** The chip varies per unit (Parade cyttsp5 or Goodix gt9xx, detected at boot), and the event node number varies too (an extra proximity sensor on LTE units). Find the device by name in `/proc/bus/input/devices`, never by a hard-coded `eventN`.
- **Users.** Stock daemons run as unprivileged users; for example `pillowd` runs as `framework`. Files you bind-mount over theirs must be readable by them. A root-only `umask 077` copy silently broke an earlier Pillow patch.
- **Taking the screen.** Per KOReader's practice (not yet verified by us), `powerd` keeps driving suspend and the screensaver after the GUI stops. `volumd` owns the USB export. Stop the GUI stack (`stop lab126_gui`) but keep or replace those two deliberately. See the KOReader notes in the reference.
- **Kernel and boot.** Modules are unsigned, so our own `.ko` files load if the vermagic matches `4.1.15-lab126`. The PW4 is reported **secure-boot locked** (U-Boot: `secure_cpu: 1, production: 1, unlocked: 0`; fastboot flash is blocked). eMMC access goes through Lab126's "Falcon" layer. So "own kernel" means kexec from the running stock kernel, tried first in the QEMU PW4 emulator (`katadelos/eink-emulator`). Treat the boot partitions, diags and IDME as read-only.
- **Prior art.** Read `docs/prior-art.md` before building anything that might already exist. Display-ABI Rust code is in `slint-kindle-backend`, a reversible takeover pattern in `yb-reader`, a QEMU PW4 machine in `eink-emulator`, and the maintained Plato fork is `cadmus`.

## Workflow

- Static questions: answer them from the firmware image in `~/.cache/aristotle/probe/static/` before touching the device.
- Runtime questions: extend `probe/device/aristotle-probe/bin/probe.sh`. It must stay read-only outside its output directory. Copy results into `~/.cache/aristotle/probe/device/` and record durable findings in `docs/pw4-reference.md`.
- For Plato: the device-specific parts to port are in `third_party/plato/crates/core/src/{framebuffer,input.rs,device.rs,frontlight,battery,lightsensor}`. Rendering goes through MuPDF (`mupdf_wrapper`). The `emulator` crate runs the UI on the desktop through SDL.
