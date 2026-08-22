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

The rust tail is drawn first and starts one vertex back, running under the last
diagonal of the wave. Without that underlap the butt caps leave a wedge of
ground showing at the corner. Keep the draw order.

The wave runs edge to edge by design. Both ends sit clear of a 22% corner
radius, so an icon mask does not clip them.
