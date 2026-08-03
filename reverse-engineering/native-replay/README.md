# Native replay

This crate is a differential conformance harness. It replays recorded native
inputs through `remarque-core` and compares layer-specific outputs. Performance
measurements may be added later, but correctness and timing are kept separate.

## Controlled scenario

Every fixture contains:

- the Xochitl binary hash and firmware version;
- a named operation and its complete ordered input;
- output observed from the native device, never inferred from decompiled C;
- an explicit comparator and tolerance;
- references to the raw capture from which the fixture was distilled.

Fixtures are immutable observations. A new firmware or corrected capture gets
a new fixture rather than silently changing an old oracle.

## Comparison layers

1. **Structure**: decoded `.rm` points, colors, widths, and erased sections;
   compare exact fields whenever their representation is deterministic.
2. **Geometry**: focal points, scales, transformed coordinates, and bounds;
   compare with an explicit numeric tolerance.
3. **Pixels**: canonicalized BGRA images; compare exact pixels where possible
   and report bounded error where rasterization differs.
4. **Display schedule**: dirty rectangles, supersession, waveform class, and
   timestamps; compare ordering and distributions separately.

One scenario changes one controlled input dimension. A mismatch reports the
fixture, native value, reconstructed value, and allowed tolerance.

Run all recorded scenarios with:

```sh
cargo test -p remarque-native-replay
cargo run -p remarque-native-replay
```

Current fixtures cover two native pinch ratios, thin and medium fineliner
widths, 24 consecutive medium-fineline conversions, and one 32-point medium
pixel raster. Point conversion compares position, two-segment distance, width,
direction, and pressure exactly. The pixel fixture requires exact support and
bounds its observed intensity error independently. The raw marker stream still
precedes Xochitl's input-coalescing boundary.

## Live raster diagnosis

`compare_fineliner_raster` diagnoses a raw breakpoint capture before its
canonicalized support and intensities are promoted into the fixture set:

```sh
cargo run -p remarque-native-replay --bin compare_fineliner_raster -- CAPTURE PREFIX_BYTES
```

The first two 3.27.3.0 captures use `PREFIX_BYTES=16`: framebuffer allocation
discovery originally returned the allocation prefix rather than the first BGRA
pixel. The comparator corrects that provenance explicitly; new captures use
the corrected pixel address.

For the 32-point medium capture, native rendering and the reconstructed ribbon
both change the same 52 pixels. Mean BGRA error is `2.404`; all but three pixels
are exact, and the remaining error is kept explicit in the fixture. The old
capsule renderer changed 83 pixels. The recovered pipeline constructs
four-corner ribbons, fills joins separately, interpolates signed-distance
coverage over triangles, and uses a `0.75`-pixel inward antialias transition.
An independent 32-call triangle-sink capture compares 158 changed pixels with
zero BGRA-channel error, so the remaining whole-stroke intensity residual is
above the triangle raster stage.

For longer synchronized captures, `capture-native-stroke-trace` records raw
marker frames, native line points, triangle calls, update requests, and raster
snapshots on one monotonic timeline. `summarize-native-stroke-trace` validates
the bundle and reports per-stroke cadence, finalization work, render
destinations, and the output difference rectangle.
