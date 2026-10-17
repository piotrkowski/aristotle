# Related projects

What the device bundle ships, with licences, is in
[THIRD_PARTY.md](../THIRD_PARTY.md).

## Built on

- [Plato](https://github.com/baskerville/plato) (Rust, AGPL-3.0) renders the
  book pages, EPUB with its own engine and the other formats with MuPDF. We use
  `plato-core` from our fork
  ([piotrkowski/plato](https://github.com/piotrkowski/plato), branch
  `aristotle`) and none of Plato's Kobo interface. See [reader.md](reader.md).
- [KOReader](https://github.com/koreader/koreader) (AGPL-3.0). The device code
  started from its PW4 sysfs paths (battery, frontlight, hall sensor), the
  `refresh_rex` waveform policy in
  [koreader-base](https://github.com/koreader/koreader-base), and its way of
  stopping the stock framework. The dev loop uses KOReader's `dropbear`.
- [FBInk](https://github.com/NiLuJe/FBInk) (GPL-3.0). Its `eink/mxcfb-kindle.h`
  is our reference for the Kindle EPDC ABI. We link no FBInk code.
- [eink-emulator](https://github.com/katadelos/eink-emulator) and its
  [QEMU fork](https://github.com/katadelos/qemu/tree/eink-emulator) emulate
  the PW4 (`imx6sll-rex`) well enough to boot the stock firmware.
  `tools/emulator/` sets them up for trying risky changes.

## Device knowledge

- [slint-kindle-backend](https://github.com/sverrejb/slint-kindle-backend)
  (MIT or Apache-2.0), a Rust Kindle backend tested on the PW4, with the Rex
  update struct, ioctl probing and evdev touch.
- [yb-reader](https://github.com/albu/yb-reader) (AGPL-3.0), a Rust reader for
  newer Kindles that takes over from the stock system through Amazon's
  `DONT_START_FRAMEWORK` flag and can fall back to it.
- [kindlemodding.org](https://kindlemodding.org) lists the LIPC services,
  their properties and the upstart jobs.
- [openlipc](https://github.com/arkq/openlipc) and
  [liblipc-go](https://github.com/clintharrison/liblipc-go) show that LIPC,
  the stock IPC, is D-Bus underneath and works without `liblipc.so`.
- Amazon's GPL source release for firmware 5.13.6
  (`Kindle_src_5.13.6_3731980038.tar.gz`) has the 4.1.15-lab126 kernel tree
  and configuration, which kernel modules need.

## Jailbreak and tools

- [KindleModding](https://github.com/KindleModding) publishes the jailbreaks,
  Hotfix and the documentation the Aristotle install relies on. LanguageBreak
  covers firmware up to 5.16.2.1.1, so 5.13.x too.
- KUAL, the launcher for the dev-loop extensions, and MRPI, the package
  installer. Both come from NiLuJe's snapshots on MobileRead.
- [KindleTool](https://github.com/NiLuJe/KindleTool) unpacks firmware update
  packages.
- [koxtoolchain](https://github.com/koreader/koxtoolchain) has the reference
  toolchains; `kindlepw2` matches the PW4 below firmware 5.16.3. Aristotle
  builds static musl binaries with Zig and Rust instead.

## A kernel of our own

The PW4 is secure-boot locked ([pw4-reference.md](pw4-reference.md)). Our own
kernel would start through kexec from the stock one.

- [kindle-kexec](https://github.com/katadelos/eink-emulator/tree/HEAD/kindle-kexec)
  is a loadable kexec module with a PW4 profile, tested only in the emulator.
- [akemnade/linux](https://github.com/akemnade/linux) has an out-of-tree
  mainline DRM driver for the i.MX6 EPDC, with device trees for Kobo readers on
  the same SoC.

## Other readers and systems

- [cadmus](https://github.com/OGKevin/cadmus), a maintained Plato fork for
  Kobo.
- [kindle-dash](https://github.com/H4rryp0tt3r/kindle-dash), a replacement OS
  for the PW2 with its own init, rootfs and kernel, possible there because the
  PW2 is not locked.
- [Quill OS](https://github.com/Quill-OS/quill), a replacement system for Kobo
  readers and the Kindle Touch.
