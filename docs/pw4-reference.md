# Kindle Paperwhite 4 reference

Detailed facts about the device and its stock firmware. `AGENTS.md` carries the short list of things that will bite you; this file has the full detail.

Every fact here is marked by source:

- **[fw]** read from the stock firmware 5.13.6 update package on the host. Reproduce with `probe/host/`.
- **[dev]** observed on our device.
- **[ko]** taken from KOReader's device support code, not yet confirmed by our own probe.

## Device

| | |
| --- | --- |
| Model | Kindle Paperwhite 4 (2018), codename *Moonshine*, platform *Rex* [fw][dev] |
| Board DTBs | `imx6sll-moonshine-wfo` (Wi-Fi only), `imx6sll-moonshine-wan` (LTE), plus the generic `imx6sll-rex` and a stub `imx6sll-jaeger-wfo` [fw] |
| SoC | NXP i.MX6 SoloLite-Lite (imx6sll), one Cortex-A9 core with NEON and VFPv3, PL310 L2 cache [fw] |
| RAM | 512 MB. The DT `memory` node says 2 GB at `0x80000000`; the bootloader fixes it up. The Falcon BIOS sits at `0x9ffc0000`, just below 512 MB [fw] |
| Firmware | 5.13.6 (373198 038), jailbroken, KUAL and MRPI installed, KOReader installed [dev] |
| Kernel | `Linux 4.1.15-lab126 #1 SMP PREEMPT Thu Apr 8 09:28:53 UTC 2021`, gcc 4.9.1, `HZ=100` [fw][dev] |

## Boot chain

Contents of the update package (`update_kindle_all_new_paperwhite_v2_5.13.6.bin`, unpacked with KindleTool) [fw]:

| File | What it is |
| --- | --- |
| `imx6sll_rex/u-boot.bin` | U-Boot |
| `imx6sll_rex/bios.bin`, `s-bios.bin` | Lab126 "Falcon" BIOS: resident firmware loaded high in RAM (`CONFIG_FALCON_BIOS_ADDR=0x9ffc0000`, storage BIOS at `0x9ffd0000`) |
| `imx6sll_rex/boot.img` | Android v0 boot image: LZO-compressed zImage, initramfs cpio, and a wrapper DTB in the "second" slot holding one DTB per board |
| `rootfs.img.gz` | ext3 root filesystem (`/dev/mmcblk1p8`, mounted read-only) |

Every component ships with a `.sig` (RSA-256). Prior art reports the PW4 as secure-boot locked (`secure_cpu: 1, production: 1, unlocked: 0` in a serial U-Boot log; fastboot flash and erase are blocked), so we assume signed boot images are enforced. See `docs/prior-art.md`.

Kernel command line: `console=ttymxc0,115200 consoleblank=0 uart_at_4m root=/dev/mmcblk1p8 rootwait quiet`, giving a serial console on UART1 at 115200 baud [fw].

### Falcon

The kernel is built with `CONFIG_FALCON`, `FALCON_BLK` (with `BOOTPART` and `MULTISEG`), `FALCON_DRV` and `FALCON_WRAPPER`. The eMMC controller node `usdhc@02194000` has compatible `falcon_blk` rather than the standard `fsl,imx7d-usdhc`. `falcon.ko` and `CONFIG_HIBERNATION_IMX6SLL=y` are also present [fw].

Our reading, still an inference: the eMMC is accessed through Lab126's resident BIOS, which also does fast hibernation and resume. A mainline kernel would need its own eMMC path, and that route is unexplored.

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

The touch controller differs between units. `/etc/upstart/touch` powers the touch rail through PMIC register `0x12`, probes I2C2 at `0x24` (Parade) and then at `0x5d`/`0x14` (Goodix), and loads the matching driver [fw]. The probe reports which variant our unit has.

## Display

From FBInk and sysfs on the device [dev]:

| | |
| --- | --- |
| Framebuffer device | `/dev/fb0`, id `mxc_epdc_fb` |
| Visible area | 1072 × 1448 at 300 DPI |
| Virtual size | 1088 × 6144 (line length 1088 bytes) |
| Pixel format | 8 bpp grayscale |
| Rotation | `rotate=3` |
| Offsets | `xoffset=0`, `yoffset=0` from `FBIOGET_VSCREENINFO`; FBInk reports `viewVertOrigin=4`, its own view adjustment |
| Night mode | hardware inversion available (`canHWInvert=1`) |

Waveforms are loaded by the EPDC driver through `request_firmware("eink/waveform")` from `/mnt/wfm/waveform_to_use/`, a dedicated partition. Upstart jobs `wfmupdate` and `wfmdelete` manage it [fw]. The driver selects a LUT by temperature, read from FP9929 or BD71827, and supports a temperature override through sysfs.

The update ioctl is Lab126's variant of `MXCFB_SEND_UPDATE`. FBInk's Kindle headers are the working reference for its layout.

## Kernel configuration highlights

The full config is embedded in the kernel (`IKCONFIG_PROC=y`, so it is also at `/proc/config.gz` on the device) [fw]:

- No module signing, so out-of-tree modules load if their vermagic matches.
- `DEVTMPFS` and `DEVTMPFS_MOUNT`, `CGROUPS`, `FUSE_FS`, `EXT4`, `SQUASHFS` (zlib), `INPUT_EVDEV`.
- USB gadget: configfs with mass storage, RNDIS, ECM, ACM, FunctionFS and MTP. `g_ether.ko`, `g_mass_storage.ko` and `g_ffs.ko` ship as modules.
- `CFG80211` and `MAC80211` are built in. Wi-Fi uses Broadcom's out-of-tree `bcmdhd`, not `brcmfmac`.
- `DRM=y` with `DRM_VIVANTE`, an unused leftover from the BSP. The display path is fbdev only.
- `CPU_FREQ_DEFAULT_GOV_PERFORMANCE`, `PREEMPT`, `SUSPEND`, `PM_SLEEP`, and an i.MX6SLL hibernation hook.

## Filesystems

| Mount | Type | Notes |
| --- | --- | --- |
| `/` | ext3, read-only | `/dev/mmcblk1p8` [fw][dev] |
| `/var/local` | ext3, read-write | `/dev/mmcblk1p9`, persistent state: `appreg.db`, `cc.db`, Java prefs [dev] |
| `/var` | tmpfs, 64 MB | [dev] |
| `/mnt/us` (also `/mnt/base-us`) | FAT32, about 6.2 GB | "userstore", exported to the host as USB mass storage [dev] |
| `/mnt/wfm` | | e-ink waveforms [fw] |

## Stock userspace

- **Libraries and ABI:** glibc 2.20, softfp ARM EABI5 (soft-float calling convention, VFPv3 and NEON code; `/lib/ld-linux.so.3`) [fw]. Static `armv7-unknown-linux-musleabihf` binaries run on it.
- **Init:** upstart. The probe captures the full job list.

Job graph, simplified [fw]:

```
system (udevd) → filesystems → system_setup → lab126 → lab126_gui
    → x (lxinit + awesome) → framework (cvm Java) → framework_ready
    → pillow (WebKit chrome) / kb / kfxreader / webreader / ...
```

| Group | Jobs |
| --- | --- |
| Core we probably keep at first | `powerd` (suspend, screensaver, battery events), `volumd` (USB mass-storage export), `lipcd` (IPC daemon on D-Bus), `deviced`, `wifid` and `wifis` (wpa_supplicant), `syslog` |
| GUI stack we replace | `x`, `lab126_gui`, `framework` (Java `cvm`, heap `-Xmx25m`–`49m`), `pillow` (runs as user `framework`), `kb` (on-screen keyboard), `kppreader` (`/app/bin/AaMenuApplication`, React Native), `whisperstore`, `progressivedownloads` |
| Native renderers | `kfxreader`, `kfxview`, `webreader` |
| Bloat | `todo`, `tmd`, `dmld`, `fastmetrics`, `clickstream_logging`, `perfd`, `stackdumpd`, `otaupd`, `otav3`, `rcm`, `archive`, `scanner`, `stored`, `asr`, `ivona_ttsd`, `ttsorchestrator`, `playermgr`, `btd`, `btmanagerd`, `btfd`, `acsbtfd`, `ffsd`, `demd`, `cmd`, `phd`, `wand` |
| Developer jobs, inert | `sshd`, `usbnetd`, `usbnet-autostart`, `adb-autostart`, `vnc`. They are gated on flag files in `/mnt/base-us` (`ENABLE_USBNET`, `ENABLE_ADB`, `ENABLE_VNC`), and their binaries under `/usr/local` are absent in production images. |

### IPC

`lipc` is Lab126's property and event bus on top of D-Bus, served by `lipc-daemon`. On-device tools: `lipc-probe`, `lipc-get-prop`, `lipc-set-prop`, `lipc-send-event`, `lipc-wait-event`. Services are named `com.lab126.<name>`, for example `com.lab126.powerd`, `com.lab126.appmgrd` and `com.lab126.pillow`. The probe dumps every service and property.

### Reader sidecars and clippings [fw]

The stock reader keeps per-book state in `<book>.sdr/` next to each book, as two files in the "KRDS" serialisation (`ReaderContentSDK.jar`). MOBI 7 books use `.mbs`/`.mbp1`, KF8 books `.azw3f`/`.azw3r`, KFX books `.yjf`/`.yjr` and PDFs `.pdt`/`.pds`. They hold the last and furthest read position, bookmarks, highlight and note positions, note text and font preferences. The legacy `.mbp` is only read once for migration and then deleted. `My Clippings.txt` is written by `Clipping` in `ReaderSDK-impl.jar`, with localised labels for en, en_GB, de, es, fr, it, ja, nl, pt, ru and zh; there is no Polish bundle in 5.13.6. Layouts, and what Aristotle imports, are in `docs/library.md`.

### How KOReader takes over the screen [ko]

KOReader's `koreader.sh` on our device shows the existing practice:

- Its optional framework-stop mode runs `stop lab126_gui` and restarts it on exit.
- Without that mode, it disables Pillow (`lipc-set-prop com.lab126.pillow disableEnablePillow disable`) and sends `SIGSTOP` to `awesome`.
- It stops `stored webreader kfxreader kfxview todo tmd rcm archive scanner otav3 otaupd` to free RAM.
- PW4 paths it uses: frontlight `/sys/class/backlight/bl/brightness`; battery `/sys/class/power_supply/bd71827_bat/{capacity,charging,status}`; hall sensor `/sys/bus/platform/drivers/hall_sensor/rex_hall/hall_enable`; touch `/dev/input/event2` on Wi-Fi units and `event3` on LTE units (the extra SX9310 proximity sensor shifts the numbering).
- Verified on our unit [dev]: frontlight `bl`, range 0–2047 (read 20 at a dim reading setting; writes take effect and read back exactly); battery `bd71827_bat` with `capacity`, `status`, `charging`, `voltage_now` (µV) and `temp` (°C); charger `bd71827_ac`; touch is Goodix `goodix-ts` on `event2`, axes 0–1072 × 0–1448, identical to framebuffer coordinates. `aristotle-hal::power` and `::input` use these.

## Host-side artifacts

Large artifacts stay out of git. The scripts expect them at these defaults, in the cache directory `$ARISTOTLE_CACHE` (`devloop/env.sh`):

| Path | What |
| --- | --- |
| `$ARISTOTLE_CACHE/firmware/pw4-5.13.6/` (`ARISTOTLE_PW4_FW`) | The 5.13.6 update package unpacked with KindleTool: `imx6sll_rex/` and `rootfs.img`. `debugfs -R 'ls -p /path' rootfs.img` reads it without root. The emulator setup takes its boot files from here. |
| `$ARISTOTLE_CACHE/firmware/native-5.13.6/` | Decompiled `home.jar` (CFR) and Pillow sources, for reading the stock UI |
| `$ARISTOTLE_CACHE/probe/static/` | Output of `probe/host/` (kernel `Image`, `kernel.config`, DTS files, kernel modules, upstart jobs) |
| `$ARISTOTLE_CACHE/tools/` | Ghidra 12.1.4, JDK 21 and JADX from `tools/fetch-tools.sh` |

## Measured e-ink timings

Measured on our device [dev]: `aristotle-hello` with stock UI idle, `MXCFB_SEND_UPDATE_REX` followed by `MXCFB_WAIT_FOR_UPDATE_COMPLETE`, and ambient temperature.

| Update | Region | Time to complete |
| --- | --- | --- |
| GC16, full (flashing) | 1072 × 1448 | about 570 ms |
| GL16, partial | 1072 × 724 | about 565 ms |
| DU, partial | 992 × 60 | about 320 ms |

### Waveform numbering and per-mode latency

The PW4 (Rex) EPDC driver uses the **Zelda numbering** from FBInk's `mxcfb-kindle.h`, not the PW2 one: A2 is mode 6, not 4. Measured with `aristotle-bench` on a 120 × 120 region, toggling black and white, median of 6 [dev]:

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

The times above are for an idle panel; A2 alone measures 120–144 ms. Under a stream of updates the driver behaves as **one update at a time**: submitting while an earlier update is still on the panel blocks the ioctl until that update finishes, whether or not the regions overlap, so each streamed update costs about two waveform durations. Several updates submitted back to back *before* the first starts are merged and finish together. Full measurements and the resulting design (a display thread with latest-wins damage) are in `docs/eink-rendering.md`.

## Taking over the screen

`devloop/kindle/takeover.sh start|stop|status` [dev]:

- `stop lab126_gui` takes down the framework (`cvm`), Pillow and the keyboard. The script also stops 13 cloud and telemetry jobs (`stored todo tmd rcm archive scanner otav3 otaupd dmld fastmetrics clickstream_logging perfd`). Available RAM goes from 236 MB to 392 MB.
- `x` (Xorg, awesome, `lxinit`, and `blanket`, about 22 MB together) keeps running, because its job only stops on `stopping dbus or stop_x`. We leave it on purpose for now: `blanket -t splash screensaver langpicker blankwindow` looks like what draws the screensaver when `powerd` suspends, so removing it should wait until we own power management. This is inferred from the command line, not tested.
- `powerd`, `volumd`, `wifid`, `lipcd`, `dbus`, our USB network and SSH keep running.
- `start lab126_gui` brings the stock UI back, showing the boot splash with a progress bar while the framework starts.
- Stroke latency is the same with the stock GUI stopped (p50 194–197 ms against 199 ms), so the panel, not CPU contention, sets it.
