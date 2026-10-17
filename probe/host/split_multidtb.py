#!/usr/bin/env python3
"""Lab126 boot images carry a wrapper DTB whose /fdts/fdt@N nodes each embed
one board DTB (platform_name + data). Extract each one as .dtb and .dts.

Usage: split_multidtb.py WRAPPER_DTB OUT_DIR   (needs the `fdt` module)
"""
import sys
from pathlib import Path

import fdt


def main(wrapper, out_dir):
    out = Path(out_dir)
    out.mkdir(parents=True, exist_ok=True)
    tree = fdt.parse_dtb(Path(wrapper).read_bytes())
    for node in tree.get_node("/fdts").nodes:
        name = node.get_property("platform_name").value.removesuffix(".dtb")
        data = node.get_property("data")
        if isinstance(data, fdt.PropWords):  # parser guessed 32-bit cells
            blob = b"".join(word.to_bytes(4, "big") for word in data.data)
        else:
            blob = bytes(data.data)
        (out / f"{name}.dtb").write_bytes(blob)
        (out / f"{name}.dts").write_text(fdt.parse_dtb(blob).to_dts())
        print(f"{node.name}: {name} ({len(blob)} bytes)")


if __name__ == "__main__":
    if len(sys.argv) != 3:
        sys.exit(__doc__)
    main(sys.argv[1], sys.argv[2])
