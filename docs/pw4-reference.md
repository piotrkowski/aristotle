# Kindle Paperwhite 4 reference

Device and stock-firmware facts for contributors. Source marks apply to the claim or table they follow:

- [fw]: read from firmware 5.13.6 on the host with `tools/probe/`.
- [dev]: observed on our device.
- [ko]: taken from KOReader, not verified by our probe.
- [ext]: another public source.
- [inference]: a conclusion, not a measurement.

## Device

| Property | Value |
| --- | --- |
| Model | Kindle Paperwhite 4 (2018), codename *Moonshine*, platform *Rex* [fw][dev] |
| Board DTBs | `imx6sll-moonshine-wfo` for Wi-Fi, `imx6sll-moonshine-wan` for LTE [fw] |
| SoC | NXP i.MX6 SoloLite-Lite, one Cortex-A9 with NEON and VFPv3 [fw] |
| RAM | 512 MB. The DT `memory` node says 2 GB at `0x80000000`, and the bootloader fixes it up [fw] |
| Firmware | 5.13.6. Version file: `/etc/prettyversion.txt` [dev] |
| Identification | `/proc/usid` holds the serial. `device/extension/guard.sh` checks PW4 codes and firmware [dev][ko] |
| Kernel | `4.1.15-lab126`, `HZ=100` [fw][dev] |

## Boot chain

The update package `update_kindle_all_new_paperwhite_v2_5.13.6.bin`, unpacked with KindleTool [fw]:

| File | What it is |
| --- | --- |
| `imx6sll_rex/u-boot.bin` | U-Boot |
| `imx6sll_rex/bios.bin`, `s-bios.bin` | Resident Falcon BIOS |
| `imx6sll_rex/boot.img` | Kernel, initramfs, and board device trees |
| `rootfs.img.gz` | ext3 root filesystem |

Components have `.sig` signatures [fw]. A [MobileRead serial-log report](https://www.mobileread.com/forums/showthread.php?t=354311) shows `secure_cpu: 1, production: 1, unlocked: 0` and blocked fastboot writes [ext]. Signed boot-image enforcement remains an inference [inference]. Treat boot partitions as read-only.

## Hardware map

From `imx6sll-moonshine-wfo.dts` [fw]:

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

The config is embedded in the kernel and at `/proc/config.gz` on the device [fw].

- Modules are not signed, so out-of-tree modules load if their vermagic matches.
- `DEVTMPFS_MOUNT`, `CGROUPS`, `FUSE_FS`, `EXT4`, `SQUASHFS`, `INPUT_EVDEV`, `PREEMPT` and `SUSPEND` are on. The default cpufreq governor is `performance`.
- USB gadget functions include mass storage, RNDIS, ECM, ACM, FunctionFS and MTP. `g_ether.ko`, `g_mass_storage.ko` and `g_ffs.ko` are modules.
- `DRM_VIVANTE` is built in and unused. The display path is fbdev only.

## Display

From FBInk and sysfs on the device [dev]:

| Property | Value |
| --- | --- |
| Framebuffer device | `/dev/fb0`, id `mxc_epdc_fb` |
| Visible area | 1072 × 1448 at 300 DPI |
| Virtual size | 1088 × 6144 (line length 1088 bytes) |
| Pixel format | 8 bpp grayscale |
| Rotation | `rotate=3` |
| Offsets | `xoffset=0`, `yoffset=0` from `FBIOGET_VSCREENINFO`. FBInk's `viewVertOrigin=4` is its own view adjustment |
| Night mode | hardware inversion available (`canHWInvert=1`) |

The EPDC driver loads waveforms with `request_firmware("eink/waveform")` from `/mnt/wfm/waveform_to_use/`, a partition managed by the upstart jobs `wfmupdate` and `wfmdelete` [fw]. It picks a LUT by the temperature from the FP9929 or BD71827, which sysfs can override. The update ioctl is Lab126's variant of `MXCFB_SEND_UPDATE`, and FBInk's Kindle headers are the reference for its layout.

### Waveform modes and timings

The Rex EPDC driver uses the Zelda numbering from FBInk's `mxcfb-kindle.h`, where A2 is mode 6. The PW2 numbering does not apply. Times were measured with `aristotle-observe bench latency` on a 120 × 120 region toggling black and white, median of 6, on an idle panel [dev]:

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

Other measured updates: A2 120 to 144 ms, full-screen GC16 about 570 ms, 1072 × 724 GL16 partial about 565 ms, and 992 × 60 DU partial about 320 ms [dev].

The driver runs one update at a time, even for non-overlapping regions [dev]. Use the [display thread](ui-framework.md#e-ink-rendering) for partial UI updates.

## Input

- `/etc/upstart/touch` probes Parade and Goodix controllers and loads the matching driver [fw].
- Our Goodix `goodix-ts` uses framebuffer coordinates [dev]. Event numbers vary, including on LTE units [ko]. Find touch by name in `/proc/bus/input/devices`.
- The Goodix node declares only multi-touch axes and has no `BTN_TOUCH`, `ABS_X` or `ABS_Y` [dev]. [benchmarks.md](benchmarks.md#method) lists what an injected contact needs.
- The kernel's `struct input_event` is 16 bytes, with a 32-bit `timeval`. musl's time64 layout is 24 bytes, so `aristotle-hal::input` defines its own.

## Power and light

- Frontlight: `/sys/class/backlight/bl/brightness`, range 0 to 2047. Writes take effect and read back exactly [dev].
- Battery: `/sys/class/power_supply/bd71827_bat/` with `capacity`, `status`, `charging`, `voltage_now` (µV) and `temp` (°C). The charger is `bd71827_ac` [dev].
- Hall sensor: `/sys/bus/platform/drivers/hall_sensor/rex_hall/hall_enable` [ko].
- `preventScreenSaver=1` blocks screensaver and suspend [ko], and disables the power button [dev]. The reader sets it unless `ARISTOTLE_POWERD=1`.
- `powerd` also ignores the button in USB drive mode, including our USB network setup [dev].
- `powerd` requires `bootSplashCleanup` from `com.lab126.hal` [fw]. `ui-boot.sh` sends it when booting without the framework.

## Filesystems

| Mount | Type | Notes |
| --- | --- | --- |
| `/` | ext3, read-only | `/dev/mmcblk1p8` [fw][dev] |
| `/var/local` | ext3, read-write | `/dev/mmcblk1p9`, persistent state: `appreg.db`, `cc.db`, Java prefs [dev] |
| `/var` | tmpfs, 64 MB | lost on reboot, `/var/run` included [dev] |
| `/mnt/us` (also `/mnt/base-us`) | FAT32, about 6.2 GB | the "userstore", exported to the host as USB mass storage [dev] |
| `/mnt/wfm` | | e-ink waveforms [fw] |

## Stock userspace

The C library is glibc 2.20 with the softfp ARM EABI5: soft-float calling convention, VFPv3 and NEON code, `/lib/ld-linux.so.3` [fw]. Static `armv7-unknown-linux-musleabihf` binaries run on it [dev].

Key jobs [fw]:

| Job | Role |
| --- | --- |
| `powerd` | Suspend, screensaver, and battery events |
| `volumd` | USB storage export |
| `lipcd` | Property and event bus |
| `x`, `framework`, `pillow`, `kb` | Window system, Java framework, UI, and keyboard |
| `otaupd`, `otav3` | Firmware updates |

Upstart 0.6.6 [dev] supports neither `.override` files nor `manual` [fw]. A job using `on starting X` completes before X starts. The agent uses this ordering to bind-mount over `/usr/bin/otav3` [dev, emulator].

`stored` reboots after more than five starts lasting under 10 seconds [fw]. `takeover.sh stop` leaves it and related jobs to `framework_ready`.

The framework recursively changes group ownership and permissions under `/var/local`, `/var/log`, `/var/lock`, `/var/tmp`, and `/var/run` [fw]. `/var/tmp` is world-writable without a sticky bit, and `fs.protected_symlinks` is 0 [dev]. See [Security](../device/README.md#security).

LIPC is Lab126's D-Bus property and event service. Its commands include `lipc-probe`, `lipc-get-prop`, `lipc-set-prop`, and `lipc-wait-event`. Names use `com.lab126.<name>` [fw].

The stock reader stores state in `<book>.sdr/` and highlights in `My Clippings.txt` [fw]. See [Library formats](library.md#stock-files-sidecars-and-my-clippingstxt).

## USB network

By default the port runs `g_mass_storage`, and `volumd` exports the userstore [dev]. While it is exported, the device cannot write `/mnt/us`. `g_mass_storage` unloads only after the host has ejected the drive [dev].

The Aristotle agent and KUAL › Aristotle › Start USB network replace it with `g_ether` [dev]:

| Property | Value |
| --- | --- |
| Device MAC, address | `02:61:72:69:73:01`, `usb0` at `192.168.15.244/24` |
| Host MAC, address | `02:61:72:69:73:02`, `192.168.15.201/24`, no gateway or DHCP |
| Firewall | the stock iptables rules drop unsolicited input. The scripts accept `usb0` only |
| SSH | KOReader's `dropbear`, key-only, on `192.168.15.244:22` only |

Use `udisksctl unmount` to eject. Re-entering drive mode requires a physical replug. `udisksctl power-off` also requires a replug [dev]. See [Network policy](../device/README.md#network).

## Taking over the screen

`device/app/takeover.sh start|stop|status` controls the stock GUI. On our device [dev]:

- `stop lab126_gui` stops `cvm`, Pillow, and the keyboard. The script also stops cloud and telemetry jobs. Available RAM rises from 236 MB to 392 MB.
- Xorg, awesome, `lxinit`, and `blanket` remain running.
- SIGSTOP is not a substitute. The watchdog killed the frozen framework.
- Restarting the GUI takes 30 to 60 seconds of boot splash.

KOReader uses `stop lab126_gui` or disables Pillow and freezes awesome [ko]. KOReader reports that `powerd` continues suspend and screensaver control, and `volumd` owns USB export [ko]. Keep these services deliberately.

## Host-side artifacts

Large artifacts stay under `$ARISTOTLE_CACHE`, set by `tools/env.sh`. Firmware, decompiled code, and probe dumps are not committed ([Contribution rules](../.github/CONTRIBUTING.md#what-this-project-will-never-include)).

| Path | What |
| --- | --- |
| `$ARISTOTLE_CACHE/firmware/pw4-5.13.6/` (`ARISTOTLE_PW4_FW`) | Unpacked boot files and `rootfs.img`. Read without root using `debugfs` |
| `$ARISTOTLE_CACHE/firmware/native-5.13.6/` | Decompiled `home.jar` (CFR) and Pillow sources, for reading the stock UI |
| `$ARISTOTLE_CACHE/probe/static/` | Output of `tools/probe/`: kernel `Image`, `kernel.config`, DTS files, kernel modules, upstart jobs |
| `$ARISTOTLE_CACHE/tools/` | Ghidra 12.1.4, JDK 21 and JADX from `tools/fetch-tools.sh` |

For example, `debugfs -R 'cat /etc/prettyversion.txt' rootfs.img` reads a stock file without root.
