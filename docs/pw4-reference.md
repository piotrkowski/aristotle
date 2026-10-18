# Kindle Paperwhite 4 reference

This reference records device and stock-firmware facts for contributors. Source footnotes apply to the claim or table they follow.

## Device

Device trees, abbreviated DT, describe the board hardware. DTBs are compiled trees, and SoC means system on a chip. NEON is Arm's vector instruction set, and VFPv3 is its floating-point hardware interface.

| Property | Value |
| --- | --- |
| Model | Kindle Paperwhite 4 (2018), codename `Moonshine`, platform `Rex`[^1][^2] |
| Board DTBs | `imx6sll-moonshine-wfo` for Wi-Fi, `imx6sll-moonshine-wan` for LTE[^1] |
| SoC | NXP i.MX6 SoloLite-Lite, one Cortex-A9 with NEON and VFPv3[^1] |
| RAM | 512 MB. The DT `memory` node says 2 GB at `0x80000000`, and the bootloader fixes it up[^1] |
| Firmware | 5.13.6. Version file: `/etc/prettyversion.txt`[^2] |
| Identification | `/proc/usid` holds the serial. `device/extension/guard.sh` checks PW4 codes and firmware[^2][^3] |
| Kernel | `4.1.15-lab126`, `HZ=100`[^1][^2] |

## Boot chain

The update package `update_kindle_all_new_paperwhite_v2_5.13.6.bin`, unpacked with [KindleTool](glossary.md#kindletool):[^1]

| File | What it is |
| --- | --- |
| `imx6sll_rex/u-boot.bin` | [U-Boot](glossary.md#u-boot) |
| `imx6sll_rex/bios.bin`, `s-bios.bin` | Resident Falcon BIOS |
| `imx6sll_rex/boot.img` | Kernel, initramfs, and board device trees |
| `rootfs.img.gz` | ext3 [root filesystem](glossary.md#rootfs) |

Components have `.sig` signatures.[^1] A [MobileRead serial-log report](https://www.mobileread.com/forums/showthread.php?t=354311) shows `secure_cpu: 1, production: 1, unlocked: 0` and blocked fastboot writes.[^4] Signed boot-image enforcement remains an inference.[^5] Treat boot partitions as read-only.

## Hardware map

From `imx6sll-moonshine-wfo.dts`.[^1] [EPDC](glossary.md#epdc) controls the e-ink display, and [PXP](glossary.md#pxp) processes pixels. EPD means electrophoretic display, PMIC means power-management chip, and RTC means real-time clock.

The table lists each component, bus address, and driver:

| Function | Part | Bus and address | Kernel driver |
| --- | --- | --- | --- |
| E-ink controller | i.MX EPDC `fsl,imx6sll-epdc` | `0x020f4000`, IRQ 97 | `mxc_epdc_v2_fb.ko` (module) |
| Pixel pipeline | PXP v3 `fsl,imx6ull-pxp-dma` | `0x020f0000` | built in |
| EPD power and temperature | FitiPower FP9929 | I2C1 `0x48` | built in |
| System PMIC, fuel gauge, RTC | ROHM BD71827 | I2C1 `0x4b` | built in |
| Frontlight | TI LM3695 | I2C1 `0x63` | `ti,lm3695-backlight` |
| Touch, variant A | Parade TrueTouch `cy,cyttsp5_i2c_adapter` | I2C2 `0x24` | `cyttsp5*.ko` |
| Touch, variant B | Goodix `gt9xx` | I2C2 `0x14` | `gt9xx-ts.ko` |
| USB charger detection | Maxim MAX14656 | I2C3 `0x35` | built in |
| Wi-Fi | Broadcom BCM43430 (4343W), SDIO | | `bcmdhd.ko`, firmware `/opt/brcm_4343w/` |
| Bluetooth | same chip, `amzn,bt_pwr_ctrl` | Property | Value |
| Cover sensor | `amzn,rex_hall` | Property | Value |
| Crash log | `fsl,mxc_ramoops` | Property | Value |

## Kernel configuration

The config is embedded in the kernel and at `/proc/config.gz` on the device.[^1]

- Modules are not signed, so separately built modules load if their kernel-version signature, `vermagic`, matches.
- `DEVTMPFS_MOUNT`, `CGROUPS`, `FUSE_FS`, `EXT4`, `SQUASHFS`, `INPUT_EVDEV`, `PREEMPT` and `SUSPEND` are on. The default cpufreq governor is `performance`.
- [USB gadget](glossary.md#usb-gadget) functions include mass storage, RNDIS and ECM networking, ACM serial, FunctionFS userspace functions, and MTP media transfer. `g_ether.ko`, `g_mass_storage.ko` and `g_ffs.ko` are modules.
- `DRM_VIVANTE` is built in and unused. The display path uses only fbdev, Linux's [framebuffer](glossary.md#framebuffer) interface.

## Display

From [FBInk](glossary.md#fbink) and sysfs on the device.[^2] The framebuffer stores pixels, with a row stride that can exceed the visible width:

| Property | Value |
| --- | --- |
| Framebuffer device | `/dev/fb0`, id `mxc_epdc_fb` |
| Visible area | 1072 × 1448 at 300 DPI |
| Virtual size | 1088 × 6144 (line length 1088 bytes) |
| Pixel format | 8 bpp grayscale |
| Rotation | `rotate=3` |
| Offsets | `xoffset=0`, `yoffset=0` from `FBIOGET_VSCREENINFO`. FBInk's `viewVertOrigin=4` is its own view adjustment |
| Night mode | hardware inversion available (`canHWInvert=1`) |

The EPDC driver loads [waveforms](glossary.md#waveform) with `request_firmware("eink/waveform")` from `/mnt/wfm/waveform_to_use/`, a partition managed by the [Upstart](glossary.md#upstart) jobs `wfmupdate` and `wfmdelete`.[^1] It picks a LUT by the temperature from the FP9929 or BD71827, which sysfs can override. The update ioctl is [Lab126](glossary.md#lab126)'s variant of `MXCFB_SEND_UPDATE`, and FBInk's Kindle headers are the reference for its layout.

### Waveform modes and timings

The Rex EPDC driver uses the Zelda numbering from FBInk's `mxcfb-kindle.h`, where [A2](glossary.md#a2) is mode 6. The PW2 numbering does not apply. [DU](glossary.md#du) and [DU4](glossary.md#du4) use fewer shades than [GC16](glossary.md#gc16) and [GL16](glossary.md#gl16). [REAGL](glossary.md#reagl) includes GLR16 and its dithered variant GLD16, while [GCK16](glossary.md#gck16) and [GLKW16](glossary.md#glkw16) are the modes FBInk labels for night use.

Times were measured with `aristotle-observe bench latency` on a 120 × 120 region toggling black and white, median of 6, on an idle panel:[^2]

| Mode | Name | Time |
| --- | --- | --- |
| 1 | DU | 309 ms |
| 2 | GC16 | 544 ms |
| 3 | GL16 | 544 ms |
| 4 | GLR16 (REAGL) | 545 ms |
| 5 | GLD16 (REAGL dithered) | 544 ms |
| 6 | A2 | 144 ms |
| 7 | DU4 | 344 ms |
| 8 | GCK16 (night) | 544 ms |
| 9 | GLKW16 (night) | 544 ms |
| 257 | AUTO (picked DU for this content) | 309 ms |

Other measured updates:[^2]

- A2: 120 to 144 ms.
- Full-screen GC16: about 570 ms.
- 1072 × 724 GL16 partial: about 565 ms.
- 992 × 60 DU partial: about 320 ms.

The driver runs one update at a time, even for non-overlapping regions.[^2] Use the [display thread](ui-framework.md#e-ink-rendering) for partial UI updates.

## Input

- `/etc/upstart/touch` probes Parade and Goodix controllers and loads the matching driver.[^1]
- Our Goodix `goodix-ts` uses framebuffer coordinates.[^2] Event numbers vary, including on LTE units.[^3] Find touch by name in `/proc/bus/input/devices`.
- The Goodix node declares only multi-touch axes and has no `BTN_TOUCH`, `ABS_X` or `ABS_Y`.[^2] [`benchmarks.md`](benchmarks.md#method) lists what an injected contact needs.
- The kernel's `struct input_event` is 16 bytes, with a 32-bit `timeval`. musl's time64 layout is 24 bytes, so `aristotle-hal::input` defines its own.

## Power and light

[`powerd`](glossary.md#powerd) is the stock power-management service.

- Frontlight: `/sys/class/backlight/bl/brightness`, range 0 to 2047. Writes take effect and read back exactly.[^2]
- Battery: `/sys/class/power_supply/bd71827_bat/` with `capacity`, `status`, `charging`, `voltage_now` (`µV`) and `temp` (`°C`). The charger is `bd71827_ac`.[^2]
- Hall sensor: `/sys/bus/platform/drivers/hall_sensor/rex_hall/hall_enable`.[^3]
- `preventScreenSaver=1` blocks screensaver and suspend,[^3] and disables the power button.[^2] The reader sets it with `ARISTOTLE_POWERD=0` or when managed sleep cannot be acquired.
- Firmware uses an enum for `powerd`'s `drive_mode_state`: `1` means drive mode is off, `2` is the passcode state, and `3` is drive mode on. The button's drive-mode blocker checks for `3`, not `1`.[^1] A button was ignored during USB networking,[^2] but that observation does not establish USB networking as the cause.
- Before accepting the button, `powerd` checks that KAF's `frameworkStarted` property is nonzero and blanket's loaded modules do not include `splash`. A failed property read also blocks the button.[^1] Framework-less sleep needs separate validation.
- Stock framework setup unloads blanket's splash module and sends `bootSplashCleanup` from `com.lab126.hal`.[^1] `ui-boot.sh` repeats those steps when booting without the framework; the event does not replace the button's property checks.

## Filesystems

tmpfs is volatile storage. The [userstore](glossary.md#userstore) is the book storage exposed over USB.

| Mount | Type | Notes |
| --- | --- | --- |
| `/` | ext3, read-only | `/dev/mmcblk1p8`[^1][^2] |
| `/var/local` | ext3, read-write | `/dev/mmcblk1p9`, persistent state: `appreg.db`, `cc.db`, Java prefs[^2] |
| `/var` | tmpfs, 64 MB | lost on reboot, `/var/run` included[^2] |
| `/mnt/us` (also `/mnt/base-us`) | FAT32, about 6.2 GB | the "userstore", exported to the host as USB mass storage[^2] |
| `/mnt/wfm` | | e-ink waveforms[^1] |

## Stock userspace

The C library is glibc 2.20 with [softfp](glossary.md#softfp) ARM EABI5, the embedded ABI in version 5: soft-float calling convention, VFPv3 and NEON code, `/lib/ld-linux.so.3`.[^1] Static musl binaries targeting `armv7-unknown-linux-musleabihf` use hard-float calls internally and run on it.[^2]

Key jobs:[^1]

| Job | Role |
| --- | --- |
| `powerd` | Suspend, screensaver, and battery events |
| `volumd` | USB storage export |
| `lipcd` | Property and event bus |
| `x`, `framework`, `pillow`, `kb` | Window system, Java framework, UI, and keyboard |
| `otaupd`, `otav3` | Firmware updates |

Upstart 0.6.6[^2] supports neither `.override` files nor `manual`.[^1] A job using `on starting X` completes before X starts. The agent uses this ordering to bind-mount over `/usr/bin/otav3`.[^6]

`stored` reboots after more than five starts lasting under 10 seconds.[^1] `takeover.sh stop` leaves it and related jobs to `framework_ready`.

The framework recursively changes group ownership and permissions under `/var/local`, `/var/log`, `/var/lock`, `/var/tmp`, and `/var/run`.[^1] `/var/tmp` is world-writable without a sticky bit, and `fs.protected_symlinks` is 0.[^2] See [Security](../device/README.md#security).

[LIPC](glossary.md#lipc) is Lab126's property and event service, built on the D-Bus interprocess message system. Its commands include `lipc-probe`, `lipc-get-prop`, `lipc-set-prop`, and `lipc-wait-event`. Names use `com.lab126.<name>`.[^1]

The stock reader stores state in `<book>.sdr/` and highlights in [`My Clippings.txt`](glossary.md#my-clippingstxt).[^1] See [Library formats](library.md#stock-files-sidecars-and-my-clippingstxt).

## USB network

By default the port runs `g_mass_storage`, and `volumd` exports the userstore.[^2] While it is exported, the device cannot write `/mnt/us`. `g_mass_storage` unloads only after the host has ejected the drive.[^2]

After a storage-to-network transition, the tested PW4 retained the FSP mount at `/mnt/us` but lacked its FAT backing mount at `/mnt/base-us`. Directory access returned `ESTALE`, and the stock interface was unusable. `volumd` still reported `userstoreIsAvailable=1`, `driveModeState=0`, and `useUsbForNetwork=1`. With no storage gadget loaded and drive mode cleared, plain `/usr/sbin/mntus mount` restored the backing mount and access to the stock interface. No filesystem check or forced unmount was used.[^2]

In the 5.13.6 firmware, `/etc/sysconfig/mntus` sets `MNTUS_MP=/mnt/base-us` and `MNTUS_LOOP_DEV=/dev/loop/0`. The stock `mntus mount` helper attaches the loop device if needed, then mounts the configured filesystem. It checks the filesystem only when called with `--fsck`. It has no gadget or `volumd` export-state guard, so the caller must check those conditions before mounting.[^1]

The Aristotle agent and *[KUAL](glossary.md#kual) › Aristotle › Start USB network* replace it with [`g_ether`](glossary.md#g_ether) and use [Dropbear](glossary.md#dropbear) for SSH.[^2] The link uses fixed addresses without DHCP, and iptables controls access:

| Property | Value |
| --- | --- |
| Device MAC, address | `02:61:72:69:73:01`, `usb0` at `192.168.15.244/24` |
| Host MAC, address | `02:61:72:69:73:02`, `192.168.15.201/24`, no gateway or DHCP |
| Firewall | the stock `iptables` rules drop unsolicited input. The scripts accept `usb0` only |
| SSH | KOReader's `dropbear`, key-only, on `192.168.15.244:22` only |

`udisksctl unmount` ejects the drive. Re-entering drive mode requires a physical replug. `udisksctl power-off` also requires a replug.[^2] See [Network policy](../device/README.md#network).

## Taking over the screen

[`device/app/takeover.sh start|stop|status`](glossary.md#takeover) controls the stock GUI. On our device:[^2]

- `stop lab126_gui` stops `cvm`, Pillow, and the keyboard. The script also stops cloud and telemetry jobs. Available RAM rises from 236 MB to 392 MB.
- Xorg, `awesome`, `lxinit`, and `blanket` remain running.
- `SIGSTOP` is not a substitute, because the watchdog killed the frozen framework.
- Restarting the GUI takes 30 to 60 seconds of boot splash.

KOReader uses `stop lab126_gui` or disables Pillow and freezes `awesome`.[^3] KOReader reports that `powerd` continues suspend and screensaver control, and `volumd` owns USB export.[^3] These services remain necessary when the stock GUI is stopped.

## Host-side artifacts

Large artifacts stay under `$ARISTOTLE_CACHE`, set by `tools/env.sh`. Firmware, decompiled code, and probe dumps are not committed ([Contribution rules](../.github/CONTRIBUTING.md#what-this-project-will-never-include)).

| Path | What |
| --- | --- |
| `$ARISTOTLE_CACHE/firmware/pw4-5.13.6/` (`ARISTOTLE_PW4_FW`) | Unpacked boot files and `rootfs.img`. Read without root using `debugfs` |
| `$ARISTOTLE_CACHE/firmware/native-5.13.6/` | Decompiled `home.jar` (CFR) and Pillow sources, for reading the stock UI |
| `$ARISTOTLE_CACHE/probe/static/` | Output of `tools/probe/`: kernel `Image`, `kernel.config`, DTS files, kernel modules, upstart jobs |
| `$ARISTOTLE_CACHE/tools/` | Ghidra 12.1.4, JDK 21 and JADX from `tools/fetch-tools.sh` |

For example, `debugfs -R 'cat /etc/prettyversion.txt' rootfs.img` reads a stock file without root.

[^1]: Read from firmware 5.13.6 on the host with `tools/probe/`.
[^2]: Observed on our device.
[^3]: Taken from [KOReader](glossary.md#koreader), not verified by our probe.
[^4]: Another public source.
[^5]: A conclusion, not a measurement.
[^6]: Observed on our device and in the emulator.
