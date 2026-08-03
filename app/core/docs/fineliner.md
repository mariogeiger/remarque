# Fineliner sample conversion

`FinelinerStrokeBuilder` reconstructs the device-independent conversion from
shared `PenSample` values to Xochitl's 14-byte `StrokePoint` values. It does not
own colors, read evdev, render pixels, or write `.rm` scene blocks.

For scene sample `p[i]`, view scale `z`, and pressure `r[i]`, the recovered
fineliner-v2 fields are:

- position: `p[i]` as two little-endian `f32` values;
- distance field: `round(5 z (|p[i] - p[i-1]| + |p[i-1] - p[i-2]|))`;
- width: thin `8`, medium `16`, or thick `24`, in quarter pixels;
- direction: the arithmetic mean of the two wrapped segment headings, mapped
  from `[0, 2π]` to `[0, 255]` by round-to-nearest;
- pressure: `round(255 clamp((r[i-1] + r[i]) / 2, 0, 1))`.

The first sample is used as both previous samples. This yields zero distance
and direction for the first stored point. Fineliner pressure remains metadata;
it does not alter width.

The stored width is not yet the exact live raster width. At display scale `z`,
the native antialiased renderer uses

`raster_width = z stored_width / 4 + 0.75` pixels.

A breakpoint after the native render call observed `4.75` for every point of a
medium line at scale `1`, while the packed width remained `16`. The `0.75`
term is selected by the active antialiased-renderer flag and is independent of
pressure.

## Evidence boundary

Thin width `8` and medium width `16` are exact fields observed in saved native
v6 scenes from the two controlled captures. The slow and fast lines also show
that the distance field varies with motion while width remains fixed. The
field equations and thick width `24` come from
`quantize_pen_sample_to_line_point` at `0x00ef7ac0` in the hashed 3.27.3.0
binary.

The distance multiplier is initialized to `1.0` in the ELF but is `2.5` in the
running Paper Pro process. The live fixture records the effective transform
scale separately and reproduces the resulting fields exactly.

The raw marker device emits substantially more reports than Xochitl converts
to line points. Qt delivery or an upstream input stage coalesces those reports.
A temporary breakpoint immediately after the native conversion captured 24
ordered input samples, the effective transform scale, and every packed output
field. `fineliner-medium-samples-3.27.3.0.json` reproduces all fields exactly.

A second breakpoint after the live render and image-copy calls captured the
primary BGRA pixels before and after 32 consecutive points. It established
that the renderer receives the original view coordinates exactly and uses a
`4.75`-pixel medium width. A breakpoint at the active triangle sink then showed
that every vertex carries half-width `2.375` and signed distance `±2.375`.
Coverage is full inside `half_width - 0.75`, falls linearly to zero at the
ribbon edge, and is blended with the native integer BGRA rule.

Before pen-up, the reconstructed four-corner ribbons, round joins, start cap,
and triangle rasterizer change exactly the same 52 pixels as the 32-point
native capture. Mean channel error is `2.404`; three intensities differ, with
maximum error `101`. The passing pixel fixture preserves both the exact support
requirement and these explicit intensity bounds. Completing a stroke adds the
corresponding rounded end cap through the native finalizer at `0x00f089e0`; the
incremental rasterizer keeps it absent while the stylus is still down.

A separate capture around 32 calls to the native triangle sink isolates the
lowest raster stage from stroke geometry and display updates. The Rust
triangle renderer changes the same 158 pixels with zero channel error.

Raster width remains floating point after the stored quarter-pixel width is
scaled. The exact relation is

`raster_width = 0.75 + view_scale * stored_width_quarters / 4`.

A large fineliner at captured scale `3.1425083` therefore renders at
`19.60505` pixels. Rounding the scaled quarter-pixel value first would produce
`19.5` and is not native behavior.

The dirty rectangle follows pixel-center coverage rather than adding a fixed
margin. For geometric edge `e`, the clipped integer bound is
`ceil(e - 0.5)`. The first segment in the synchronized trace therefore maps
exactly to the native 20-by-19 update rectangle. This keeps live partial
updates no larger than the pixels the segment can change.

Raw `pen.raw` still cannot be replayed directly because it precedes that
coalescing boundary. The live sample fixture starts at the mathematically
defined conversion boundary and isolates it from input scheduling.
