# Remarque pen display-policy comparison

Each directory contains camera luminance, clock anchors, injection write times,
per-region response, and a self-describing summary for one replay of the same
captured pen trace. The source input and spatial regions live one directory up.

| Directory | Submission rule | Median visible onset |
|---|---|---:|
| `uncoalesced/` | submit every changed pen frame | 109.8 ms |
| `batch-coalesced/` | submit once after each evdev drain | 105.8 ms |
| `frame-paced/` | preserve all samples; submit at most every 16 ms | 99.9 ms |

All runs use events 13 through 6,447, the default thin black fineliner, and a
blank second page in `Carnet 2`. The analyzer uses the same 7×7 optical regions,
20% sustained threshold, clock model, raw-to-camera calibration, and 6-pixel
causal overlap offset as the native result.

The virtual marker remains open after replay until the tablet application has
stopped. This matters operationally: destroying a bound `uinput` device under
an active reader returns `ENODEV`. Keeping separate start and end trigger files
makes setup, capture, and teardown deterministic.

Reproduce one derived result from this directory with:

```sh
python3 ../../../../measure_pen_response.py \
  --camera frame-paced/camera.csv.zst \
  --injection frame-paced/injection.jsonl.zst \
  --clock-before frame-paced/clock-before.csv \
  --clock-after frame-paced/clock-after.csv \
  --input-axis x --raw-start 2448 --raw-end 6366 \
  --camera-start 300 --camera-end 630 \
  --camera-fixed-axis x --camera-fixed-coordinate 940 \
  --region-radius 3 --stroke-radius 3 \
  --output /tmp/response.csv --summary /tmp/summary.json
```

The final policy reduces median onset by 9.9 ms relative to the uncoalesced
baseline and reduces redundant panel work by construction. Its 99.9 ms median
still trails the native 71.0 ms result by 28.9 ms. Since the applications draw
different raster widths and use different rendering stacks, the remaining gap
is a measured system difference rather than a waveform-only estimate.
