# Screenshots

`compose.py` frames raw panel captures for the README and docs. Inputs are
1072 x 1448 8-bit gray PNGs: the desktop preview, or the device's framebuffer
via `devloop/host/screenshot.sh`. Outputs go to `assets/screenshots/` as
lossless gray-and-alpha PNGs with a transparent background, so they sit on
both GitHub's light and dark pages.

## Run

```sh
tools/screenshots/compose.sh
```

The wrapper keeps uv's cache, the virtualenv and `TMPDIR` under
`~/.cache/aristotle/`, and runs against the pinned `uv.lock`.

## Inputs

`manifest.toml` lists every output and its captures. Relative input paths
resolve against `base` (default `~/.cache/aristotle/ui-preview`); paths may use
`~` and `$VARS`. To use device captures, put them in one directory with the
same relative names and point `SCREENSHOTS_BASE` at it:

```sh
SCREENSHOTS_BASE=~/.cache/aristotle/screens/readme tools/screenshots/compose.sh
```

or edit an `inputs` entry to an absolute path. Each capture is checked for size
and mode, and the run stops with an error naming the bad file.

## Frame

Each screen is a rounded card with a 1 px hairline just outside its edge and a
soft shadow. The `[style.*]` tables set the corner radius, an optional thin
dark bezel (`bezel_px`, used by the `bezel` styles), the hairline colour and
alpha, the shadow layers and the padding. `default` is for day screens on
GitHub's light theme; `night` has a light hairline and a stronger shadow for
night-mode screens on the dark theme. `screen_width` sets the screen size in
pixels: the README uses 536, half the panel, so the images are 2x. The panel
is only ever scaled down, once, with Lanczos.
