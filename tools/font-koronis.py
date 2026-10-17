#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Map U+1FBD GREEK KORONIS to the font's U+2019 glyph, in place.

usage: font-koronis.py FONT...   (needs fontTools on PYTHONPATH)

Greek texts often write the elision mark with U+1FBD (πάντ᾽ ἐφίεται,
Bywater's Aristotle among them); Unicode recommends U+2019 for it. Literata
draws U+1FBD as a spacing breathing half an em wide (509 units against 328
for U+2019), so every elision showed a gap before the next word. The koronis
proper sits over a vowel in crasis (κἀγώ) and is encoded with the vowel, so
a spacing U+1FBD is in practice always an elision mark. Fonts without either
character are left alone. The file keeps its modification time, which the
reader's snapshot key reads.
"""

import os
import sys

from fontTools.ttLib import TTFont

for path in sys.argv[1:]:
    st = os.stat(path)
    font = TTFont(path)
    changed = False
    for table in font["cmap"].tables:
        if table.isUnicode() and 0x2019 in table.cmap and 0x1FBD in table.cmap:
            table.cmap[0x1FBD] = table.cmap[0x2019]
            changed = True
    if changed:
        font.save(path)
        os.utime(path, ns=(st.st_atime_ns, st.st_mtime_ns))
