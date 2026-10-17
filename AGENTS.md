# AGENTS.md

Aristotle replaces the Kindle Paperwhite 4's interface. Read the [architecture](docs/architecture.md), [device guide](device/README.md), and [build and checks](.github/CONTRIBUTING.md) for details.

## Ground rules

- No PW4 recovery path is verified. Without the owner's explicit go-ahead, do not write to the root filesystem, boot partitions, bootloader, IDME, diags, `/mnt/wfm`, or `/var/local`.
- Keep device experiments reversible. Use bind mounts, copies in `/var/tmp`, or stopped services. A reboot must restore the stock system.
- Never add jailbreak, exploit, or DRM code, Amazon files, or Amazon cloud calls. Follow [What this project will never include](.github/CONTRIBUTING.md#what-this-project-will-never-include).
- Write code, comments, docs, and commits in English.
- `third_party/plato` is our fork on branch `aristotle`. Commit each Plato change there as one small, described commit, then bump the submodule here. Follow the [Plato fork workflow](docs/reader.md#plato-fork).
- Use public-domain books or text written for tests. Never use copyrighted books or anyone's library.
- Mark device claims as measured on the device, read from firmware, or taken from a named project. Label inferences as inferences.

## Host

- Put builds, downloads, probe output, and scratch files under `$ARISTOTLE_CACHE`, never `/tmp`. Source `tools/env.sh` before manual Cargo or SSH commands. It sets `$ARISTOTLE_CACHE` and `$TMPDIR`. Run `export CARGO_TARGET_DIR="$ARISTOTLE_CACHE/target"`.
- Keep machine-specific paths out of the repo. Use variables.

## Device access and tests

- Run `tools/host/setup-host-net.sh` once on the host. In KUAL, choose Aristotle > Start USB network once (manual). Connect with `ssh -F tools/host/ssh_config kindle`.
- Use `tools/host/run.sh PACKAGE[:BINARY] [ARGS]` to build, copy to `/var/tmp`, run, and capture the screen. See the [device guide](device/README.md) for setup.
- The [device guard](device/README.md#the-device-guard) requires a PW4 on firmware 5.13.x. `ARISTOTLE_FORCE=1` bypasses it. After changing guarded scripts, run the [device tests](device/README.md#tests).
- `/mnt/us/aristotle-disable` prevents the agent and Aristotle from starting at boot.
- Before and after latency changes, run the same `tools/scenarios/` script with `aristotle-observe run SCRIPT`. Compare driver logs using the [benchmark method](docs/benchmarks.md#method).

## Device constraints

- Firmware uses softfp userspace. Build static `armv7-unknown-linux-musleabihf` binaries, not hard-float dynamic binaries. See [stock userspace](docs/pw4-reference.md#stock-userspace) for firmware and device evidence.
- The measured framebuffer is 1072 × 1448 visible, with 1088 bytes per line, 6144 virtual lines, 8 bpp, and `rotate=3`. Read stride through `FBIOGET_FSCREENINFO` and offsets through `FBIOGET_VSCREENINFO`. See [display measurements](docs/pw4-reference.md#display).
- Use Lab126's `MXCFB_SEND_UPDATE` layout and FBInk's Zelda waveform numbers, not Kobo or PW2 numbers. A2 = 6, GL16 = 3, and REAGL = 4. See [waveform modes](docs/pw4-reference.md#waveform-modes-and-timings).
- Measurements show one EPDC update at a time. Submits block behind earlier updates, even for separate regions. Never submit from the input loop. Pass damage to `aristotle_hal::display::Display` and its [display thread](docs/ui-framework.md#display-thread).
- Turn pages with one full-screen, full-mode update in mode 9, as measured in the stock reader. Partial updates and A2 followed by GL16 caused ghosting. See the [page-turn experiment](docs/benchmarks.md#page-turn-strategy-experiment).
- Find touch by name in `/proc/bus/input/devices`, never by `eventN`. See the [input evidence](docs/pw4-reference.md#input).
