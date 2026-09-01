# Disk image window

What a reader sees when the macOS image mounts: two 128px icons, an arrow
between them, and the sentence naming the gesture. `packaging/build-DS_Store.py`
places the icons and the window; this directory holds what is drawn behind them.

| File | Use |
|---|---|
| `background.png`, `background@2x.png` | the window background, 600×400 and its Retina twin. `packaging/build-dmg.sh` stacks the pair into one `.tiff` with `tiffutil -cathidpicheck` and stages it at `.background/background.tiff` inside the image. Not committed as a TIFF: `tiffutil` exists only on macOS, and building it on the runner keeps a Mac out of the loop for anyone redrawing this. |
| `make-background.py` | draws both. Run `python3 assets/dmg/make-background.py` from the repository root; it needs Pillow, which is a developer's tool here and not a build dependency. |

The drawing is transparent everywhere except the arrow and the two lines of
text. Finder composites it over the window's own background, which follows the
reader's appearance, so one file reads on the white panel of a light Mac and
the dark one of a night Mac. Ink is a single mid grey clearing 3:1 against
both. A painted panel would be a white rectangle in dark mode.

There is no SVG master, unlike the logo. The drawing is an arrow and two
strings, and a master would only be `make-background.py` with a rasteriser
bolted on.

The geometry lives in two files. `WINDOW_W`, `WINDOW_H`, `ICON_GAP`, and
`ICON_Y` appear in both `make-background.py` and `packaging/build-DS_Store.py`,
because the arrow has to point between icons Finder places from the second.
Change one and change the other, or the arrow lands under an icon.
