# Glossary

Terms used in Aristotle's docs. Device-specific claims follow the evidence in the [PW4 reference](pw4-reference.md).

## A2

A fast black-and-white [waveform](#waveform) used for press feedback, PW4 mode 6 ([FBInk's headers](https://github.com/NiLuJe/FBInk/blob/master/eink/mxcfb-kindle.h)).

## ABI

An *application binary interface* defines how compiled code calls functions and exchanges data. Aristotle must match the kernel's device-control layouts, even though its static binaries use a different C library from the firmware ([ABI overview](https://en.wikipedia.org/wiki/Application_binary_interface)).

## Atom

An XML feed format used by OPDS 1.2 catalogs to list books and navigation links ([RFC 4287](https://www.rfc-editor.org/rfc/rfc4287)).

## AZW3

A Kindle book container using KF8, the newer format than MOBI 7. Aristotle reads its metadata and stock reading state, but its MOBI-to-EPUB converter handles only MOBI 7 ([calibre's format support](https://manual.calibre-ebook.com/faq.html#what-formats-does-calibre-support-conversion-to-from)).

## Basic auth

HTTP authentication that sends an encoded username and password, rather than encrypting them. Aristotle supports it for catalogs, so HTTPS is needed to protect those credentials in transit ([RFC 7617](https://www.rfc-editor.org/rfc/rfc7617)).

## Bind mount

A second filesystem path to an existing file or directory. The agent uses read-only bind mounts over update programs without replacing the stock files ([Linux mount documentation](https://man7.org/linux/man-pages/man2/mount.2.html)).

## BOM

A *byte-order mark*, a Unicode signature at the start of a text file. A newly created `My Clippings.txt` starts with a UTF-8 BOM ([Unicode FAQ](https://www.unicode.org/faq/utf_bom.html)).

## CBZ

A ZIP archive of comic-page images. Aristotle's library and sync code recognize it, but the default renderer build omits CBZ support ([comic book archives](https://en.wikipedia.org/wiki/Comic_book_archive)).

## Damage

Rectangles whose pixels need repainting or a panel update. Aristotle's UI reports these rectangles to the app, which chooses the hardware update path ([UI model](ui-framework.md#model)).

## Device tree

A hardware description that tells the kernel which components and connections a board has. DTS is its source form, DTB its compiled binary form, and DT an abbreviation for the tree ([Devicetree specification](https://www.devicetree.org/specifications/)).

## DHCP

The protocol that supplies network addresses and settings automatically. Aristotle's USB link uses fixed addresses instead ([RFC 2131](https://www.rfc-editor.org/rfc/rfc2131)).

## Digest auth

HTTP challenge-response authentication that derives a response from credentials and request data. Aristotle supports it for catalogs, but it does not encrypt downloaded content ([RFC 7616](https://www.rfc-editor.org/rfc/rfc7616)).

## Dithering

A way to approximate shades with patterns of available pixel values. Aristotle dithers cover thumbnails to the panel's 16 gray levels ([dithering overview](https://en.wikipedia.org/wiki/Dither)).

## DocFragment

KOReader's internal element for one EPUB spine item, addressed as `DocFragment[N]` in a saved position. Aristotle maps these numbers to Plato's spine items for chapter-level sync ([KOReader's EPUB importer](https://github.com/koreader/crengine/blob/master/crengine/src/epubfmt.cpp)).

## Drive mode

USB mass-storage mode, in which the computer accesses the Kindle's userstore as a drive. The device cannot write `/mnt/us` while it is exported, and USB SSH is unavailable ([USB mass-storage gadget](https://docs.kernel.org/usb/mass-storage.html)).

## DRM

Digital rights management restricts access to protected content. Aristotle opens DRM-free books and includes no DRM-removal code ([DRM overview](https://en.wikipedia.org/wiki/Digital_rights_management)).

## Dropbear

A small SSH server. The agent uses a copy from KOReader, with public-key authentication and a listener restricted to the USB network address ([Dropbear project](https://matt.ucc.asn.au/dropbear/dropbear.html)).

## DU

A waveform for black-and-white UI changes, PW4 mode 1 ([FBInk's headers](https://github.com/NiLuJe/FBInk/blob/master/eink/mxcfb-kindle.h)).

## DU4

A four-gray-level waveform offered as a faster page-turn setting, PW4 mode 7 ([FBInk's headers](https://github.com/NiLuJe/FBInk/blob/master/eink/mxcfb-kindle.h)).

## EPDC

The electrophoretic display controller that drives the e-ink panel. Aristotle submits update rectangles and waveform choices through its kernel driver ([NXP's EPDC driver documentation](https://docs.nxp.com/bundle/RM00293/page/topics/software_operation_005.html)).

## EPUB

A ZIP-based book format containing text, styles, metadata, and a reading order. Aristotle uses Plato to lay out EPUB pages ([EPUB specification](https://www.w3.org/TR/epub-33/)).

## evdev

Linux's input-event interface, exposed through `/dev/input/event*`. Aristotle finds the touchscreen by device name, not a fixed event number ([Linux input documentation](https://docs.kernel.org/input/input.html)).

## EXTH

An extended MOBI header containing metadata such as authors and cover references. Aristotle reads it without loading the book text ([MOBI format reference](https://wiki.mobileread.com/wiki/MOBI)).

## FBInk

An e-ink framebuffer library whose Kindle headers document the update layouts and mode numbers Aristotle uses ([FBInk repository](https://github.com/NiLuJe/FBInk)).

## Framebuffer

Memory containing the screen's pixel values, accessed through `/dev/fb0` on the PW4. Writing pixels alone does not refresh the e-ink panel ([Linux framebuffer documentation](https://docs.kernel.org/fb/framebuffer.html)).

## GC16

A 16-gray-level waveform, PW4 mode 2, whose UI full-refresh hint the app maps to a mode-8 flash ([FBInk's headers](https://github.com/NiLuJe/FBInk/blob/master/eink/mxcfb-kindle.h)).

## GCK16

PW4 mode 8, used for full-screen flashes in day and night mode ([FBInk's headers](https://github.com/NiLuJe/FBInk/blob/master/eink/mxcfb-kindle.h)).

## g_ether

The Linux USB Ethernet gadget module, which makes the Kindle appear as a network adapter to the host. Aristotle switches to it from mass storage for USB SSH ([Linux gadget documentation](https://docs.kernel.org/usb/gadget_configfs.html)).

## Ghosting

Visible traces of an earlier screen after an e-ink update.[^1] Aristotle's page-turn experiments found that full-mode, full-screen mode-9 updates avoided the residue seen with partial updates ([page-turn experiment](benchmarks.md#page-turn-strategy-experiment)).

## GL16

A 16-gray-level waveform for partial gray UI updates, PW4 mode 3 ([FBInk's headers](https://github.com/NiLuJe/FBInk/blob/master/eink/mxcfb-kindle.h)).

## glibc

The GNU C library used by the stock firmware. Aristotle links musl into its binaries instead ([GNU C Library](https://www.gnu.org/software/libc/)).

## GLKW16

PW4 mode 9, used for book pages in day and night mode ([FBInk's headers](https://github.com/NiLuJe/FBInk/blob/master/eink/mxcfb-kindle.h)).

## HAL

The hardware abstraction layer, `aristotle-hal`, provides framebuffer, display, touch, light, battery, and USB access. It supports only the PW4 ([hardware abstraction overview](https://en.wikipedia.org/wiki/Hardware_abstraction)).

## Hard-float

An ARM calling convention that passes floating-point arguments in floating-point registers. Aristotle's static hard-float binaries run on the PW4, but dynamic binaries cannot use the stock softfp libraries ([Arm procedure-call standard](https://github.com/ARM-software/abi-aa/blob/main/aapcs32/aapcs32.rst)).

## HUFF/CDIC

A MOBI text-compression scheme that Aristotle's MOBI 7 converter does not support. Such files fall back to MuPDF ([MOBI compression reference](https://wiki.mobileread.com/wiki/MOBI)).

## IDME

Kindle device-identity and configuration data that can affect boot behavior. Project rules prohibit writes without the owner's explicit approval because no PW4 recovery path is verified ([Kindle IDME notes](https://wiki.mobileread.com/wiki/Kindle_Touch_Hacking)).

## initramfs

An initial in-memory filesystem used during early boot. The firmware's boot image contains it alongside the kernel and device trees ([Linux initramfs documentation](https://docs.kernel.org/filesystems/ramfs-rootfs-initramfs.html)).

## ioctl

A system call for device-specific control requests. Aristotle uses it to read framebuffer geometry and submit EPDC updates ([Linux ioctl documentation](https://man7.org/linux/man-pages/man2/ioctl.2.html)).

## iptables

The Linux firewall tool used by the agent to restrict Wi-Fi traffic and permit USB SSH. Its rules match addresses and ports, not website identities ([iptables documentation](https://www.netfilter.org/projects/iptables/index.html)).

## KF8

Kindle Format 8, an HTML-based successor to MOBI 7, commonly stored as AZW3. Aristotle's converter does not convert KF8 text ([KF8 format reference](https://wiki.mobileread.com/wiki/KF8)).

## KFX

A newer Kindle book format mentioned in the stock-sidecar reference. Aristotle does not import its reading positions or handle Kindle Store DRM ([KFX format reference](https://wiki.mobileread.com/wiki/KFX)).

## Kill switch

The empty `/mnt/us/aristotle-disable` file that prevents the agent and boot-started app from running. It does not disable the separate update-blocking job ([device behavior](../device/README.md#kill-switch)).

## KindleTool

A host tool for inspecting and unpacking Kindle update packages. Aristotle uses it for firmware probes, not for installing its release bundle ([KindleTool repository](https://github.com/NiLuJe/KindleTool)).

## Knuth-Plass

A line-breaking algorithm that chooses breaks across a whole paragraph to balance spacing. Aristotle uses it for justified UI text, and Plato uses it for book layout ([TeX line-breaking paper](https://doi.org/10.1002/spe.4380111102)).

## KOReader

An open-source reader for e-ink devices. Aristotle imports some of its reading state and shares its progress-sync protocol, but uses Plato for book layout ([KOReader project](https://koreader.rocks/)).

## KOSync

KOReader's protocol for exchanging a book's reading position through a server. Aristotle sends EPUB chapter starts or page numbers where supported, with percentage-only sync for other formats ([sync server repository](https://github.com/koreader/koreader-sync-server)).

## KRDS

The typed binary format in Kindle reading-state sidecars. Aristotle reads it to import positions and annotations without changing stock files ([krdsrw format implementation](https://github.com/midrare/krdsrw)).

## KUAL

*Kindle Unified Application Launcher*, the menu that launches Aristotle and its device scripts. It is installed separately ([KUAL repository](https://github.com/coplate/KUAL_Booklet)).

## Lab126

Amazon's device-development organization, whose name appears in Kindle kernel interfaces and service names. Aristotle needs the Kindle-specific display interface, not the corresponding Kobo layout ([Lab126 overview](https://en.wikipedia.org/wiki/Amazon_Lab126)).

## Liang hyphenation patterns

Weighted letter patterns that identify possible word breaks without listing every word. Plato uses language-specific patterns for book text, while Aristotle's UI has separate Polish syllable rules ([Liang's thesis](https://tug.org/docs/liang/)).

## LIPC

Lab126's property and event service, built on D-Bus. Aristotle uses its command-line tools to interact with stock services such as `powerd` ([LIPC reference](https://wiki.mobileread.com/wiki/Lipc)).

## LTO

*Link-time optimization* lets the compiler optimize across compiled units. Aristotle enables it in release builds to reduce binary size ([Cargo profile reference](https://doc.rust-lang.org/cargo/reference/profiles.html#lto)).

## LUT

A *lookup table*, here the waveform data selected for the panel's temperature. The stock EPDC driver chooses it when updating the screen ([EPDC waveform documentation](https://docs.nxp.com/bundle/RM00293/page/topics/software_operation_005.html)).

## mntroot

A stock Kindle command that remounts the root filesystem read-write or read-only. Agent installation and removal use it briefly to add or remove their own jobs ([MobileRead command examples](https://wiki.mobileread.com/wiki/Kindle_Touch_Hacking#Setting_the_time_zone)).

## MOBI

The Mobipocket book-format family. Aristotle converts DRM-free MOBI 7 to cached EPUB, while newer KF8-only files use a different path ([MOBI format reference](https://wiki.mobileread.com/wiki/MOBI)).

## MRPI

*MobileRead Package Installer*, a separately installed tool used to install Kindle homebrew packages such as KUAL. Aristotle's release itself is unpacked onto the USB drive ([MRPI repository](https://wiki.mobileread.com/wiki/MRPI)).

## MuPDF

A document-rendering library used through Plato, including for PDF pages and fallback format handling. Aristotle's build selects which formats and fonts it includes ([MuPDF project](https://mupdf.com/)).

## musl

A C standard library that Aristotle links statically into its binaries. This avoids depending on the firmware's glibc libraries ([musl project](https://musl.libc.org/)).

## My Clippings.txt

The stock Kindle's text export of highlights and notes. Aristotle appends new highlights in the stock format but does not remove old entries when highlights are deleted ([clippings format notes](https://github.com/koreader/koreader/blob/master/plugins/exporter.koplugin/target/my_clippings.lua)).

## OPDS

*Open Publication Distribution System*, a catalog format for finding and downloading books. Aristotle supports OPDS 1.2 feeds, not the newer OPDS 2 format ([OPDS 1.2 specification](https://specs.opds.io/opds-1.2)).

## OpenSearch

A format for describing a server's search URL. Aristotle recognizes it in OPDS catalogs ([OpenSearch specification](https://github.com/dewitt/opensearch/blob/master/opensearch-1-1-draft-6.md)).

## OPF

The EPUB package document that lists metadata, files, and reading order. Aristotle reads it for book metadata and EPUB chapter mapping ([EPUB package specification](https://www.w3.org/TR/epub-33/#sec-package-doc)).

## OTA

Over-the-air firmware updates. The optional agent blocks the stock update programs until the agent is removed ([update-block behavior](../device/README.md#amazons-updates)).

## PalmDOC

A text-compression format supported by Aristotle's MOBI 7 converter. It is distinct from the unsupported HUFF/CDIC scheme ([PalmDOC reference](https://wiki.mobileread.com/wiki/PalmDOC)).

## Partial MD5

KOReader's content identifier, computed from small samples rather than the whole file. Aristotle uses the same algorithm so book identity survives renames and works with KOSync ([KOReader's algorithm](https://koreader.rocks/doc/modules/util.html#partialMD5)).

## Plato

An open-source e-reader originally built for Kobo devices. Aristotle uses its document engine from a fork, not its Kobo interface ([Plato repository](https://github.com/baskerville/plato)).

## powerd

The stock Kindle service for suspend, screensaver, and battery events. Aristotle currently prevents its screensaver by default, with an optional event-handling path that still needs device validation ([KOReader's Kindle power handling](https://github.com/koreader/koreader/blob/master/frontend/device/kindle/powerd.lua)).

## PXP

The i.MX pixel-processing hardware used alongside the EPDC. The PW4 firmware lists its DMA driver in the hardware map ([NXP's EPDC driver documentation](https://docs.nxp.com/bundle/RM00293/page/topics/software_operation_005.html)).

## QEMU

A machine emulator used by `eink-emulator` to test Kindle boot and service changes on the host. It does not replace panel measurements on a physical device ([QEMU project](https://www.qemu.org/)).

## Rasterization

Turning shapes or text outlines into pixels. Aristotle's UI uses Swash to rasterize shaped glyphs ([Swash repository](https://github.com/dfrg/swash)).

## REAGL

A ghost-compensation waveform family, GLR16 (mode 4) or dithered GLD16 (mode 5) on PW4 ([FBInk's headers](https://github.com/NiLuJe/FBInk/blob/master/eink/mxcfb-kindle.h)).

## rootfs

The root filesystem containing the operating system, distinct from the USB-visible userstore. The PW4 normally mounts it read-only ([Linux root filesystem terminology](https://docs.kernel.org/filesystems/ramfs-rootfs-initramfs.html)).

## RSS

Resident set size, the process memory currently held in RAM. The benchmarks distinguish current `VmRSS` from peak `VmHWM` ([Linux process-status reference](https://man7.org/linux/man-pages/man5/proc_pid_status.5.html)).

## SCHED_IDLE

A Linux scheduling policy for very low-priority background work. Aristotle combines it with nice 19, a low CPU priority, for idle workers ([Linux scheduler reference](https://man7.org/linux/man-pages/man7/sched.7.html)).

## Shaping

Converting text into positioned glyphs before drawing it. Aristotle's UI uses Swash for this step ([Swash repository](https://github.com/dfrg/swash)).

## Sidecar

A separate file holding state associated with a book. Aristotle reads Kindle state from `.sdr` folders and KOReader metadata without changing those files ([KOReader sidecar handling](https://github.com/koreader/koreader/blob/master/frontend/docsettings.lua)).

## Snapshot

A saved image of the current and next book pages, with positions and menu labels. Aristotle can display it while the book opens in the background ([snapshot behavior](reader.md#snapshots)).

## softfp

An ARM convention that permits hardware floating-point instructions but passes floating-point arguments through integer registers. The stock firmware uses this convention, so it cannot supply dynamic libraries to Aristotle's hard-float binaries ([Arm procedure-call standard](https://github.com/ARM-software/abi-aa/blob/main/aapcs32/aapcs32.rst)).

## Spine

The ordered list of EPUB content documents. Saved Aristotle positions use a spine item and an offset, rather than a page number that changes with text size ([EPUB spine specification](https://www.w3.org/TR/epub-33/#sec-spine-elem)).

## StarDict

A dictionary format with separate metadata, word-index, and definition files. Aristotle reads user-supplied dictionaries and builds lookup indexes for them ([StarDict format specification](https://github.com/huzheng001/stardict-3/blob/master/dict/doc/StarDictFileFormat)).

## Static binary

A program with its library code linked into the executable, rather than loaded from system libraries. Aristotle ships static ARM binaries to avoid the stock firmware's C-library calling convention.[^2]

## Stride

The byte distance between successive framebuffer rows, which can exceed the visible width. Aristotle reads it from the driver instead of assuming tightly packed pixels ([Linux framebuffer API](https://docs.kernel.org/fb/api.html)).

## Submodule

A Git repository recorded at a specific commit inside another repository. Aristotle keeps its Plato fork in `third_party/plato` this way ([Git submodule documentation](https://git-scm.com/docs/gitsubmodules)).

## sysfs

The kernel's file-based interface under `/sys` for hardware attributes and controls. Aristotle's probes and benchmarks read it for panel information and debug settings ([Linux sysfs documentation](https://docs.kernel.org/filesystems/sysfs.html)).

## Takeover

Stopping the stock GUI and selected background services so Aristotle can use the screen. `takeover.sh` restores the GUI on exit, rather than replacing stock system files ([takeover behavior](pw4-reference.md#taking-over-the-screen)).

## TLS

*Transport Layer Security* encrypts connections and authenticates servers through certificates. Aristotle's release bundles enable it for HTTPS catalogs and sync servers ([RFC 8446](https://www.rfc-editor.org/rfc/rfc8446)).

## tmpfs

A filesystem backed by virtual memory whose contents disappear when unmounted or rebooted. On the measured PW4, `/var` is a 64 MB tmpfs, so `/var/tmp` is not persistent storage ([Linux tmpfs documentation](https://docs.kernel.org/filesystems/tmpfs.html)).

## U-Boot

The bootloader in the stock firmware package. Project rules treat bootloader writes as unsafe until a recovery path is verified ([U-Boot documentation](https://docs.u-boot.org/en/latest/)).

## Upstart

The stock firmware's service manager, which starts and stops jobs in response to events. The agent installs three of its own job files ([Upstart documentation](https://en.wikipedia.org/wiki/Upstart_%28software%29)).

## USB gadget

Linux's device-side USB functionality, which makes the Kindle appear as storage or a network adapter. `g_mass_storage` provides drive mode, while `g_ether` provides the network link ([Linux gadget documentation](https://docs.kernel.org/usb/gadget_configfs.html)).

## Userstore

The USB-visible book storage mounted at `/mnt/us`, also available as `/mnt/base-us` on the measured PW4. It is separate from the root filesystem and `/var/local` ([filesystem measurements](pw4-reference.md#filesystems)).

## Watchdog

A monitor that reacts when a process stops responding or exits. The agent's watchdog can restart the stock GUI, while the stock watchdog makes freezing that GUI unreliable ([takeover measurements](pw4-reference.md#taking-over-the-screen)).

## Waveform

A panel-driving sequence that moves e-ink particles to the requested shades. The stock firmware supplies the waveform data, and Aristotle selects named modes rather than shipping that data ([NXP's waveform documentation](https://docs.nxp.com/bundle/RM00293/page/topics/software_operation_005.html)).

## xpointer

A path-like saved position in KOReader's document tree, not an Aristotle byte offset. Aristotle preserves imported values, but resolves synced EPUB positions only to a `DocFragment` chapter, refined by percentage ([KOReader's position API](https://github.com/koreader/crengine/blob/master/crengine/include/lvtinydom.h)).

## Zig

A programming language and toolchain whose C compiler Aristotle uses for cross-compilation. The build script downloads a pinned version ([Zig project](https://ziglang.org/)).

## zstd

The Zstandard compression format used for saved page snapshots. Compression and writes happen on a worker thread ([Zstandard project](https://facebook.github.io/zstd/)).

[^1]: [Electronic paper limitations](https://en.wikipedia.org/wiki/Electronic_paper).
[^2]: [Rust linkage reference](https://doc.rust-lang.org/reference/linkage.html).
