#!/usr/bin/env python3
"""Draw the disk image's window background: an arrow and two lines of text.

The image is transparent everywhere else, and that is the point. Finder draws
it over the window's own background, which follows the reader's appearance, so
one file reads on both the white panel of a light Mac and the dark one of a
night Mac. A painted panel would be a white rectangle in dark mode, which is
the reason `build-DS_Store.py` refused a background colour before it had a
picture to draw.

The ink is a single mid grey, chosen so it clears 3:1 against both of those
backgrounds; nothing here is drawn small enough to need more. The geometry is
duplicated in `build-DS_Store.py`, which places the icons this arrow points
between, and the two files have to move together. There is no SVG master: the
drawing is an arrow and two strings, and a master would only be this script
with a rasteriser bolted on.

Run from the repository root:

    python3 assets/dmg/make-background.py

Pillow is a developer's tool here, not a build dependency. macOS has no copy of
it, so the rasters are committed and `packaging/build-dmg.sh` only stacks them
into the TIFF that carries both resolutions.
"""

import pathlib
import sys

from PIL import Image, ImageDraw, ImageFont

# The window, in the coordinates `build-DS_Store.py` writes into `bwsp`. Finder
# gives the background image the window's content area at its natural size, so
# this is also the 1x pixel size of the file.
WIDTH, HEIGHT = 600, 400

# Icon centres, repeated from `build-DS_Store.py`. Only the arrow needs them.
ICON_Y = 168
ICON_GAP = 270
ICON_HALF = 64

INK = (142, 142, 147)

FONT = "fonts/IBMPlexSans-Variable.ttf"
LEAD = "Drag hww onto Applications"
LEAD_SIZE, LEAD_WEIGHT, LEAD_Y = 19, 500, 296
TAIL = "then eject this image"
TAIL_SIZE, TAIL_WEIGHT, TAIL_Y = 14, 400, 328
TAIL_ALPHA = 190

# Supersampling for the arrow alone. Text comes off FreeType already
# anti-aliased and is drawn after the shapes are scaled down, so it is hinted
# at the size it ships at rather than blurred by a resize.
OVER = 4


def arrow(scale):
    """The arrow, drawn large and scaled down, as an alpha mask."""
    size = (WIDTH * scale * OVER, HEIGHT * scale * OVER)
    mask = Image.new("L", size, 0)
    draw = ImageDraw.Draw(mask)

    unit = scale * OVER
    left = (WIDTH / 2 - ICON_GAP / 2 + ICON_HALF + 17) * unit
    right = (WIDTH / 2 + ICON_GAP / 2 - ICON_HALF - 17) * unit
    middle = ICON_Y * unit
    head = 24 * unit
    shaft = 3.5 * unit
    barb = 13 * unit

    draw.rectangle([left, middle - shaft, right - head, middle + shaft], fill=255)
    draw.polygon(
        [
            (right - head, middle - barb),
            (right, middle),
            (right - head, middle + barb),
        ],
        fill=255,
    )
    return mask.resize((WIDTH * scale, HEIGHT * scale), Image.LANCZOS)


def draw_background(scale):
    canvas = Image.new("RGBA", (WIDTH * scale, HEIGHT * scale), (0, 0, 0, 0))
    canvas.paste(Image.new("RGBA", canvas.size, INK + (255,)), (0, 0), arrow(scale))

    draw = ImageDraw.Draw(canvas)
    for text, size, weight, centre, alpha in (
        (LEAD, LEAD_SIZE, LEAD_WEIGHT, LEAD_Y, 255),
        (TAIL, TAIL_SIZE, TAIL_WEIGHT, TAIL_Y, TAIL_ALPHA),
    ):
        font = ImageFont.truetype(FONT, size * scale)
        font.set_variation_by_axes([weight, 100])
        draw.text(
            (WIDTH * scale / 2, centre * scale),
            text,
            font=font,
            fill=INK + (alpha,),
            anchor="mm",
        )
    return canvas


def main(argv):
    out = pathlib.Path(argv[1] if len(argv) > 1 else "assets/dmg")
    if not pathlib.Path(FONT).is_file():
        print(f"{FONT} is missing; run this from the repository root", file=sys.stderr)
        return 2
    out.mkdir(parents=True, exist_ok=True)
    for scale, name in ((1, "background.png"), (2, "background@2x.png")):
        draw_background(scale).save(out / name)
        print(out / name)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
