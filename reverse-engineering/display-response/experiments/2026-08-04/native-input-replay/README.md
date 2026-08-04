# Native pen input replay

This experiment closes the input-to-panel loop for one Xochitl fineliner
contact. It preserves the original raw input, exact replay write times, camera
samples, clock anchors, spatial calibration, derived response, and final
photograph.

## Replayed contact

The source trace first contained a short setup contact, then a 5.21 s pen
contact. `measure_pen_response.py` selects the longest contact automatically.
Its X coordinate increased from 2,448 to 6,366 while Y remained near 8,600.
The projected ink crossed camera coordinates `(940, 300)` through `(940, 630)`.

Each camera region is 7×7 pixels and the projected stroke radius is 3 pixels.
For motion toward increasing camera Y, the first geometrically possible
intersection with a region centered at `y` occurs when the stroke center
reaches `y - 6`. Input time is linearly interpolated between the surrounding
`SYN_REPORT` samples at that causal coordinate. Optical onset is the first two
consecutive 60 Hz frames at or beyond 20% of the final luminance change.

Run the preserved analysis from this directory with:

```sh
python3 ../../../measure_pen_response.py \
  --camera pen-camera.csv.zst --injection pen-injection.jsonl.zst \
  --clock-before pen-clock-before.csv --clock-after pen-clock-after.csv \
  --input-axis x --raw-start 2448 --raw-end 6366 \
  --camera-start 300 --camera-end 630 \
  --camera-fixed-axis x --camera-fixed-coordinate 940 \
  --region-radius 3 --stroke-radius 3 \
  --output pen-response.csv --summary pen-summary.json
```

The result contains 33 locations: median visible onset 71.0 ms and median
absolute deviation 21.5 ms. Clock uncertainty is 2.9 ms; camera quantization
is ±8.5 ms. The 10.7–158.5 ms range also contains spatial calibration and
local contrast variation, so the median is the stable campaign result.

## Files

- `pen-input.raw`: original 24-byte Linux `input_event` records.
- `pen-injection.jsonl.zst`: source index, source-relative time, and tablet
  monotonic write bounds for every injected event.
- `pen-camera.csv.zst`: 60 Hz luminance from the dense camera grid.
- `pen-regions.csv`: camera-space region definitions.
- `pen-clock-before.csv`, `pen-clock-after.csv`: clock-alignment exchanges.
- `pen-response.csv`, `pen-summary.json`: reproducible derived measurements.
- `pen-after.jpg`: physical panel after the replay.
- `eraser-*`: the rejected first capture, preserved because its raw events
  prove `BTN_TOOL_RUBBER=1` and explain the observed eraser behavior.

The pen appears during contact and remains stable after touch release and tool
exit. The lift triggers finalization, but the panel does not wait for it before
showing the stroke.

## Rust comparison

`rust-comparison/` replays source events 13 through 6,447 into Remarque on a
blank page. Three complete optical runs isolate the display-submission policy:

| Remarque policy | Regions | Median onset | MAD |
|---|---:|---:|---:|
| every input frame | 30 | 109.8 ms | 24.2 ms |
| one update per drained batch | 33 | 105.8 ms | 15.0 ms |
| batch plus 16 ms display pacing | 32 | 99.9 ms | 20.8 ms |
| native Xochitl | 33 | 71.0 ms | 21.5 ms |

Pacing preserves every geometric input sample while bounding panel requests to
one per 16 ms. It improves the paired-trace Remarque median by 9.9 ms. A 28.9
ms median gap to Xochitl remains; this campaign does not attribute that gap to
one subsystem.
