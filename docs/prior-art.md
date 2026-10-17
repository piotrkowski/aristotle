# aristotle / Aristotle: GitHub prior-art survey

Target: Kindle Paperwhite 4 (PW4, 2018, "Moonshine", platform "rex", i.MX6SLL), FW 5.13.6, kernel 4.1.15-lab126, soft-float-ABI glibc 2.20 userspace, mxc_epdc_fb, BD71827, cyttsp5 / gt9xx.

Survey date: 2026-10-17. Method: `gh search repos`, `gh search code`, `gh api` (README, trees, file contents), plus WebSearch/WebFetch for MobileRead threads and Amazon's GPL page.

Labels used below:
- **[V]** verified: I read the README, source, or API data myself.
- **[I]** inferred: from search snippets, naming, or reasoning. Check it before relying on it.

Star counts and dates come from the GitHub API on 2026-10-17.

---

## Top 10 most useful for aristotle

| # | Repo | Why (one line) |
|---|------|----------------|
| 1 | [katadelos/eink-emulator](https://github.com/katadelos/eink-emulator) + [katadelos/qemu @ eink-emulator](https://github.com/katadelos/qemu/tree/eink-emulator) | QEMU machine `imx6sll-rex` (PW4) that boots stock PW4 firmware. It models the EPDC, BD71827, Goodix gt9xx, cyttsp5, PXP and the Falcon BIOS. That gives a brick-free dev loop, and it also ships `kindle-kexec` with a `pw4` profile. |
| 2 | [baskerville/plato](https://github.com/baskerville/plato) / [OGKevin/cadmus](https://github.com/OGKevin/cadmus) | The base reader. AGPL-3.0, Rust. Its HAL (framebuffer/, battery/, frontlight/, input.rs) is small and well separated. Cadmus is the actively maintained Plato fork (~1986 commits, pushed today). |
| 3 | [sverrejb/slint-kindle-backend](https://github.com/sverrejb/slint-kindle-backend) | MIT/Apache-2.0 Rust Kindle backend, listed as tested on PW4. Has a ready Rust `mxcfb_update_data_rex` struct with ioctl probing, evdev touch with rotation, RTC-wake suspend, and a SIGSTOP-the-framework launcher. Directly liftable into a Plato Kindle HAL. |
| 4 | [NiLuJe/FBInk](https://github.com/NiLuJe/FBInk) `eink/mxcfb-kindle.h` | The authoritative Kindle EPDC ABI reference: Rex/Zelda/MTK update structs, waveform-mode numbering, nightmode, `MXCFB_*` ioctls. GPL-3.0, so use it as a reference, not as linked code. |
| 5 | [koreader/koreader](https://github.com/koreader/koreader) + [koreader-base](https://github.com/koreader/koreader-base) | Production-grade PW4 support: device table (sysfs paths for battery, frontlight, hall sensor), `refresh_rex` waveform policy, framework-stop launcher, powerd/suspend handling. AGPL-3.0, the same licence as Plato. |
| 6 | [albu/yb-reader](https://github.com/albu/yb-reader) | Rust (AGPL) "reading OS" for jailbroken Kindles. Its takeover mode (Amazon's own `DONT_START_FRAMEWORK` flag + its own upstart job + a fallback ladder) is the safest staged path to "replace Amazon userspace" without touching the kernel. Built for PW5 (MTK). |
| 7 | [KindleModding/kindlemodding.github.io](https://github.com/KindleModding/kindlemodding.github.io) (kindlemodding.org) | Per-service LIPC property catalogue (~90 `com.lab126.*` services, including powerd and appmgrd), upstart dependency diagram, SDK/toolchain docs. This is the map of what we are replacing. |
| 8 | [akemnade/linux](https://github.com/akemnade/linux) branch `kobo/drm-merged-7.1` | Out-of-tree mainline DRM `mxc-epdc` driver (with waveform loader), maintained up to 7.1, plus imx6sll Kobo Clara HD/2E DTs on the same SoC. The long-term "own kernel" base, but see the secure-boot caveat. |
| 9 | [H4rryp0tt3r/kindle-dash](https://github.com/H4rryp0tt3r/kindle-dash) | A real "replace the whole Kindle userspace" OS in Rust: runit as PID 1, own rootfs, own kernel, written for PW2. Its DEVICE.md documents SDP/fastboot/IDME bootmode recovery. It is the closest structural template for aristotle. |
| 10 | [zacharydenton/boko](https://github.com/zacharydenton/boko) | Rust (GPL-3.0, which can be combined with AGPL-3.0) reader for MOBI, AZW3 and KFX that writes EPUB. The cleanest way to make the owner's MOBI files first-class in an EPUB-centric Plato pipeline. |

Honourable mentions: [arkq/openlipc](https://github.com/arkq/openlipc) and [clintharrison/liblipc-go](https://github.com/clintharrison/liblipc-go) (LIPC is D-Bus underneath), [akemnade/kobo-firmware-extractor](https://github.com/akemnade/kobo-firmware-extractor), [fread-ink/inkwave](https://github.com/fread-ink/inkwave), [KindleModding/kindle_modding_recovery_project](https://github.com/KindleModding/kindle_modding_recovery_project).

---

## Critical cross-cutting findings (read these first)

1. **The PW4 is secure-boot locked. This is the biggest risk to "own kernel".** [V, MobileRead]
   - A PW4 serial U-Boot log shows `secure_cpu: 1, production: 1, unlocked: 0`.
   - Marek (notmarek), 2024-01-11: "The pw4 has locked fastboot and uboot - you can't flash anything". A second user confirms that fastboot erase and flash are disabled. Source: [MobileRead t=354311](https://www.mobileread.com/forums/showthread.php?t=354311).
   - The lockdown and signing code was found in an Amazon GPL drop and re-uploaded as [katadelos/ufbl-features-lab126](https://github.com/katadelos/ufbl-features-lab126). Source: [MobileRead t=343387](https://www.mobileread.com/forums/showthread.php?t=343387). A forum member calls it RSA-PSS and "secure boot the right way".
   - [I] On a HAB-closed i.MX6SLL, SDP (USB serial downloader) only runs images signed with Amazon's keys. The "load a patched U-Boot over SDP" trick from kindle-dash (PW2) and the KT3 recovery write-up very likely does not work on a production PW4.
   - **Consequence:** booting our own kernel on PW4 most plausibly goes through **kexec from the running stock kernel**. katadelos ships a loadable kexec module with a `pw4` profile (4.1.15, "Falcon") in [eink-emulator/kindle-kexec](https://github.com/katadelos/eink-emulator/tree/HEAD/kindle-kexec). Its README says: "The scope is these emulator guests. Physical Kindle hardware is not verified." It also only re-kexecs the stock kernel today.

2. **PW4 eMMC goes through a "Falcon BIOS".** [V, QEMU source]
   - Stock PW4 DTBs use `compatible = "falcon_blk"` for storage. The `s-bios` (0x9ffc0000) and `f-bios` (0x9ffd0000) firmware sit resident in RAM.
   - The QEMU Rex machine rewrites that node to `fsl,imx6sl-usdhc` so a plain USDHC driver works. A mainline or own kernel will need either the same DT change (if the Falcon layer is not enforcing anything) or a Falcon block driver. The kexec README notes that "Falcon firmware remains in memory… the next kernel can then rebuild the BIOS I/O mappings".
   - Source: [hw/arm/imx6sll-rex.c](https://github.com/katadelos/qemu/blob/eink-emulator/hw/arm/imx6sll-rex.c).

3. **Recovery on PW4 is weak.**
   - What we can rely on: a 1.8 V UART (U-Boot console visible), the stock diags partition, the userspace recovery KMRP, and Amazon's "3 framework restarts, then reboots" safety net.
   - Fastboot flashing is locked [V]. SDP with unsigned images is very likely unusable [I].
   - So: **never write to the kernel, boot, diags, IDME or bootloader regions of the eMMC**. Keep the stock rootfs bootable. Make every userspace takeover reversible from `/mnt/us`, for example with a USB-visible flag file.

4. **ABI: we are not stuck with soft-float.** [V]
   - Small correction: the PW2+ pre-5.16.3 userspace is the `kindlepw2` TC (`arm-kindlepw2-linux-gnueabi`). That is the softfp ABI with VFP/NEON available, not pure soft-float.
   - Fully static musl hard-float binaries run fine on top of it. KinNovel's README says its static binary runs on both the softfp (<5.16.3) and hf firmwares. slint-kindle-backend deploys `armv7-unknown-linux-musleabihf` via `cargo zigbuild` and is reported working on PW4.
   - Since aristotle replaces the whole userspace, choose **`armv7-unknown-linux-musleabihf` static** (or a full armhf rootfs). Use `armv7-unknown-linux-gnueabi` + the koxtoolchain `kindlepw2` sysroot only while we still have to `dlopen` Amazon libraries such as `liblipc.so`.
   - [I] Note: PW2 is Cortex-A9 VFPv3-D16 (kindle-dash DEVICE.md). The i.MX6SLL is Cortex-A9 too, so avoid `+vfp4` and `+neon` assumptions until we have checked `/proc/cpuinfo`.

5. **MOBI support is easy.** [V]
   - Plato already routes `mobi`, `fb2`, `xps` and `txt` to MuPDF (`crates/core/src/document/mod.rs`). MuPDF has had a native MOBI handler since 2022-08-19 (`source/html/mobi.c`, commit "Add MOBI format").
   - [I] I did not check MuPDF's coverage of HUFF/CDIC or of the KF8 halves of AZW3. If it falls short, convert MOBI/AZW3 to EPUB once with `boko`.

---

## PRIORITY 1: ciscoriordan/kindling (in-depth)

**Repo:** https://github.com/ciscoriordan/kindling [V]

| Field | Value |
|---|---|
| What | "The missing Kindle toolkit. Dictionaries, books, comics. Single Rust binary." A drop-in **kindlegen replacement** that **builds** MOBI/AZW3 (KF7 + KF8) from EPUB/OPF. Also: comics (CBZ/CBR/folders) to MOBI, StarDict export, EPUB2/3 export (from OPF/EPUB input), EPUB repair, in-place MOBI metadata rewrite, structural dump, a "lookup simulator" for PW5 dictionary popups, KDP pre-flight validation (ported epubcheck rules). |
| Language | Rust (edition 2024, MSRV 1.85), ~2.96 MB of Rust source, plus shell scripts |
| License | MIT (© 2026 Francisco Riordan) |
| Crate | `kindling-mobi` on crates.io (lib name `kindling`), v0.45.6 (2026-09-21) |
| Activity | Created 2026-04-06. 327 commits. Last push 2026-10-17 (today). Last commits 2026-10-12 (webtoon fixes). Releases about every few days. |
| Maturity | Single main author (323/327 commits, 3 drive-by contributors). 58 stars, 8 forks. CI, a large test suite (`src/tests.rs` 464 KB, parity tests against committed kindlegen reference files), and on-device verification policy (issues close only after a real-device check). The README says AI-assisted contributions are accepted. |
| Devices / firmware | Output targets Kindle readers in general. The README cites verification on PW3 (5.16.2.1.1) and PW5 (5.18/5.19.2), plus Scribe and Colorsoft notes. **It never runs on the Kindle.** It is a host tool for macOS, Linux x86_64 and Windows. |
| Dependencies | clap, scraper/ego-tree, quick-xml, lightningcss, image, zip, flate2, rayon, regex, memchr |

**Architecture** [V, from README "Project layout" and the tree]:
- `mobi.rs` (363 KB): PalmDB + record 0 + EXTH writer, UTF-8/tag-safe 4096-byte record splitter.
- `kf8.rs`: KF8 section, BOUNDARY, FDST, skeleton/fragment INDX.
- `indx.rs`, `ordt.rs`, `cncx.rs`: dictionary orth index and collation tables.
- `palmdoc.rs`, `huffcdic.rs`, `huffdic_encode.rs`: compression codecs. HUFF/CDIC can be read and (behind a flag) written.
- `exth.rs`, `vwi.rs`: metadata and integer encodings.
- `opf.rs`, `epub.rs`, `extracted.rs`, `nav.rs`, `fonts.rs`: EPUB input side.
- `comic.rs` (128 KB), `profile.rs`, `moire.rs`, `cbr.rs`: comics pipeline.
- `mobi_dump.rs` (`pub fn dump_mobi(path) -> String`), `mobi_check.rs`, `mobi_rewrite.rs`: reader-side tools. They are structural only. There is no "MOBI to renderable HTML/EPUB" API.
- `epub_build.rs`: EPUB2/EPUB3 output. **Input is OPF/EPUB, not MOBI**, so kindling cannot convert MOBI to EPUB.
- `stardict.rs`: StarDict 2.4.2 bundle writer.
- `thumbnail.rs`: writes `system/thumbnails/thumbnail_<ASIN>_<type>_portrait.jpg` onto a mounted stock Kindle.

**How it could help aristotle:**
- (+, small) **MOBI format knowledge in MIT-licensed Rust.** The README's MOBI header/EXTH tables are good reference material, and so is `huffcdic.rs` (a clean HUFF/CDIC decoder, cross-checked against KindleUnpack, calibre and libmobi). MIT code can be vendored into an AGPL project. Useful only if we write our own Rust MOBI reader instead of using MuPDF.
- (+, small) **StarDict export.** If the owner wants dictionaries, kindling can turn an EPUB/OPF dictionary source into StarDict, which a Plato-style reader can consume. Plato already supports StarDict-format dictionaries [I, from Plato's `dictionary/` module].
- (−) Everything else targets **Amazon's stock reader**: kindlegen parity, lock-screen covers, EXTH 501/113 quirks, Kindle popup lookup routing, thumbnail cache, KDP rules. Once Amazon's userspace is gone, these firmware behaviours no longer exist.
- (−) It does not run on the device, has nothing to do with framebuffer, input, power or system services, and has no EPUB/MOBI renderer.

**Verdict:** kindling is a high-quality, very active **host-side publishing tool**, not a device or userspace project. It is **not useful for building aristotle**. At most it is reference material for the MOBI container (MIT, vendorable) and a StarDict converter. For the owner's DRM-free MOBI files, use MuPDF through Plato (already wired up) or `boko` (MOBI/AZW3 to EPUB in Rust). Don't spend more time on kindling.

---

## 1. Plato ports and other Rust/Zig/Go/C readers or UI frameworks for Kindle framebuffers

**Main finding:** no Plato port to Kindle exists on GitHub [V]. I checked all 138 forks of baskerville/plato, searched for "plato kindle", and searched Plato issues for "kindle". Separately, a web search turned up MobileRead discussion and only a reMarkable port.

| Repo | Lang / Licence | Last push | Notes |
|---|---|---|---|
| [baskerville/plato](https://github.com/baskerville/plato) | Rust / AGPL-3.0 (LICENSE-AGPLv3; GitHub shows NOASSERTION) | 2026-03-30 (v0.9.45, 2026-01-11) | [V] Kobo only. HAL modules: `framebuffer/{kobo1,kobo2,mxcfb_sys,ion_sys,sunxi_sys,linuxfb_sys}.rs`, `battery/{kobo,fake}.rs`, `frontlight/{standard,natural,premixed}.rs`, `input.rs`, `rtc.rs`, `lightsensor`, `device.rs`. Its `MxcfbUpdateDataV2` (Kobo Mark 7) equals the **Kindle Rex struct minus the two trailing `hist_bw/hist_gray_waveform_mode` u32s**. The ioctl number encodes the struct size, so the Kindle port needs a new struct and ioctl plus Kindle waveform numbering. Document dispatch: EPUB/HTML native; DjVu via djvulibre; everything else (PDF, CBZ, **MOBI**, FB2, XPS, TXT) via MuPDF. |
| [OGKevin/cadmus](https://github.com/OGKevin/cadmus) | Rust / Plato licence (AGPL [I]) | 2026-10-17 | [V] "Cadmus is a fork of Plato". ~1986 commits, 49 stars, CI, codecov, docs site, crowdin. Has `crates/` split, build scripts for mupdf, harfbuzz, djvulibre, gumbo, sqlite (kobo/native/generic). Kobo only. Probably a better-maintained base than upstream Plato. Evaluate it before forking. |
| [LinusCDE/plato](https://github.com/LinusCDE/plato) | Rust / AGPL | 2025-09-17 | [V metadata] reMarkable port of Plato. Shows how to add a non-Kobo HAL to Plato. |
| [sverrejb/slint-kindle-backend](https://github.com/sverrejb/slint-kindle-backend) | Rust / MIT OR Apache-2.0 | 2026-09-28 | [V] 77 stars, 72 commits, v0.7.1. Slint software-renderer backend for jailbroken Kindles. Tested on PW1, PW3, **PW4**, KT4, KOA3. Code to reuse: `src/framebuffer/ffi.rs` (fb + `UpdateRequestRex`/Zelda/MTK structs), `framebuffer/mod.rs` (probes the Legacy, Rex (80-byte), Zelda (88), MTK (96) ioctl variants on first refresh and caches the winner), `touch.rs`, `power.rs` (RTC `wakealarm` + `/sys/power/state` "mem"), `wakeup.rs`. Its getting_started.md has the canonical SIGSTOP launcher (`lipc-set-prop com.lab126.pillow disableEnablePillow disable; killall -STOP awesome cvm KPPMainApp`). Its deploy uses `cargo zigbuild --target armv7-unknown-linux-musleabihf`. |
| [albu/yb-reader](https://github.com/albu/yb-reader) | Rust / AGPL-3.0 | 2026-10-14 | [V] 156 commits, 0 stars, created 2026-09. Full reader (own typesetter "yread" for EPUB/FB2/TXT, MuPDF for PDF/CBZ, no MOBI). Targets PW5 (MTK, armhf musl). The README says PW4-class i.MX needs different ioctls and input. **Takeover mode is the valuable part:** `/mnt/us/DONT_START_FRAMEWORK` (Amazon's own pre-start check in `framework.conf`), `/etc/upstart/yb-reader.conf` started on `stopped framework`, plus a boot script with a hang watchdog, boot audit, crash counter, and automatic fallback to stock. Verified surviving without cvm: powerd, wifid/wpa_supplicant/udhcpc, volumd/fsp, dropbear. Never started: Xorg, awesome, lxinit, pillow, kb, KPPMainApp, webreader. |
| [metabot/zephyr](https://github.com/metabot/zephyr) | Zig + crengine (C++) / no licence | 2026-07-29 | [V] EPUB reader for jailbroken Kindles (Oasis 2). Pure-Zig backend: mmap fb, mxcfb ioctl, evdev, sysfs frontlight/battery. Static `arm-linux-musleabihf`. Clean architecture (pure screens, then render commands, then a refresh controller). Early stage. |
| [H4rryp0tt3r/kindle-dash](https://github.com/H4rryp0tt3r/kindle-dash) | Rust + shell / no licence | 2026-10-10 | [V] "Dash OS": a minimal OS for **PW2** (stock lab126 3.0.35 kernel with built-in EPDC + waveform, runit PID 1, own ext3 rootfs on p1). docs/DEVICE.md covers the eMMC map, IDME `bootmode`, SDP (15a2:0063) and fastboot (1949:d0e0) recovery. A reproducible-build discipline. Binaries are kept in a private pins repo. |
| [mmnnwjw/KinNovel](https://github.com/mmnnwjw/KinNovel) | Rust / GPL-3.0 | 2026-10-16 | [V] A Rust Kindle app (light-novel client) using FBInk FFI. Static binary that runs on both softfp and hf firmware (PW2+). Tested on PW5. |
| [Bill13579/buck](https://github.com/Bill13579/buck) | Rust / Apache-2.0 | 2022-06-09 | [V] Music player for Kobo and Kindle Touch. Old. |
| [simsor/go-kindle](https://github.com/simsor/go-kindle), [efskap/shiny-kindle](https://github.com/efskap/shiny-kindle) | Go / MIT, n/a | 2021, 2020 | [V metadata] Go app libraries for Kindle; shiny fork for PW. Stale. |
| [DogParty/libsdl2-kindlehf](https://github.com/DogParty/libsdl2-kindlehf) | C / Zlib | 2026-09-16 | [V metadata] SDL2 video driver for Kindle (hf firmware). |
| [zampierilucas/kindle-button-mapper-rs](https://github.com/zampierilucas/kindle-button-mapper-rs) | Rust / GPL-3.0 | 2026-10-11 | [V] evdev mapper daemon running under upstart. Useful evdev patterns. |
| [Aveygo/KindleDashboard](https://github.com/Aveygo/KindleDashboard), [jehna/kindle-information-radiator](https://github.com/jehna/kindle-information-radiator), [justinledwards/ferrink-home-assistant](https://github.com/justinledwards/ferrink-home-assistant) | Rust | 2025–2026 | Small Rust dashboards with their own fb code. Minor. |
| [BandarLabs/Cobalt](https://github.com/BandarLabs/Cobalt) | Rust / AGPL-3.0 | 2026-10-16 | [V] Kobo app SDK + launcher + per-app sandbox + simulator (400 stars). Has `kobo-hal` with mxcfb refresh. Good HAL and app-model reference. Kobo only. |
| [dgalli1/EinkHome](https://github.com/dgalli1/EinkHome) | Rust + Slint | 2026-09-25 | [V] PocketBook home replacement with a clean HAL crate split (`eh_hal`, backends for inkview, SDL and linuxfb incl. `mxcfb.rs`). |
| [Quill-OS/quill](https://github.com/Quill-OS/quill) (InkBox) | C++/Qt | 2026-08 | [V] Full OS replacement for Kobos plus **Kindle Touch** (`kt` kernel target). Own kernel, init, power daemon (`inkbox-power-daemon`), Qt QPA for Kobo/Kindle e-ink. The README says "expect no support". Good reference for an "own init + power daemon" design. |
| [fread-ink](https://github.com/fread-ink) | various | 2017–2023 | [V metadata] Historic Kindle 4/5 distro attempt (mainline 4.19/5.8 for K4, fastboot-only u-boot, inkwave). Dormant. |

## 2. E-ink / framebuffer libraries

| Repo | Lang / Licence | Last push | Notes |
|---|---|---|---|
| [NiLuJe/FBInk](https://github.com/NiLuJe/FBInk) | C / GPL-3.0 | 2026-08-06 | [V] 551 stars. `eink/mxcfb-kindle.h` is the reference for Kindle. **Rex (PW4/KT4) vs Kobo differences, from the header:** `struct mxcfb_update_data_rex { rect; waveform_mode; update_mode; update_marker; int temp; uint flags; int dither_mode; int quant_bit; alt_buffer_data; uint32 hist_bw_waveform_mode; uint32 hist_gray_waveform_mode; }` sent via `MXCFB_SEND_UPDATE_REX = _IOW('F', 0x2E, struct mxcfb_update_data_rex)`. Legacy PW2/PW3 `mxcfb_update_data` puts the `hist_*` fields *before* `temp`. Zelda (KOA2) adds `ts_pxp/ts_epdc`. Kindle waveform modes: INIT 0, DU 1, GC16 2, GC16_FAST 3, A2 4, GL16 5, GL16_FAST 6, DU4 7, REAGL 8, REAGLD 9, GL4 0xA, GL16_INV 0xB. On Rex/Zelda, KOReader uses the ZELDA constants (GL16=3, GLR16=4, GLD16=5, A2=6, GCK16=8, GLKW16=9). Also `MXCFB_WAIT_FOR_UPDATE_COMPLETE` = `_IOWR('F',0x2F, {marker, collision_test})`, `MXCFB_SET_NIGHTMODE 0x4A` with `NIGHTMODE_STRIDE_DEFAULT_REX 138`, `MXCFB_GET_TEMPERATURE 0x38`, `FB_POWERDOWN_DELAY` (PW4 min 0). |
| [koreader/koreader-base](https://github.com/koreader/koreader-base) `ffi/framebuffer_mxcfb.lua` | Lua / AGPL-3.0 | 2026-10-16 | [V] `refresh_rex()`: REAGL handling (GLR16 / GLD16 + `EPDC_FLAG_USE_ZELDA_REGAL`), A2/DU fast paths, and the comment "Rex … same driver as Zelda, just a slightly smaller mxcfb_update_data". The best reference for a refresh *policy*. |
| [sublipri/fbink-rs](https://github.com/sublipri/fbink-rs) | Rust / GPL-3.0 | 2024-07-23 | [V metadata] Rust bindings (thin + safe) to FBInk. [Szybet/fbink-sys](https://github.com/Szybet/fbink-sys) is a raw -sys crate. Both are GPL via FBInk. For an AGPL project, linking GPL-3 FBInk is licence-compatible (AGPLv3 §13 / GPLv3 §13), but a native Rust implementation (see slint-kindle-backend) is lighter. |
| [canselcik/libremarkable](https://github.com/canselcik/libremarkable) | Rust / MIT | 2026-06-16 | [V metadata] Rust mxcfb/refresh framework for reMarkable 1 (i.MX6SL EPDC). A good example of Rust struct/ioctl modelling. |
| [pl-semiotics/mxc_epdc_fb_damage](https://github.com/pl-semiotics/mxc_epdc_fb_damage) (+ [NiLuJe fork](https://github.com/NiLuJe/mxc_epdc_fb_damage)) | C / GPL-2.0 | 2021 | [V metadata] Kernel module exposing EPDC damage to userspace. Only relevant if we ever run an unmodified client, such as a VNC or X-like compositor. |
| einkfb crates | | | [V] Nothing relevant. `einkfb` is the pre-PW Kindle 2/3/4 driver (e.g. [dororo42/k4refresh](https://github.com/dororo42/k4refresh)). PW4 uses `mxc_epdc_fb`. |

## 3. Kindle low-level documentation (lipc, appmgrd, powerd, upstart, waveforms)

- **[KindleModding/kindlemodding.github.io](https://github.com/KindleModding/kindlemodding.github.io)** (kindlemodding.org, 429 stars, pushed today) [V]
  - `content/kindle-apps-and-services/` has a page per LIPC service (~90 `com.lab126.*` services, e.g. `com.lab126.powerd` props `state`, `deferSuspend`, `rtcWakeup`, `wakeUp`, `suspendGrace`, `flRawIntensity`; `com.lab126.appmgrd` props `load`, `startdefault`, `savecontext`…; plus pillow, winmgr, cmd, wifid, volumd, blanket, etc.). The descriptions are mostly "TODO", but the property names and types are listed.
  - `static/kindle-hacking/upstart-diagram.html` is a Mermaid diagram of all upstart jobs (taken from PW6; the same family as 5.13).
  - `kindle-dev/` covers the SDK, KPM, awesome WM and GTK tutorials.
  - The OTA-files page is still a placeholder.
- **[arkq/openlipc](https://github.com/arkq/openlipc)** (C, MIT, 2017) [V]: a reverse-engineered `openlipc.h` for `liblipc.so`, plus reimplemented `lipc-get-prop` and `lipc-probe`. "LIPC seems to be an IPC library based on D-Bus". It builds `--host=armv7a-softfp-linux-gnueabi`. Origin: [bfabiszewski/openlipc](https://github.com/bfabiszewski/openlipc) (2013).
- **[clintharrison/liblipc-go](https://github.com/clintharrison/liblipc-go)** (Go, MIT, 2026-02) [V]: **a pure D-Bus LIPC implementation that does not link liblipc**. Supports int/string get and set. Hashmap properties are not supported (they use shared memory). This proves a Rust `zbus`-based LIPC client is feasible, so we would not need liblipc or the softfp ABI.
- **[tachibana-shin/lipc-rust](https://github.com/tachibana-shin/lipc-rust)** (Rust, no licence, 2026-07) [V]: safe Rust FFI over `liblipc.so` (props, events, hasharrays, stubs for host tests).
- [notmarek/openlipclua](https://github.com/notmarek/openlipclua) / [NiLuJe/openlipclua](https://github.com/NiLuJe/openlipclua): Lua bindings used by KOReader. [KindleModding/utild](https://github.com/KindleModding/utild): a sample LIPC service in C++.
- **powerd / suspend without the framework:** [V]
  - KOReader `platform/kindle/koreader.sh`: `--framework_stop` runs `stop lab126_gui` / `/etc/init.d/framework stop`. Otherwise it disables pillow and SIGSTOPs awesome and volumd.
  - [shezik/SuspendHack](https://github.com/shezik/SuspendHack) and [mireq/KOReader-without-framework…](https://github.com/mireq/KOReader-without-framework-support-for-Amazon-Kindle) mock the KAF interface so powerd still handles the power button without cvm.
  - slint-kindle-backend `power.rs` goes straight to sysfs (`/sys/class/rtc/rtcN/wakealarm` + `/sys/power/state`) while powerd keeps running.
  - yb-reader confirms powerd survives the absence of cvm.
- **PW4 hardware map from KOReader** `frontend/device/kindle/device.lua` [V]:
  - `KindlePaperWhite4`: `isRex`, 300 dpi, `touch_dev=/dev/input/event2` (event3 on LTE because of an SX9310 proximity sensor).
  - Frontlight: `/sys/class/backlight/bl/brightness`.
  - Battery: `/sys/class/power_supply/bd71827_bat/{capacity,charging,status}`.
  - Hall sensor: `/sys/bus/platform/drivers/hall_sensor/rex_hall/hall_enable`.
  - PW4 serial prefixes: `0PP, 0T1–0T7, 0TJ–0TN, 102, 103, 16Q–16V`.
- **Waveforms:**
  - [V-forum] MobileRead (Nephiel and others, PW2-era): `.wbf` files live in `/var/local/eink` and `/opt/eink/images/`. `/var/local/eink/dont_update` and `/var/local/eink/update_flash` control flashing the waveform to the panel. Panel IDs must match.
  - [I] Verify the PW4 location on the device (`/var/local/eink`, `/sys/devices/…/mxc_epdc_fb/…`).
  - Tools:
    - [fread-ink/inkwave](https://github.com/fread-ink/inkwave) (C, GPL-2.0): `.wbf` to `.wrf`, the format NXP/mainline EPDC drivers use.
    - [aceisace/python-waveform-parser](https://github.com/aceisace/python-waveform-parser).
    - [KOLANICH-tools/inkwave.py](https://github.com/KOLANICH-tools/inkwave.py) (Kaitai spec).
    - [akemnade/kobo-firmware-extractor](https://github.com/akemnade/kobo-firmware-extractor): extracts waveforms from hidden Kobo/Tolino partitions.
  - kindle-dash (PW2) bakes `epdc_E60_V220.fw` (gzip, so 1.24 MB) into the kernel with `CONFIG_FIRMWARE_IN_KERNEL` and warns that a placeholder waveform gives a "white-screen freeze".
- **EPDC hardware:** the mainline driver in akemnade/linux (`epdc_regs.h`, `epdc_hw.c`, `epdc_update.c`, `epdc_waveform.c`) is the most readable open documentation of the i.MX6 EPDC registers and waveform header layout [V].

## 4. Mainline / alternative kernels for imx6sll and e-readers

- **[akemnade/linux](https://github.com/akemnade/linux)** [V]
  - 222 branches. The latest DRM branch is `kobo/drm-merged-7.1` (last commit 2026-08-24).
  - `drivers/gpu/drm/mxc-epdc/` holds the DRM driver (`DRM_MXC_EPDC`, module `mxc_epdc_drm`, Kconfig "i.MX EPD Controller", copyright 2020 Andreas Kemnade). Waveform handling uses the NXP `.fw` header format with a temperature-range table, auto-waveform LUT and thermal-zone temperature.
  - DTs: `imx6sll-kobo-clarahd{,-b}.dts`, `imx6sll-kobo-clara2e-{a,b}.dts`, `imx6sll-kobo-librah2o.dts`, `e60k02.dtsi`, `e70k02.dtsi`, plus Tolino imx6sl.
  - Upstream status [V-web]: [RFC PATCH 0/6 "drm: EPDC driver for i.MX6", 2022-02-06](https://lore.kernel.org/all/Yi0A5inJh0Whkh5n@latitude/T/). Clara HD and Clara 2E DTs are upstream. The EPDC driver is still out of tree. Kemnade also has [akemnade/drmminitest](https://github.com/akemnade/drmminitest) (tiny static DRM/touch test tools for initrds) and [akemnade/uboot-kobo](https://github.com/akemnade/uboot-kobo).
- **Mainline drivers for PW4 parts** [V via code search, torvalds/linux]: `drivers/input/touchscreen/cyttsp5.c` exists. Goodix (gt9xx family) is mainline (`goodix.c`). `rohm,bd71828-pmic.yaml` mentions BD71827 [I: probably a "compatible with" note, so the BD71827 charger/fuel gauge may need work]. imx6sll SoC support (`imx6sll.dtsi`) is upstream.
- **Yocto:** [akemnade/meta-kobo-clara-hd](https://github.com/akemnade/meta-kobo-clara-hd) (fork of kevinior/meta-kobo-clara-hd, MIT): a working Yocto image for the Clara HD on the Kemnade kernel, with RAUC updates.
- **postmarketOS:** [I] Kindle 4 (amazon-yoshi) has a wiki page. A Kobo Clara HD port appears in pmaports history (search hit), but the wiki now redirects and I could not verify it. [Ouroboroshen/posh_kindle](https://github.com/Ouroboroshen/posh_kindle) runs a pmOS userspace on the stock kernel. There is no PW4 pmOS port [V by search].
- **Amazon GPL sources** [V]:
  - The catalogue is at https://digprjsurvey.amazon.com/csad/help/node/200203720 (amazon.com/gp/help/...nodeId=200203720 returned 503).
  - Paperwhite 10th-gen entries include `Kindle_src_5.13.1 … 5.13.7`.
  - **`https://kindledownloads.s3.amazonaws.com/Kindle_src_5.13.6_3731980038.tar.gz`** returns HTTP 200 (S3 Last-Modified 2021-05-11). The same tarball is listed under several devices, so it is a multi-device drop.
  - The latest PW10 release listed is `Kindle_src_5.18.1.1.1_4769680023.tar.gz`.
  - The kexec README notes that you need the exact matching vendor source and config to build modules (vermagic and modversions are enforced).
- **Community re-uploads of Lab126 sources:** [katadelos/linux-3.0.35-lab126](https://github.com/katadelos/linux-3.0.35-lab126), [-3.10.53](https://github.com/katadelos/linux-3.10.53-lab126), [-2.6.31](https://github.com/katadelos/linux-2.6.31-lab126), [katadelos/uboot-2009.08-lab126](https://github.com/katadelos/uboot-2009.08-lab126), and [katadelos/ufbl-features-lab126](https://github.com/katadelos/ufbl-features-lab126) (the secure-boot and fastboot-lockdown code). **No 4.1.15-lab126 re-upload was found**, so we get it from the Amazon tarball. [metacrito/linux-kindle-pw2](https://github.com/metacrito/linux-kindle-pw2) covers PW2.
- **u-boot / barebox:** [katadelos/barebox](https://github.com/katadelos/barebox) (branch `kata/bringup-work`) and [qwert-Xx/kindle4nt-linux](https://github.com/qwert-Xx/kindle4nt-linux) (mainline 6.6 + barebox + Alpine on K4NT). Both are only useful for unlocked older devices.
- **kexec:** [katadelos/eink-emulator/kindle-kexec](https://github.com/katadelos/eink-emulator/tree/HEAD/kindle-kexec) [V]
  - Static kexec-tools plus a loadable kexec module. It is adapted from amonakov/kexec-module (ARM) and fabianishere/kexec-mod.
  - It has a saved `configs/pw4.config` and a `pw4` profile ("4.1.15, Falcon"). `falcon.py` regenerates Falcon BIOS I/O cursor resets.
  - Verified in the emulator only, and only re-kexecs the stock kernel. **This is the most promising route to an own kernel on a secure-boot-locked PW4.**

## 5. Jailbreak and tooling ecosystem (relevant to 5.13.6)

- **[KindleModding](https://github.com/KindleModding)** org [V]
  - Jailbreaks: [WinterBreak](https://github.com/KindleModding/WinterBreak) (1601 stars), [LanguageBreak](https://github.com/notmarek/LanguageBreak) (≤5.16.2.1.1, which covers 5.13.6), Winterbreak2, SpringBreak, AdBreak, SpiderCat (5.16.3–5.19.5).
  - [Hotfix](https://github.com/KindleModding/Hotfix) (227 stars, GPL-3.0): keeps the jailbreak across OTAs.
  - [KPM](https://github.com/KindleModding/KPM) package manager, [KindleTool fork](https://github.com/KindleModding/KindleTool), [koxtoolchain fork](https://github.com/KindleModding/koxtoolchain).
  - [kindle-sdk](https://github.com/KindleModding/kindle-sdk) (GPL-3.0): downloads target firmware, copies its libraries into the koxtoolchain sysroot, and adds pkg-config and a meson cross file.
  - [architect](https://github.com/KindleModding/architect): runs one binary on both armel and armhf.
  - [KPP_Patch](https://github.com/KindleModding/KPP_Patch), [kindle_modding_recovery_project](https://github.com/KindleModding/kindle_modding_recovery_project).
  - The SDK/TC table says: **`kindlepw2` = "Kindle PW2 & everything since on FW <5.16.3"**, which is the PW4 on 5.13.6, and `kindlehf` = FW ≥5.16.3.
- **[koreader/koxtoolchain](https://github.com/koreader/koxtoolchain)** (122 stars, 2026-08-02) [V]: crosstool-ng scripts for `kindle5`, `kindlepw2`, `kindlehf`, `kobo`, and others.
- **[NiLuJe/KindleTool](https://github.com/NiLuJe/KindleTool)** (C, GPL-3.0, 535 stars, pushed today) [V metadata]: builds and extracts Kindle update packages (OTA/recovery). eink-emulator uses it to import recovery packages.
- **KUAL / MRPI / USBNetwork:**
  - [V metadata] [NiLuJe/KUAL_Booklet](https://github.com/NiLuJe/KUAL_Booklet) is a staging mirror of the bitbucket ixtab/kindlelauncher. Canonical KUAL and MRPI builds are NiLuJe's "Snapshots" on MobileRead (not GitHub).
  - [notmarek/kindle-usbnetlite](https://github.com/notmarek/kindle-usbnetlite) (137 stars): minimal USBNet + dropbear installed via MRPI (`;log mrpi`). The full USBNetwork is in NiLuJe's MobileRead snapshots.
  - [PEKI](https://github.com/KindleModding/PEKI) is a KUAL installer.
- **Rust targets used for Kindle** [V]:
  - `armv7-unknown-linux-musleabihf` static: slint-kindle-backend (PW4-tested), yb-reader, KinNovel, zephyr (Zig `arm-linux-musleabihf`).
  - `armv7-unknown-linux-gnueabihf`: kindle-button-mapper (only fits hf firmware).
  - `notmarek/koreader-companion` (Rust) ships a `Dockerfile.kindlepw2` that builds against the softfp TC.
  - [progzone122/kff](https://github.com/progzone122/kff) (Rust, GPL-3.0) automates TC + SDK install (kindlehf-oriented).
  - [I] For a static Rust binary on PW4 5.13.6, `armv7-unknown-linux-musleabihf` is the low-friction choice. Use `armv7-unknown-linux-gnueabi` (softfp, glibc ≥2.17 is fine with 2.20) only when dlopen'ing Amazon libraries.
- **Emulation:** [katadelos/eink-emulator](https://github.com/katadelos/eink-emulator) [V]
  - Python CLI, no licence file. Created 2026-08-10, 127 commits, pushed 2026-10-15, 35 stars.
  - Supports Kindle 4 through Scribe Colorsoft, including **"Kindle Paperwhite 4 | PW4 | Moonshine"**, plus Kobos and Tolinos.
  - PW4 needs `u-boot.bin, boot.img, rootfs.img, s-bios.bin, bios.bin`. The Rex disk image is rebuilt with the stock GPT layout: kernel, recovery, keys, diags, persist, miscdata, snapshot, rootfs, varlocal, userstore. IDME is placed in eMMC boot partition 2.
  - `create --mrpi` pre-installs KUAL, MRPI and the jailbreak. Ships dropbear for kindle5, kindlepw2 and kindlehf. SSH over emulated USB, QMP screendump.
  - The QEMU fork (`katadelos/qemu`, branch `eink-emulator`) adds `hw/arm/imx6sll-rex.c`, `hw/display/imx_epdc.c`, `hw/i2c/{bd71827,cyttsp5,goodix_gt9xx,fp9929,lm3695,max14656}.c`, `hw/misc/imx6sl_pxp.c` and `hw/usb/chipidea.c`.

## 6. Recovery / unbrick for PW4

- **[V] PW4 specifics** (MobileRead [t=354311](https://www.mobileread.com/forums/showthread.php?t=354311)):
  - The serial console works (CP2102 at 1.8 V). U-Boot prints `secure_cpu: 1, production: 1, unlocked: 0` and drops to "Enter fastboot mode".
  - Fastboot needs the "Lab126 USB download gadget" enumerated, but erase and flash are locked. `fastboot setvar bootmode diags` fails.
- **[V] General Kindle recovery repos (mostly older or unlocked devices):**
  - [fgarancini/kindle-kt3-sy69jl-recovery](https://github.com/fgarancini/kindle-kt3-sy69jl-recovery): KT3 via SDP (15a2:0063) to a patched MollySophia u-boot to fastboot flash, Oct 2026.
  - [MollySophia/imx6_android_u-boot_kindle](https://github.com/MollySophia/imx6_android_u-boot_kindle).
  - [yifanlu/Fastboot-Kindle](https://github.com/yifanlu/Fastboot-Kindle).
  - [fread-ink/fread-installer-u-boot](https://github.com/fread-ink/fread-installer-u-boot).
  - [boundarydevices/imx_usb_loader](https://github.com/boundarydevices/imx_usb_loader) (LGPL-2.1); NXP `uuu` is the modern equivalent.
  - [rh45-one/Unbrick-Kindle](https://github.com/rh45-one/Unbrick-Kindle): PW2 serial recovery menu (export FAT, update from update*.bin).
  - kindle-dash `docs/DEVICE.md`: IDME `bootmode` (main/diags/fastboot/uboot/factory/reset). Never write LBA 760, which holds serial, MAC, sec and pcbsn.
- **[V] Userspace-level recovery that still works on locked devices:**
  - [KindleModding/kindle_modding_recovery_project](https://github.com/KindleModding/kindle_modding_recovery_project) (KMRP, GPL-3.0): runs before the lab126 upstart job and stalls boot if USB is plugged in.
  - Amazon's own framework-restart and reboot cascade.
  - yb-reader's fallback ladder (watchdog, boot audit, crash counter, remove flag, stock framework).
- **[I] Implication:** treat the kernel, boot, diags, keys and IDME as read-only forever. Our recovery story has to be built in userspace: a flag file on `/mnt/us`, USB-plug escape, and ssh. Practise everything first in eink-emulator's PW4 machine.

## 7. Other "replace the Kindle userspace" efforts

- **KOReader framework-stop mode** [V]: `koreader.sh --framework_stop` stops `lab126_gui` (upstart) and restarts the framework on exit. The non-stop mode disables pillow (`lipc-set-prop com.lab126.pillow disableEnablePillow disable`) and SIGSTOPs awesome, cvm (sysv) and volumd.
- **yb-reader takeover** [V]: see §1. It boots straight into the reader with no framework, via the `DONT_START_FRAMEWORK` flag and its own upstart job. This is the best documented "Amazon userspace is gone except powerd/wifid/volumd" configuration.
- **kindle-dash (PW2)** [V]: replaces everything, including init (runit) and the kernel. Not possible as-is on a locked PW4, but its rootfs and service layout is a good model for phase 2 if kexec works.
- **[pascalw/kindle-dash](https://github.com/pascalw/kindle-dash)** (Shell, MIT, 1390 stars, K4NT) [V metadata]: a framework-stopped dashboard (fbink + rtcwake). Its power-saving tricks are reusable.
- **Alpine on Kindle** [V metadata]: [schuhumi/alpine_kindle](https://github.com/schuhumi/alpine_kindle) (519 stars, GPL-3.0, 2019) + [alpine_kindle_kual](https://github.com/schuhumi/alpine_kindle_kual) (2026-03), [vifetch/alpine-linux-kindle](https://github.com/vifetch/alpine-linux-kindle), [katadelos/alpine-kindle-chroot-builder](https://github.com/katadelos/alpine-kindle-chroot-builder). All are chroots on the stock kernel. Proof that a full non-Amazon userspace runs, but they keep Amazon's init.
- **InkBox / Quill OS** [V]: a full replacement (own kernel, init, power daemon, Qt UI) for Kobos and Kindle Touch. Unmaintained-ish.
- **fread.ink** [V metadata]: an early full-distro attempt (K4/K5). Dormant.
- **Cobalt (Kobo)**, **EinkHome (PocketBook)** [V]: modern Rust app platforms on other vendors' devices. Good architectural references (HAL crates, sandboxed apps, simulator).

---

## Suggested next actions (derived)

1. Build eink-emulator with the PW4 profile and import the 5.13.6 PW4 firmware. Stand up a dev loop with a static `armv7-unknown-linux-musleabihf` binary that paints via `MXCFB_SEND_UPDATE_REX`.
2. Fork Plato or Cadmus and add a `kindle_rex` HAL:
   - framebuffer: Rex struct + Zelda waveform numbering (from FBInk / KOReader).
   - battery: bd71827 sysfs.
   - frontlight: `/sys/class/backlight/bl`.
   - input: evdev, with event2/event3 probing.
   - suspend: powerd LIPC over D-Bus or direct sysfs.
   - Borrow code from slint-kindle-backend (MIT/Apache).
3. Phase-1 takeover: copy the yb-reader method (the `DONT_START_FRAMEWORK` flag + an upstart job + a fallback ladder). Keep powerd, wifid and volumd at first, and replace them one by one, using the kindlemodding LIPC catalogue as the spec.
4. Download `Kindle_src_5.13.6_3731980038.tar.gz` and extract the 4.1.15-lab126 tree and config. It is needed for kernel modules (kexec, any helper drivers) and to understand `mxc_epdc_fb`, `falcon_blk`, the hall sensor, bd71827, and the waveform loading path.
5. Phase-2 kernel: test the kindle-kexec `pw4` module in the emulator, then very carefully on hardware. Only after that, try kexec into an akemnade 7.x kernel with an imx6sll PW4 DT, either using the `falcon_blk` to `fsl,imx6sl-usdhc` rewrite or a Falcon-aware block driver.
