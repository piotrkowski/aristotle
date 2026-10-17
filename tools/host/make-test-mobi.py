#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Make a small MOBI 7 test book from an EPUB, without calibre or kindlegen.

Usage: make-test-mobi.py BOOK.epub OUT.mobi [LANG]

The body HTML of the EPUB's spine items is joined with <mbp:pagebreak/>
the way kindlegen does it, compressed with PalmDOC (literal codes only,
which decoders must accept), split into 4096-byte text records with
multibyte and TBS trailing entries, and given an EXTH header with title,
author and language (EXTH 524). The first image of the EPUB becomes image
record 1 and is referenced as <img recindex="00001"/>. Good enough to test
aristotle-render's MOBI conversion; not a general converter.
"""
import re
import struct
import sys
import zipfile

RECORD = 4096


def palmdoc_literal(data: bytes) -> bytes:
    """PalmDOC stream using only literals: 0x00 and 0x09..0x7f stand for
    themselves, 1..8 announce that many raw bytes."""
    out = bytearray()
    i = 0
    while i < len(data):
        c = data[i]
        if c == 0 or 0x09 <= c <= 0x7F:
            out.append(c)
            i += 1
        else:
            run = data[i : i + 8]
            out.append(len(run))
            out += run
            i += len(run)
    return bytes(out)


def body_of(xhtml: str) -> str:
    m = re.search(r"<body[^>]*>(.*)</body>", xhtml, re.S)
    return m.group(1) if m else ""


def main(epub, out, lang="pl"):
    z = zipfile.ZipFile(epub)
    container = z.read("META-INF/container.xml").decode()
    opf_path = re.search(r'full-path="([^"]+)"', container).group(1)
    base = opf_path.rsplit("/", 1)[0] + "/" if "/" in opf_path else ""
    opf = z.read(opf_path).decode()
    title = re.search(r"<dc:title[^>]*>([^<]*)<", opf).group(1)
    author_m = re.search(r"<dc:creator[^>]*>([^<]*)<", opf)
    author = author_m.group(1) if author_m else "Unknown"
    items = dict(re.findall(r'<item [^>]*?id="([^"]+)"[^>]*?href="([^"]+)"', opf))
    items.update({i: h for h, i in re.findall(r'<item [^>]*?href="([^"]+)"[^>]*?id="([^"]+)"', opf)})
    spine = re.findall(r'<itemref [^>]*idref="([^"]+)"', opf)

    image = None
    parts = []
    for idref in spine:
        href = items.get(idref)
        if not href or not href.endswith(("html", "htm")):
            continue
        body = body_of(z.read(base + href).decode("utf-8", "replace"))
        # Replace images with the first image of the book, as a recindex.
        def img(m):
            nonlocal image
            if image is None:
                src = m.group(1)
                path = (base + href).rsplit("/", 1)[0] + "/" + src
                path = re.sub(r"[^/]+/\.\./", "", path)
                try:
                    image = z.read(path)
                except KeyError:
                    return ""
            return '<img recindex="00001"/>'
        body = re.sub(r'<img[^>]*src="([^"]+)"[^>]*/?>', img, body)
        parts.append(body)
    html = ("<html><head><guide></guide></head><body>"
            + "<mbp:pagebreak/>".join(parts) + "</body></html>").encode()

    text_records = []
    for i in range(0, len(html), RECORD):
        chunk = html[i : i + RECORD]
        rec = palmdoc_literal(chunk)
        # Trailing entries for flags 0b11, read by decoders from the end:
        # last a TBS entry of 3 bytes whose size is coded backwards (0x83),
        # before it the multibyte entry: one byte whose low two bits are
        # the number of extra bytes (0: no overlap).
        rec += b"\x00" + b"\xAA\xBB\x83"
        text_records.append(rec)

    exth_recs = [(100, author.encode()), (503, title.encode()), (524, lang.encode())]
    exth_body = b"".join(struct.pack(">II", t, len(d) + 8) + d for t, d in exth_recs)
    exth = b"EXTH" + struct.pack(">II", 12 + len(exth_body), len(exth_recs)) + exth_body
    exth += b"\0" * (-len(exth) % 4)

    n_text = len(text_records)
    first_image = n_text + 1 if image else 0xFFFFFFFF
    first_non_book = n_text + 1
    name = title.encode()
    mobi = bytearray(232)
    mobi[0:4] = b"MOBI"
    struct.pack_into(">IIII", mobi, 4, 232, 2, 65001, 0x12345678)
    struct.pack_into(">I", mobi, 0x14, 6)  # file version
    for off in range(0x18, 0x40, 4):
        struct.pack_into(">I", mobi, off, 0xFFFFFFFF)
    struct.pack_into(">I", mobi, 0x40, first_non_book)
    struct.pack_into(">II", mobi, 0x44, 0x10 + 232 + len(exth), len(name))
    struct.pack_into(">III", mobi, 0x4C, 0x15, 0, 0)  # locale: Polish
    struct.pack_into(">II", mobi, 0x58, 6, first_image)
    struct.pack_into(">I", mobi, 0x70, 0x40)  # EXTH present
    struct.pack_into(">II", mobi, 0x94, 0xFFFFFFFF, 0xFFFFFFFF)
    struct.pack_into(">HH", mobi, 0xB0, 1, n_text)
    struct.pack_into(">H", mobi, 0xE2, 0b11)  # extra record data flags
    struct.pack_into(">I", mobi, 0xE4, 0xFFFFFFFF)
    rec0 = struct.pack(">HHIHHHH", 2, 0, len(html), n_text, RECORD, 0, 0) + bytes(mobi) + exth + name
    rec0 += b"\0" * (-len(rec0) % 4 + 8)

    records = [rec0] + text_records + ([image] if image else []) + [b"\xe9\x8e\x0d\x0a"]
    header = bytearray(78)
    header[0:32] = (title.encode()[:31]).ljust(32, b"\0")
    header[60:68] = b"BOOKMOBI"
    struct.pack_into(">H", header, 76, len(records))
    offset = 78 + 8 * len(records) + 2
    table = bytearray()
    for i, r in enumerate(records):
        table += struct.pack(">II", offset, i * 2)
        offset += len(r)
    with open(out, "wb") as f:
        f.write(bytes(header) + bytes(table) + b"\0\0" + b"".join(records))
    print(f"{out}: {n_text} text records, {len(html)} bytes of HTML, image: {bool(image)}")


if __name__ == "__main__":
    if len(sys.argv) not in (3, 4):
        sys.exit(__doc__)
    main(*sys.argv[1:])
