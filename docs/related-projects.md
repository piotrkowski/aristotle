# Related projects

Projects Aristotle builds on, borrows facts from, or keeps an eye on. What the
bundle actually ships, with licences, is in [THIRD_PARTY.md](../THIRD_PARTY.md).

## Built on

- **[Plato](https://github.com/baskerville/plato)** (Rust, AGPL-3.0): the
  document engine behind every book page: its HTML engine with Knuth–Plass
  justification and Liang hyphenation for EPUB, and MuPDF for PDF and the rest.
  Aristotle uses `plato-core` from a fork with a few small commits
  ([piotrkowski/plato](https://github.com/piotrkowski/plato), branch `aristotle`) and
  none of Plato's Kobo interface. See [reader.md](reader.md).
- **[KOReader](https://github.com/koreader/koreader)** (AGPL-3.0): its Kindle
  device table (sysfs paths for the battery, frontlight and hall sensor on the
  PW4), its `refresh_rex` waveform policy in
  [koreader-base](https://github.com/koreader/koreader-base), and its way of
  stopping the stock framework were the starting point for the device code.
  The dev loop runs the `dropbear` SSH server that ships with KOReader.
- **[FBInk](https://github.com/NiLuJe/FBInk)** (GPL-3.0): `eink/mxcfb-kindle.h`
  is the reference for the Kindle EPDC ABI (the Rex update struct, the Zelda
  waveform numbering, the night-mode ioctls). Used as documentation; no FBInk
  code is linked.
- **[eink-emulator](https://github.com/katadelos/eink-emulator)** and its
  [QEMU fork](https://github.com/katadelos/qemu/tree/eink-emulator): a QEMU
  machine for the PW4 (`imx6sll-rex`, with the EPDC, PMIC, touch controllers
  and Lab126's Falcon BIOS) that boots stock firmware. Risky changes are tried
  there first; `devloop/emulator/` sets it up.

## Device knowledge

- **[slint-kindle-backend](https://github.com/sverrejb/slint-kindle-backend)**
  (MIT or Apache-2.0): a Rust Kindle backend tested on the PW4, with the Rex
  update struct, ioctl probing and evdev touch.
- **[yb-reader](https://github.com/albu/yb-reader)** (AGPL-3.0): a Rust reader
  for newer Kindles with a careful takeover of the stock system (Amazon's own
  `DONT_START_FRAMEWORK` flag, an upstart job and a fallback to stock).
- **[kindlemodding.org](https://kindlemodding.org)**: the catalogue of LIPC
  services and properties, and the map of upstart jobs.
- **[openlipc](https://github.com/arkq/openlipc)** and
  **[liblipc-go](https://github.com/clintharrison/liblipc-go)**: LIPC, the
  stock IPC, is D-Bus underneath and can be spoken without `liblipc.so`.
- **Amazon's GPL source release** for firmware 5.13.6
  (`Kindle_src_5.13.6_3731980038.tar.gz`): the 4.1.15-lab126 kernel tree and
  configuration, needed for kernel modules.

## Jailbreak and tools

- **[KindleModding](https://github.com/KindleModding)**: jailbreaks
  (LanguageBreak covers firmware up to 5.16.2.1.1, so 5.13.x), Hotfix, and the
  documentation that installing Aristotle assumes.
- **KUAL** and **MRPI** (NiLuJe's snapshots on MobileRead): the launcher the
  dev-loop extensions run from, and the package installer.
- **[KindleTool](https://github.com/NiLuJe/KindleTool)**: unpacks firmware
  update packages.
- **[koxtoolchain](https://github.com/koreader/koxtoolchain)**: the reference
  toolchains (`kindlepw2` is the PW4's soft-float ABI on firmware below
  5.16.3). Aristotle builds static musl binaries with Zig and Rust instead.

## Later: an own kernel

The PW4 is secure-boot locked (see [pw4-reference.md](pw4-reference.md)), so an
own kernel would come through kexec from the running stock kernel.

- **[kindle-kexec](https://github.com/katadelos/eink-emulator/tree/HEAD/kindle-kexec)**:
  a loadable kexec module with a PW4 profile, verified in the emulator only.
- **[akemnade/linux](https://github.com/akemnade/linux)**: an out-of-tree
  mainline DRM driver for the i.MX6 EPDC, with device trees for Kobo readers on
  the same SoC.

## Other readers and systems

- **[cadmus](https://github.com/OGKevin/cadmus)**: an actively maintained
  Plato fork for Kobo.
- **[kindle-dash](https://github.com/H4rryp0tt3r/kindle-dash)**: a whole
  replacement OS for the PW2 (own init, rootfs and kernel), possible there
  because the PW2 is not locked.
- **[Quill OS](https://github.com/Quill-OS/quill)**: a full replacement system
  for Kobo readers and the Kindle Touch.
