#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Split an Android v0 boot image (as shipped in Kindle update packages) into
kernel, ramdisk and second-stage blobs, then decompress the kernel and pull out
its embedded config (IKCONFIG) and version string when present.

Usage: unpack_bootimg.py BOOT_IMG OUT_DIR
"""
import gzip
import json
import lzma
import re
import struct
import sys
import zlib
from pathlib import Path

HEADER = struct.Struct("<8s10I16s512s32s1024s")


def pages(size, page):
    return (size + page - 1) // page * page


def unlzop(stream):
    """Decode an lzop container (Kindle kernels use CONFIG_KERNEL_LZO).
    Needs the python-lzo module."""
    import lzo
    be = lambda n, at: int.from_bytes(stream[at:at + n], "big")
    version = be(2, 9)
    at = 15  # magic(9) + version + lib_version + version_needed
    at += 2 if version >= 0x0940 else 1  # method [+ level]
    flags = be(4, at)
    at += 4
    if flags & 0x800:  # F_H_FILTER
        at += 4
    at += 12 if version >= 0x0940 else 8  # mode + mtime_low [+ mtime_high]
    at += 1 + stream[at] + 4  # name + header checksum
    out = bytearray()
    while True:
        dst_len = be(4, at)
        at += 4
        if dst_len == 0:
            return bytes(out)
        src_len = be(4, at)
        at += 4
        at += 4 if flags & 0x101 else 0  # F_ADLER32_D | F_CRC32_D
        if src_len < dst_len:
            at += 4 if flags & 0x202 else 0  # F_ADLER32_C | F_CRC32_C
            out += lzo.decompress(stream[at:at + src_len], False, dst_len)
        else:
            out += stream[at:at + src_len]
        at += src_len


def decompress_zimage(zimage):
    """Find the first gzip/xz/lzo stream inside a zImage and inflate it."""
    # The decompressor stub also contains the magic as a constant; skip
    # candidates that do not decode.
    start = zimage.find(b"\x89LZO\x00\r\n\x1a\n")
    while start != -1:
        try:
            image = unlzop(zimage[start:])
            if b"Linux version" in image:
                return "lzo", image
        except Exception:
            pass
        start = zimage.find(b"\x89LZO\x00\r\n\x1a\n", start + 1)
    for magic, kind in ((b"\x1f\x8b\x08", "gzip"), (b"\xfd7zXZ\x00", "xz")):
        start = zimage.find(magic)
        while start != -1:
            try:
                if kind == "gzip":
                    return kind, zlib.decompressobj(31).decompress(zimage[start:])
                return kind, lzma.decompress(zimage[start:])
            except (zlib.error, lzma.LZMAError):
                start = zimage.find(magic, start + 1)
    return None, None


def main(boot_img, out_dir):
    data = Path(boot_img).read_bytes()
    out = Path(out_dir)
    out.mkdir(parents=True, exist_ok=True)
    (magic, kernel_size, kernel_addr, ramdisk_size, ramdisk_addr, second_size,
     second_addr, tags_addr, page_size, header_version, os_version, name,
     cmdline, img_id, extra) = HEADER.unpack_from(data)
    if magic != b"ANDROID!":
        sys.exit("not an Android boot image")
    offset = page_size
    blobs = {}
    for label, size in (("kernel", kernel_size), ("ramdisk", ramdisk_size),
                        ("second", second_size)):
        blobs[label] = data[offset:offset + size]
        (out / label).write_bytes(blobs[label])
        offset += pages(size, page_size)

    info = {
        "kernel_size": kernel_size, "kernel_addr": hex(kernel_addr),
        "ramdisk_size": ramdisk_size, "ramdisk_addr": hex(ramdisk_addr),
        "second_size": second_size, "second_addr": hex(second_addr),
        "tags_addr": hex(tags_addr), "page_size": page_size,
        "header_version": header_version,
        "name": name.rstrip(b"\0").decode(errors="replace"),
        "cmdline": (cmdline.rstrip(b"\0") + extra.rstrip(b"\0")).decode(errors="replace"),
        "second_magic": blobs["second"][:4].hex(),
        "ramdisk_magic": blobs["ramdisk"][:4].hex(),
    }

    kind, image = decompress_zimage(blobs["kernel"])
    info["kernel_compression"] = kind
    if image:
        (out / "Image").write_bytes(image)
        version = re.search(rb"Linux version [^\n\0]+", image)
        info["linux_version"] = version.group().decode(errors="replace") if version else None
        start = image.find(b"IKCFG_ST")
        if start != -1:
            end = image.find(b"IKCFG_ED", start)
            (out / "kernel.config").write_bytes(gzip.decompress(image[start + 8:end]))
            info["ikconfig"] = True
        else:
            info["ikconfig"] = False
    (out / "info.json").write_text(json.dumps(info, indent=2) + "\n")
    print(json.dumps(info, indent=2))


if __name__ == "__main__":
    if len(sys.argv) != 3:
        sys.exit(__doc__)
    main(sys.argv[1], sys.argv[2])
