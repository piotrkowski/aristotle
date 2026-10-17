# Glossary

Kindle and e-ink terms used in Aristotle's docs. Device-specific claims follow the evidence in the [PW4 reference](pw4-reference.md).

## A2

A fast black-and-white [waveform](#waveform) used for press feedback, PW4 mode 6 ([FBInk's headers](https://github.com/NiLuJe/FBInk/blob/master/eink/mxcfb-kindle.h)).

## AZW3

A Kindle book container using KF8, the newer format than MOBI 7. Aristotle reads its metadata and stock reading state, but its MOBI-to-EPUB converter handles only MOBI 7 ([calibre's format support](https://manual.calibre-ebook.com/faq.html#what-formats-does-calibre-support-conversion-to-from)).

## Drive mode

USB mass-storage mode, in which the computer accesses the Kindle's userstore as a drive. The device cannot write `/mnt/us` while it is exported, and USB SSH is unavailable ([USB mass-storage gadget](https://docs.kernel.org/usb/mass-storage.html)).

## Dropbear

A small SSH server. The agent uses a copy from KOReader, with public-key authentication and a listener restricted to the USB network address ([Dropbear project](https://matt.ucc.asn.au/dropbear/dropbear.html)).

## DU

A waveform for black-and-white UI changes, PW4 mode 1 ([FBInk's headers](https://github.com/NiLuJe/FBInk/blob/master/eink/mxcfb-kindle.h)).

## DU4

A four-gray-level waveform offered as a faster page-turn setting, PW4 mode 7 ([FBInk's headers](https://github.com/NiLuJe/FBInk/blob/master/eink/mxcfb-kindle.h)).

## EPDC

The electrophoretic display controller that drives the e-ink panel. Aristotle submits update rectangles and waveform choices through its kernel driver ([NXP's EPDC driver documentation](https://docs.nxp.com/bundle/RM00293/page/topics/software_operation_005.html)).

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

## GLKW16

PW4 mode 9, used for book pages in day and night mode ([FBInk's headers](https://github.com/NiLuJe/FBInk/blob/master/eink/mxcfb-kindle.h)).

## HAL

The hardware abstraction layer, `aristotle-hal`, provides framebuffer, display, touch, light, battery, and USB access. It supports only the PW4 ([hardware abstraction overview](https://en.wikipedia.org/wiki/Hardware_abstraction)).

## HUFF/CDIC

A MOBI text-compression scheme that Aristotle's MOBI 7 converter does not support. Such files fall back to MuPDF ([MOBI compression reference](https://wiki.mobileread.com/wiki/MOBI)).

## IDME

Kindle device-identity and configuration data that can affect boot behavior. Project rules prohibit writes without the owner's explicit approval because no PW4 recovery path is verified ([Kindle IDME notes](https://wiki.mobileread.com/wiki/Kindle_Touch_Hacking)).

## KF8

Kindle Format 8, an HTML-based successor to MOBI 7, commonly stored as AZW3. Aristotle's converter does not convert KF8 text ([KF8 format reference](https://wiki.mobileread.com/wiki/KF8)).

## KFX

A newer Kindle book format mentioned in the stock-sidecar reference. Aristotle does not import its reading positions or handle Kindle Store DRM ([KFX format reference](https://wiki.mobileread.com/wiki/KFX)).

## Kill switch

The empty `/mnt/us/aristotle-disable` file that prevents the agent and boot-started app from running. It does not disable the separate update-blocking job ([device behavior](../device/README.md#kill-switch)).

## KindleTool

A host tool for inspecting and unpacking Kindle update packages. Aristotle uses it for firmware probes, not for installing its release bundle ([KindleTool repository](https://github.com/NiLuJe/KindleTool)).

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

## LIPC

Lab126's property and event service, built on D-Bus. Aristotle uses its command-line tools to interact with stock services such as `powerd` ([LIPC reference](https://wiki.mobileread.com/wiki/Lipc)).

## mntroot

A stock Kindle command that remounts the root filesystem read-write or read-only. Agent installation and removal use it briefly to add or remove their own jobs ([MobileRead command examples](https://wiki.mobileread.com/wiki/Kindle_Touch_Hacking#Setting_the_time_zone)).

## MOBI

The Mobipocket book-format family. Aristotle converts DRM-free MOBI 7 to cached EPUB, while newer KF8-only files use a different path ([MOBI format reference](https://wiki.mobileread.com/wiki/MOBI)).

## MRPI

*MobileRead Package Installer*, a separately installed tool used to install Kindle homebrew packages such as KUAL. Aristotle's release itself is unpacked onto the USB drive ([MRPI repository](https://wiki.mobileread.com/wiki/MRPI)).

## My Clippings.txt

The stock Kindle's text export of highlights and notes. Aristotle appends new highlights in the stock format but does not remove old entries when highlights are deleted ([clippings format notes](https://github.com/koreader/koreader/blob/master/plugins/exporter.koplugin/target/my_clippings.lua)).

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

## REAGL

A ghost-compensation waveform family, GLR16 (mode 4) or dithered GLD16 (mode 5) on PW4 ([FBInk's headers](https://github.com/NiLuJe/FBInk/blob/master/eink/mxcfb-kindle.h)).

## rootfs

The root filesystem containing the operating system, distinct from the USB-visible userstore. The PW4 normally mounts it read-only ([Linux root filesystem terminology](https://docs.kernel.org/filesystems/ramfs-rootfs-initramfs.html)).

## Sidecar

A separate file holding state associated with a book. Aristotle reads Kindle state from `.sdr` folders and KOReader metadata without changing those files ([KOReader sidecar handling](https://github.com/koreader/koreader/blob/master/frontend/docsettings.lua)).

## softfp

An ARM convention that permits hardware floating-point instructions but passes floating-point arguments through integer registers. The stock firmware uses this convention, so it cannot supply dynamic libraries to Aristotle's hard-float binaries ([Arm procedure-call standard](https://github.com/ARM-software/abi-aa/blob/main/aapcs32/aapcs32.rst)).

## Takeover

Stopping the stock GUI and selected background services so Aristotle can use the screen. `takeover.sh` restores the GUI on exit, rather than replacing stock system files ([takeover behavior](pw4-reference.md#taking-over-the-screen)).

## U-Boot

The bootloader in the stock firmware package. Project rules treat bootloader writes as unsafe until a recovery path is verified ([U-Boot documentation](https://docs.u-boot.org/en/latest/)).

## Upstart

The stock firmware's service manager, which starts and stops jobs in response to events. The agent installs three of its own job files ([Upstart documentation](https://en.wikipedia.org/wiki/Upstart_%28software%29)).

## USB gadget

Linux's device-side USB functionality, which makes the Kindle appear as storage or a network adapter. `g_mass_storage` provides drive mode, while `g_ether` provides the network link ([Linux gadget documentation](https://docs.kernel.org/usb/gadget_configfs.html)).

## Userstore

The USB-visible book storage mounted at `/mnt/us`, also available as `/mnt/base-us` on the measured PW4. It is separate from the root filesystem and `/var/local` ([filesystem measurements](pw4-reference.md#filesystems)).

## Waveform

A panel-driving sequence that moves e-ink particles to the requested shades. The stock firmware supplies the waveform data, and Aristotle selects named modes rather than shipping that data ([NXP's waveform documentation](https://docs.nxp.com/bundle/RM00293/page/topics/software_operation_005.html)).

