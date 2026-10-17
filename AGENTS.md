# AGENTS.md

Aristotle replaces the Kindle Paperwhite 4's interface. This file lists what you would otherwise get wrong. `docs/architecture.md` describes the code, `docs/pw4-reference.md` the device, and `.github/CONTRIBUTING.md` the build and the checks.

## Ground rules

- No recovery path for the PW4 is verified yet, and a mistake can brick it. Do not write to the root filesystem, boot partitions, bootloader, `/mnt/wfm` or `/var/local` without the user's explicit go-ahead.
- Make device experiments reversible (bind mounts, copies in `/var/tmp`, stopped services). A reboot must bring back the stock system.
- Never add jailbreak, exploit or DRM code, firmware, waveforms, Amazon binaries or fonts, decompiled code, or calls to Amazon's cloud. The full list is "What this project will never include" in `.github/CONTRIBUTING.md`.
- Code, comments, docs and commits are in English.
- `third_party/plato` is our fork ([piotrkowski/plato](https://github.com/piotrkowski/plato), branch `aristotle`), upstream plus one small, described commit per change. Commit Plato changes there, then bump the submodule here (`docs/reader.md`, "Plato fork").
- Code borrowed from elsewhere is copied into our crates with its AGPL notice and attribution, and listed in `THIRD_PARTY.md`.
- Test data and docs use public-domain books (Prus, Mickiewicz, Plato in Witwicki's translation, Austen, Melville) or text written for the test. Never use copyrighted books or anyone's own library.
- Say where each device claim comes from: measured on the device, read from the firmware, or taken from KOReader or another project. Do not state an inference as fact.

## Host

- Builds, downloads and probe output go under `$ARISTOTLE_CACHE`, never `/tmp`, which is a small RAM disk on many hosts. Source `devloop/env.sh` (it sets the variable and `TMPDIR`) before running cargo or `ssh -F devloop/host/ssh_config` by hand, and put `CARGO_TARGET_DIR` under the cache.
- Never write paths from your own machine into the repo. Name the variable.
- Large artifacts stay out of git (`docs/pw4-reference.md`, "Host-side artifacts"). Read files from the stock rootfs image without root: `debugfs -R 'cat /etc/...' rootfs.img`.
- `tools/fetch-tools.sh` installs pinned Ghidra, JDK 21 and JADX; then `. "$ARISTOTLE_CACHE/tools/env.sh"`. The Python helpers in `probe/host/` need a virtualenv at `$ARISTOTLE_CACHE/venv` with `python-lzo` and `fdt`.
- Some hosts block destructive commands (`rm -rf`, `mv` inside caches, truncating `>`). The scripts write fresh output directories and append to logs with `>>`. Keep it that way.
- If the host mounts the Kindle read-only, remount it: `udisksctl unmount -b /dev/sdX && udisksctl mount -b /dev/sdX`.

## Running code on the device

- Try anything that could hang boot, touch the stock framework or kexec in the emulator first. `devloop/emulator/setup.sh` builds katadelos' QEMU fork and a jailbroken stock PW4 5.13.6 instance under `$ARISTOTLE_CACHE/emulator`. Start it from `eink-emulator/` with `./eink run pw4 --headless --qmp-socket --serial-socket`. SSH (`ssh -i build/ssh/id_ed25519 -p 2222 root@127.0.0.1`) is up after about 30 s, Home after about 2.5 min. Under QEMU the on-screen keyboard has one language, because a second `kb` plugin crashes the guest.
- Dev loop: run KUAL › Aristotle › Start USB network once (`devloop/kindle/aristotle-usbnet`). It puts the Kindle at `192.168.15.244` on a USB network and starts KOReader's dropbear, key-only. Stop, or a reboot, undoes it. On the host, run `devloop/host/setup-host-net.sh` once. Then `devloop/host/run.sh [crate]` builds, copies to `/var/tmp`, runs and saves a framebuffer PNG, and `ssh -F devloop/host/ssh_config kindle` gives a shell. The key is `$ARISTOTLE_CACHE/ssh/kindle_ed25519`.
- Run `devloop/host/install-extensions.sh` whenever the device is in drive mode.
- `agent/install.sh`, `bin/start.sh` and `takeover.sh start` refuse to run unless the device is a PW4 on 5.13.x (`aristotle-usbnet/guard.sh`). `ARISTOTLE_FORCE=1` overrides this, which the emulator may need. After changing these scripts, run the tests in `devloop/kindle/tests/`.
- `/mnt/us/aristotle-disable` keeps the agent from starting at boot. `agent/uninstall.sh` removes the agent and its SSH key.
- Eject with `udisksctl unmount -b /dev/sda`. `udisksctl power-off` drops the device from the bus until a physical replug.
- Drive mode is one-shot. After an eject the host sees a 0-byte `sda`, and only a replug brings the drive back. While the userstore is exported, the device cannot write `/mnt/us`.
- Without the network: copy a KUAL extension to `/mnt/us/extensions/<id>/` (`config.xml`, `menu.json`, `bin/*.sh`), eject, run it from KUAL (as root), replug and read the output from `/mnt/us`. `probe/device/aristotle-probe` is a template.
- `eips` (stock) or `/mnt/us/libkh/bin/fbink` print status on screen.

## Device facts that break naive code

- Stock userspace is softfp (glibc 2.20, `/lib/ld-linux.so.3`, koxtoolchain `kindlepw2`). Hard-float dynamic binaries, Plato's upstream build included, do not run against its libraries. Build static `armv7-unknown-linux-musleabihf`. Use `armv7-unknown-linux-gnueabi` only to `dlopen` Amazon libraries such as `liblipc.so`.
- The framebuffer is 1072 × 1448 visible, 1088 bytes per line, 6144 lines virtual, 8 bpp, `rotate=3`. Take the stride from `FBIOGET_FSCREENINFO` and the offsets from `FBIOGET_VSCREENINFO`. Both offsets are 0 on our unit; FBInk's "viewVertOrigin=4" is its own adjustment.
- Updates use Lab126's `MXCFB_SEND_UPDATE` struct and the Zelda waveform numbering (A2 = 6, GL16 = 3, REAGL = 4). Kobo and PW2 numbers are wrong here, so do not copy Plato's `mxcfb_sys.rs`. FBInk's Kindle headers are the ABI reference. The driver loads waveforms from `/mnt/wfm/waveform_to_use/` and picks them by temperature.
- Turn pages as the stock reader does: one full-mode update of the whole screen with waveform mode 9. Flashes use mode 8, full. Partial updates, and A2 followed by GL16, leave ghosting in real reading. Prerender the next page during the refresh. A tap turns on release, and a hold selects text without turning. See `docs/native-vs-aristotle.md`.
- The driver runs one update at a time. `MXCFB_SEND_UPDATE` blocks while an earlier update is on the panel, whatever the regions. Never submit from an input loop. Draw, then pass the damage to `aristotle_hal::display::Display`, whose thread merges it into the next update. Deeper queues and per-tile updates measured worse.
- Measure latency by record and replay against one reference recording (`docs/eink-rendering.md`).
- The touch controller differs per unit (Parade cyttsp5 or Goodix gt9xx), and so does the event node (LTE units have an extra proximity sensor). Find the device by name in `/proc/bus/input/devices`, never by `eventN`.
- Stock daemons run unprivileged; `pillowd` runs as `framework`. Files you bind-mount over theirs must be readable by them; a root-only copy (`umask 077`) breaks them silently.
- According to KOReader (not verified by us), `powerd` keeps driving suspend and the screensaver after `stop lab126_gui`, and `volumd` owns the USB export. Keep or replace those two on purpose.
- Kernel modules are unsigned and load if the vermagic is `4.1.15-lab126`. The PW4 is reported secure-boot locked (U-Boot `secure_cpu: 1, production: 1, unlocked: 0`), so an own kernel means kexec from the stock one. Treat boot partitions, diags and IDME as read-only.

## Workflow

- Check `docs/related-projects.md` before building something that may already exist.
- Answer static questions from the firmware image in `$ARISTOTLE_CACHE/probe/static/` before touching the device.
- For runtime questions, extend `probe/device/aristotle-probe/bin/probe.sh`. It must stay read-only outside its output directory. Copy results to `$ARISTOTLE_CACHE/probe/device/` and record lasting findings in `docs/pw4-reference.md`.
