# hww logo

A waveform whose amplitude decays into a flat line: the loud web arriving, the
quiet one handed back. Two strokes, one ground, nothing else.

    #000000   ground
    #D4D1CB   the wave        (theme.rs, Theme::Dark fg)
    #E09B54   the flat tail   (theme.rs, Theme::Dark notice_fg)

| File | Use |
|---|---|
| `hww.svg` | master, 64 grid. Anything 48 px and up. |
| `hww-32.svg`, `hww-16.svg` | redrawn on their own grids, fewer vertices, strokes on whole pixels. Do not scale the master to these sizes. The 16 carries two peaks and a longer flat, because at that size the tail is what disappears first. |
| `hww-*.png` | rasterised with `rsvg-convert`, 16 through 512. Regenerate with `rsvg-convert -w N -h N hww.svg -o hww-N.png`; take 16 and 32 from their own SVGs. |
| `hww-mark.svg` | strokes only, no ground, for placing on a dark surface that is not black. |
| `hww.ico` | the Windows executable's icon resource, linked by `build.rs`. Packs the six `hww-*.png` rasters 16 through 256 verbatim; ICO stops at 256, so the 512 has no entry. Regenerate after changing any of those PNGs:<br>`python3 -c "from PIL import Image; n=[16,32,48,64,128,256]; i={s:Image.open(f'assets/logo/hww-{s}.png') for s in n}; i[256].save('assets/logo/hww.ico', sizes=[(s,s) for s in n], append_images=[i[s] for s in n if s!=256])"`<br>Pillow reuses a supplied image whenever one matches a requested size and only resamples when none does, so every entry stays the raster drawn for it. |
| `hww-macos.svg` | the macOS variant of the master: the same drawing on a rounded tile of 824 inside a canvas of 1024, which is the grid every icon since macOS 11 is drawn to. Rasterised to `hww-macos-{64,128,256,512,1024}.png`. |
| `hww-macos-32.svg`, `hww-macos-16.svg` | the small grids again, redrawn around their own tile rather than scaled from the master. The 32's tile is 26 px at an offset of 3, the 16's is 14 px at an offset of 1: both land on whole pixels, and at 16 that is worth the 7% the tile then overruns the grid by, because a tile edge on a half-pixel is a grey border and the wave inside it loses its cores. |
| `hww-macos-*.png` | `rsvg-convert -w N -h N hww-macos.svg -o hww-macos-N.png`, 64 through 1024; take 16 and 32 from their own SVGs. Colour type 6, unlike the square set: the corners are transparent, which is the whole point. The 1024 is drawn for `icon_512x512@2x` and nothing else, and only this set has an entry that size — ICO stops at 256 and the Linux hicolor theme at 512. |
| `hww.icns` | the macOS bundle's icon, at `hww.app/Contents/Resources/hww.icns`. **Not committed:** `packaging/build-dmg.sh` assembles an `hww.iconset` of `icon_{16,32,128,256,512}x{…}.png` and their `@2x` twins, each *copied* from the `hww-macos-N.png` drawn at that size, then runs `iconutil -c icns` on it. `iconutil` exists only on macOS; building it on the runner from the committed rasters is what keeps a Mac out of the loop for anyone regenerating the logo. |

The rust tail is drawn first and starts one vertex back, running under the last
diagonal of the wave. Without that underlap the butt caps leave a wedge of
ground showing at the corner. Keep the draw order.

The wave runs edge to edge by design. Both ends sit clear of a 22% corner
radius, so an icon mask does not clip them.

macOS masks nothing, which is why there is a second set. An `.icns` is drawn as
it was authored, so a square raster stays square in the Dock, and one filling its
canvas stands taller than neighbours that stop at Apple's grid. The `hww-macos-*`
files carry the corner and the padding in the artwork; every other format takes
the square set, where the padding would be the defect. Change the master and both
sets have to be redrawn.
