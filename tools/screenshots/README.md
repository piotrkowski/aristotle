# Screenshots

`compose.py` turns raw panel captures into the images of the README, the docs
and the site. Inputs are 1072 x 1448 8-bit gray PNGs of the device's
framebuffer (`tools/host/screenshot.sh`) or of the desktop preview.

- Framed shots go to `assets/screenshots/` (README and docs) and
  `site/assets/shots/` (the site's figures), as lossless gray-and-alpha PNGs
  with a transparent background, so they sit on light and dark pages.
- The site's demo frames go to `site/demo/frames/` unframed, snapped to the
  panel's 16 gray levels and stored as 4-bit PNGs (`frame = false`).

## Run

```sh
tools/screenshots/compose.sh
```

The wrapper keeps uv's cache, the virtualenv and `TMPDIR` under the cache
directory, `$ARISTOTLE_CACHE` (`tools/env.sh`), and runs against the pinned
`uv.lock`.

## Inputs

`manifest.toml` lists every output and its captures. The captures live in
`$ARISTOTLE_CACHE/captures/readme/`, named as the manifest names them
(`day-library.png`, `night-greek-popup.png`, ...): the English interface, in
day and night mode, with public-domain books only. To take them, run
`tools/host/screenshot.sh NAME` for each screen and copy the PNGs there
under those names.

Relative input paths resolve against `base`; `SCREENSHOTS_BASE` overrides it,
and paths may use `~` and `$VARS`. Each capture is checked for size and mode,
and the run stops with an error naming the bad file.

## Frame

Each screen is a rounded card with a 1 px hairline just outside its edge and a
soft shadow. The `[style.*]` tables set the corner radius, an optional thin
dark bezel (`bezel_px`, used by the `bezel` styles), the hairline colour and
alpha, the shadow layers and the padding. `default` is for day screens on
light pages; `night` has a light hairline and a stronger shadow for night-mode
screens on dark pages; `site` and `site-night` have a narrower margin.
`screen_width` sets the screen size in pixels: 536, half the panel, so the
images are 2x. The panel is only ever scaled down, once, with Lanczos.
