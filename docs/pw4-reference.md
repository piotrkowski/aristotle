# Kindle Paperwhite 4 reference

Facts about the device and its stock firmware. `AGENTS.md` has the short list of things that break naive code. This file has the detail.

Each fact is marked by its source:

- **[fw]** read from the stock firmware 5.13.6 update package on the host. `probe/host/` reproduces it.
- **[dev]** observed on our device.
- **[ko]** taken from KOReader's device support code and not yet confirmed by our own probe.

## Device

| | |
| --- | --- |
| Model | Kindle Paperwhite 4 (2018), codename *Moonshine*, platform *Rex* [fw][dev] |
| Board DTBs | `imx6sll-moonshine-wfo` (Wi-Fi only), `imx6sll-moonshine-wan` (LTE), the generic `imx6sll-rex` and a stub `imx6sll-jaeger-wfo` [fw] |
| SoC | NXP i.MX6 SoloLite-Lite (imx6sll), one Cortex-A9 core with NEON and VFPv3, PL310 L2 cache [fw] |
| RAM | 512 MB. The DT `memory` node says 2 GB at `0x80000000`, and the bootloader fixes it up [fw] |
| Firmware | 5.13.6 (373198 038), jailbroken, with KUAL, MRPI and KOReader [dev]. `/etc/prettyversion.txt` starts with `Kindle 5.13.6` [dev] |
| Identification | `/proc/usid` holds the serial number. Unless it starts with `B` or `9`, characters 4–6 are the device code, `0PP` on ours [dev]. KOReader lists 21 PW4 codes [ko], which `devloop/kindle/aristotle-usbnet/guard.sh` checks along with the firmware |
| Kernel | `Linux 4.1.15-lab126 #1 SMP PREEMPT Thu Apr 8 09:28:53 UTC 2021`, gcc 4.9.1, `HZ=100` [fw][dev] |

## Boot chain

The update package `update_kindle_all_new_paperwhite_v2_5.13.6.bin`, unpacked with KindleTool [fw]:

| File | What it is |
| --- | --- |
| `imx6sll_rex/u-boot.bin` | U-Boot |
| `imx6sll_rex/bios.bin`, `s-bios.bin` | Lab126 "Falcon" BIOS, resident firmware at `0x9ffc0000` (storage BIOS at `0x9ffd0000`), in the top 256 KiB of RAM |
| `imx6sll_rex/boot.img` | Android v0 boot image: LZO-compressed zImage, initramfs cpio, and a wrapper DTB in the "second" slot with one DTB per board |
| `rootfs.img.gz` | ext3 root filesystem |

Each component has a `.sig` (RSA-256). MobileRead users report the PW4 as secure-boot locked, with `secure_cpu: 1, production: 1, unlocked: 0` in a serial U-Boot log and fastboot flash and erase blocked ([t=354311](https://www.mobileread.com/forums/showthread.php?t=354311)). We assume signed boot images are enforced.

The kernel command line is `console=ttymxc0,115200 consoleblank=0 uart_at_4m root=/dev/mmcblk1p8 rootwait quiet` [fw].

The kernel is built with Lab126's Falcon options (`CONFIG_FALCON`, `FALCON_BLK`, `FALCON_DRV`, `FALCON_WRAPPER`) and `CONFIG_HIBERNATION_IMX6SLL=y`. The eMMC node `usdhc@02194000` is compatible with `falcon_blk` [fw]. We infer that the eMMC goes through the resident BIOS, which also handles fast hibernation, so a mainline kernel would need its own eMMC path.

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
| Bluetooth | same chip, `amzn,bt_pwr_ctrl` | | |
| Cover sensor | `amzn,rex_hall` | | |
| Crash log | `fsl,mxc_ramoops` | | |

## Kernel configuration

The config is embedded in the kernel and at `/proc/config.gz` on the device [fw].

- Modules are not signed, so out-of-tree modules load if their vermagic matches.
- `DEVTMPFS_MOUNT`, `CGROUPS`, `FUSE_FS`, `EXT4`, `SQUASHFS`, `INPUT_EVDEV`, `PREEMPT` and `SUSPEND` are on. The default cpufreq governor is `performance`.
- USB gadget functions include mass storage, RNDIS, ECM, ACM, FunctionFS and MTP. `g_ether.ko`, `g_mass_storage.ko` and `g_ffs.ko` are modules.
- `DRM_VIVANTE` is built in and unused. The display path is fbdev only.

## Display

From FBInk and sysfs on the device [dev]:

| | |
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

The Rex EPDC driver uses the Zelda numbering from FBInk's `mxcfb-kindle.h`, where A2 is mode 6. The PW2 numbering does not apply. Times were measured with `aristotle-bench` on a 120 × 120 region toggling black and white, median of 6, on an idle panel [dev]:

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

A2 alone measures 120–144 ms. Larger regions take a little longer at room temperature: a full-screen GC16 flash about 570 ms, a 1072 × 724 GL16 partial about 565 ms, and a 992 × 60 DU partial about 320 ms, from an early bring-up test since removed [dev].

The driver runs one update at a time [dev]. A submit blocks while an earlier update is still on the panel, whether or not the regions overlap, so each streamed update costs about two waveform durations. Updates submitted back to back before the first starts are merged and finish together. `docs/eink-rendering.md` has the measurements and the display thread built on them.

## Input

- The touch chip differs between units. `/etc/upstart/touch` powers the touch rail through PMIC register `0x12`, probes I2C2 at `0x24` (Parade), then at `0x5d` and `0x14` (Goodix), and loads the matching driver [fw].
- Our unit has Goodix `goodix-ts` on `/dev/input/event2`, with axes 0–1072 × 0–1448, the same as framebuffer coordinates [dev]. LTE units have an extra SX9310 proximity sensor, which moves touch to `event3` [ko]. Find the device by name in `/proc/bus/input/devices`.
- The Goodix node declares only multi-touch axes and has no `BTN_TOUCH`, `ABS_X` or `ABS_Y` [dev]. `docs/native-vs-aristotle.md` lists what an injected contact needs.
- The kernel's `struct input_event` is 16 bytes, with a 32-bit `timeval`. musl's time64 layout is 24 bytes, so `aristotle-hal::input` defines its own.

## Power and light

- Frontlight: `/sys/class/backlight/bl/brightness`, range 0–2047. Writes take effect and read back exactly [dev].
- Battery: `/sys/class/power_supply/bd71827_bat/` with `capacity`, `status`, `charging`, `voltage_now` (µV) and `temp` (°C). The charger is `bd71827_ac` [dev].
- Hall sensor: `/sys/bus/platform/drivers/hall_sensor/rex_hall/hall_enable` [ko].
- `lipc-set-prop com.lab126.powerd preventScreenSaver 1` keeps `powerd` from blanking to the screensaver and suspending [ko]. The reader app and the USB network script set it too.

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

Init is upstart. The job graph, simplified [fw]:

```
system (udevd) → filesystems → system_setup → lab126 → lab126_gui
    → x (lxinit + awesome) → framework (cvm Java) → framework_ready
    → pillow (WebKit chrome) / kb / kfxreader / webreader / ...
```

| Group | Jobs |
| --- | --- |
| Core services | `powerd` (suspend, screensaver, battery events), `volumd` (USB mass-storage export), `lipcd` (IPC daemon on D-Bus), `deviced`, `wifid` and `wifis` (wpa_supplicant), `syslog` |
| GUI stack | `x`, `lab126_gui`, `framework` (Java `cvm`, heap `-Xmx25m`–`49m`), `pillow` (runs as user `framework`), `kb` (on-screen keyboard), `kppreader` (`/app/bin/AaMenuApplication`, React Native), `whisperstore`, `progressivedownloads` |
| Native renderers | `kfxreader`, `kfxview`, `webreader` |
| Cloud, telemetry and extras | `todo`, `tmd`, `dmld`, `fastmetrics`, `clickstream_logging`, `perfd`, `stackdumpd`, `otaupd`, `otav3`, `rcm`, `archive`, `scanner`, `stored`, `asr`, `ivona_ttsd`, `ttsorchestrator`, `playermgr`, `btd`, `btmanagerd`, `btfd`, `acsbtfd`, `ffsd`, `demd`, `cmd`, `phd`, `wand` |
| Developer jobs, inert | `sshd`, `usbnetd`, `usbnet-autostart`, `adb-autostart`, `vnc`. They wait for flag files in `/mnt/base-us` (`ENABLE_USBNET`, `ENABLE_ADB`, `ENABLE_VNC`), and their binaries under `/usr/local` are missing from production images |

`lipc` is Lab126's property and event bus on D-Bus, served by `lipc-daemon` and driven with `lipc-probe`, `lipc-get-prop`, `lipc-set-prop`, `lipc-send-event` and `lipc-wait-event`. Services are named `com.lab126.<name>`. The probe dumps all of them with their properties.

The stock reader keeps per-book state in `<book>.sdr/` and writes highlights to `My Clippings.txt` [fw]. `docs/library.md` describes both formats.

## USB network

By default the port runs `g_mass_storage`, and `volumd` exports the userstore [dev]. While it is exported, the device cannot write `/mnt/us`. `g_mass_storage` unloads only after the host has ejected the drive [dev].

The Aristotle agent and KUAL › Aristotle › Start USB network replace it with `g_ether` [dev]:

| | |
| --- | --- |
| Device MAC, address | `02:61:72:69:73:01`, `usb0` at `192.168.15.244/24` |
| Host MAC, address | `02:61:72:69:73:02`, `192.168.15.201/24`, no gateway or DHCP |
| Firewall | the stock iptables rules drop unsolicited input. The scripts accept `usb0` only |
| SSH | KOReader's `dropbear`, key-only, on `192.168.15.244:22` only |

Eject on the host with `udisksctl unmount`. `udisksctl power-off` takes the device off the bus, and the network gadget then needs a physical replug [dev]. After an eject the device leaves mass storage with the cable still in, and only a replug enters it again [dev]. `docs/network.md` covers the link and the Wi-Fi policy.

## Taking over the screen

KOReader's `koreader.sh` either runs `stop lab126_gui` and restarts it on exit, or disables Pillow (`lipc-set-prop com.lab126.pillow disableEnablePillow disable`) and sends `SIGSTOP` to `awesome`. It also stops several cloud jobs to free RAM [ko].

The stock GUI stack uses about 200 MB of 490 MB, `cvm` alone 125 MB RSS [dev]. `devloop/kindle/takeover.sh start|stop|status` takes the screen for Aristotle [dev]:

- `stop lab126_gui` takes down the framework (`cvm`), Pillow and the keyboard. The script also stops 12 cloud and telemetry jobs (`stored todo tmd rcm archive scanner otav3 otaupd dmld fastmetrics clickstream_logging perfd`). Available RAM goes from 236 MB to 392 MB.
- `x` (Xorg, awesome, `lxinit`, `blanket`, about 22 MB) keeps running, because its job stops only on `stopping dbus or stop_x`. Judging by its command line, `blanket` draws the screensaver when `powerd` suspends, so `x` stays until we own power management. This is not tested.
- `powerd`, `volumd`, `wifid`, `lipcd`, `dbus`, the USB network and SSH keep running.
- `start lab126_gui` brings the stock UI back through the boot splash.
- The flag that keeps the agent's watchdog from restarting the stock GUI, `/var/run/aristotle-takeover`, is on the tmpfs. A reboot during a takeover returns to the stock UI with the watchdog on.
- Stroke latency is the same with the stock GUI stopped (p50 194–197 ms against 199 ms), so the panel sets it.

## Host-side artifacts

Large artifacts stay in the cache directory `$ARISTOTLE_CACHE` (`devloop/env.sh`) as local working copies. Firmware, decompiled code and probe dumps are never committed (`.github/CONTRIBUTING.md`, "What this project will never include").

| Path | What |
| --- | --- |
| `$ARISTOTLE_CACHE/firmware/pw4-5.13.6/` (`ARISTOTLE_PW4_FW`) | The 5.13.6 update package unpacked with KindleTool: `imx6sll_rex/` and `rootfs.img`. `debugfs -R 'ls -p /path' rootfs.img` reads it without root. The emulator setup takes its boot files from here |
| `$ARISTOTLE_CACHE/firmware/native-5.13.6/` | Decompiled `home.jar` (CFR) and Pillow sources, for reading the stock UI |
| `$ARISTOTLE_CACHE/probe/static/` | Output of `probe/host/`: kernel `Image`, `kernel.config`, DTS files, kernel modules, upstart jobs |
| `$ARISTOTLE_CACHE/tools/` | Ghidra 12.1.4, JDK 21 and JADX from `tools/fetch-tools.sh` |
