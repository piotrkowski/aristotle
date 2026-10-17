# SPDX-License-Identifier: AGPL-3.0-or-later
"""Frame raw Aristotle panel captures for the README and docs.

Usage: python compose.py manifest.toml

Each input is a raw 1072x1448 8-bit gray capture of the Kindle Paperwhite 4
panel. Every output in the manifest places one or more captures side by side,
each as a rounded card (the bare page, or the page in a thin dark bezel) with
a 1 px hairline and a soft drop shadow, on a transparent background. The
panel is resampled once at most, downwards, with Lanczos, and the PNG is
written losslessly.

An output with `frame = false` is one capture as it is, snapped to the
panel's 16 gray levels and stored as a 4-bit PNG: the frames of the site's
demo, which draws its own device.
"""

import os
import sys
import tomllib
from pathlib import Path

from PIL import Image, ImageChops, ImageDraw, ImageFilter

PANEL = (1072, 1448)
SUPERSAMPLE = 4  # for antialiased corners


def fail(msg):
    sys.exit(f"compose: {msg}")


def expand(path, base):
    """Expand ~ and $VARS; relative paths resolve against base."""
    p = Path(os.path.expandvars(os.path.expanduser(str(path))))
    return p if p.is_absolute() else base / p


def load_capture(path):
    if not path.is_file():
        fail(f"input not found: {path}")
    img = Image.open(path)
    if img.size != PANEL or img.mode != "L":
        fail(f"{path}: expected a {PANEL[0]}x{PANEL[1]} 8-bit gray PNG, "
             f"got {img.size[0]}x{img.size[1]} mode {img.mode}")
    return img


def rgb(hexcolor, alpha=255):
    h = hexcolor.lstrip("#")
    return tuple(int(h[i:i + 2], 16) for i in (0, 2, 4)) + (alpha,)


def rgba(spec):
    """"#rrggbb" or "#rrggbb/alpha" (alpha 0-255)."""
    color, _, alpha = spec.partition("/")
    return rgb(color, int(alpha) if alpha else 255)


def rounded_mask(size, radius):
    """Antialiased rounded-rectangle alpha mask."""
    w, h = size
    big = Image.new("L", (w * SUPERSAMPLE, h * SUPERSAMPLE), 0)
    ImageDraw.Draw(big).rounded_rectangle(
        (0, 0, big.width - 1, big.height - 1), radius * SUPERSAMPLE, fill=255)
    return big.reduce(SUPERSAMPLE)


def device(capture, screen_w, style):
    """One framed screen as RGBA: the page, an optional thin bezel around it,
    rounded corners and a hairline ring just outside the edge.

    Returns the image and the alpha of the solid part (for the shadow)."""
    if screen_w > PANEL[0]:
        fail(f"screen_width {screen_w} would upscale the {PANEL[0]} px panel")
    screen_h = round(screen_w * PANEL[1] / PANEL[0])
    screen = capture
    if screen_w != PANEL[0]:
        # The only resampling step: one Lanczos downscale.
        screen = capture.resize((screen_w, screen_h), Image.Resampling.LANCZOS)

    bezel = style.get("bezel_px", 0)
    ring = 1 if style.get("border") else 0
    inner = (screen_w + 2 * bezel, screen_h + 2 * bezel)
    radius = style["radius"]
    body = Image.new("RGBA", inner, rgb(style.get("bezel_color", "#000000")))
    if bezel:
        # The screen's own corners are rounded a little less than the body's.
        screen_mask = rounded_mask(screen.size, max(radius - bezel, 2))
        body.paste(screen.convert("RGB"), (bezel, bezel), screen_mask)
    else:
        body.paste(screen.convert("RGB"), (0, 0))
    body.putalpha(rounded_mask(inner, radius))

    size = (inner[0] + 2 * ring, inner[1] + 2 * ring)
    out = Image.new("RGBA", size, (0, 0, 0, 0))
    solid = Image.new("L", size, 0)
    solid.paste(body.getchannel("A"), (ring, ring))
    if ring:
        outer = rounded_mask(size, radius + ring)
        ring_alpha = ImageChops.subtract(outer, solid)
        r, g, b, a = rgba(style["border"])
        hair = Image.new("RGBA", size, (r, g, b, 0))
        hair.putalpha(ring_alpha.point(lambda v: v * a // 255))
        out.alpha_composite(hair)
    out.alpha_composite(body, (ring, ring))
    return out, solid


def shadow(alpha, shadows):
    """Layered soft shadows from an alpha mask; black, so no halo on dark pages."""
    out = Image.new("L", alpha.size, 0)
    for dy, blur, opacity in shadows:
        layer = ImageChops.offset(alpha, 0, dy).filter(ImageFilter.GaussianBlur(blur))
        layer = layer.point(lambda v, o=opacity: round(v * o))
        out = ImageChops.add(out, layer)
    black = Image.new("RGBA", alpha.size, (0, 0, 0, 0))
    black.putalpha(out)
    return black


def compose(captures, spec, style):
    framed = [device(c, spec["screen_width"], style) for c in captures]
    dw, dh = framed[0][0].size
    pad = style["padding"]
    gap = style.get("gap", pad)
    n = len(framed)
    canvas = Image.new("RGBA", (2 * pad + n * dw + (n - 1) * gap, 2 * pad + dh), (0, 0, 0, 0))
    mask = Image.new("L", canvas.size, 0)
    for i, (_, solid) in enumerate(framed):
        mask.paste(solid, (pad + i * (dw + gap), pad))
    canvas.alpha_composite(shadow(mask, style["shadows"]))
    for i, (img, _) in enumerate(framed):
        canvas.alpha_composite(img, (pad + i * (dw + gap), pad))
    return canvas


def is_neutral(img):
    r, g, b, _ = img.split()
    return not ImageChops.difference(r, g).getbbox() and not ImageChops.difference(g, b).getbbox()


def save_png(img, path):
    """Write a lossless PNG: gray plus alpha when every pixel is a neutral gray
    (half the bytes of RGBA, nothing rounded), else RGBA."""
    path.parent.mkdir(parents=True, exist_ok=True)
    mode = "LA" if is_neutral(img) else "RGBA"
    img.convert(mode).save(path, optimize=True)
    return mode


def save_panel16(capture, path):
    """Write one capture unframed, snapped to the 16 levels the panel shows
    (0x00, 0x11, ... 0xff), as a 4-bit palette PNG."""
    path.parent.mkdir(parents=True, exist_ok=True)
    snapped = capture.point(lambda v: round(v / 17) * 17)
    pal = Image.new("P", (1, 1))
    pal.putpalette([v for i in range(16) for v in (i * 17,) * 3])
    img = snapped.convert("RGB").quantize(palette=pal, dither=Image.Dither.NONE)
    if img.convert("L").tobytes() != snapped.tobytes():
        fail(f"{path.name}: palette conversion changed pixels")
    img.save(path, optimize=True, bits=4)


def main(argv):
    if len(argv) != 2:
        fail("usage: compose.py manifest.toml")
    manifest_path = Path(argv[1]).resolve()
    with manifest_path.open("rb") as f:
        m = tomllib.load(f)
    here = manifest_path.parent
    base = expand(os.environ.get("SCREENSHOTS_BASE") or m["base"], here)
    out_dir = expand(m["out_dir"], here)
    styles = m["style"]

    for spec in m["output"]:
        captures = [load_capture(expand(p, base)) for p in spec["inputs"]]
        out = expand(spec.get("out_dir", out_dir), here) / spec["name"]
        if not spec.get("frame", True):
            if len(captures) != 1:
                fail(f"{spec['name']}: an unframed output takes one input")
            save_panel16(captures[0], out)
            print(f"{out.name}: {PANEL[0]}x{PANEL[1]}, {out.stat().st_size // 1024} KB (16 grays)")
            continue
        style = {**styles["default"], **styles.get(spec.get("style", "default"), {})}
        img = compose(captures, spec, style)
        mode = save_png(img, out)
        print(f"{out.name}: {img.width}x{img.height}, {out.stat().st_size // 1024} KB ({mode})")


if __name__ == "__main__":
    main(sys.argv)
