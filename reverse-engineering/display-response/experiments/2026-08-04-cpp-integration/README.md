# Integrated e-paper boundary validation — 2026-08-04

This campaign validates Remarque's in-tree C++ e-paper boundary against the
earlier Quill baseline on the same reMarkable Paper Pro, firmware 3.27.3.0.
The probe compiles the exact production `paper_pro_epaper.cpp` and calls the
same stable C ABI as the Rust application. It neither links nor loads
`libquill.so`.

## Method

- Three centered square sizes: 64×64, 256×256, and 512×512 logical pixels.
- Two black/paper repetitions for each supported request: monochrome mode 0,
  color mode 3, and color mode 4.
- MX Brio at 1920×1080 and 60 frames/s, HDR off, 1/120 s, ISO 500.
- A calibrated 7×7 camera region centered at approximately `(1013, 578)`.
- Tablet and camera monotonic clocks aligned immediately before and after the
  60-second capture.
- Software submit, vendor queue drain, first visible change, 90% crossing, and
  optical settling measured independently.

The run contains 36 accepted requests, 36 completed queue drains, and 3,600
camera frames. Clock uncertainty is 3.5 ms. Camera sampling contributes about
±8.5 ms of quantization uncertainty.

A separate visual stress test submits twenty 64×64 monochrome regions at
120 ms intervals, each covering 0.117% of the 1620×2160 framebuffer, then
contrasts them with one complete-screen color restore. Its process map and log
confirm that `libquill.so` is absent.

## Baseline comparison

These medians use the 64×64 and 256×256 observations shared by both campaigns.
Times are milliseconds from request submission to first visible change.

| Request | Target | Integrated | Quill | Difference |
|---|---:|---:|---:|---:|
| monochrome mode 0 | black | 36.5 | 35.0 | +1.5 |
| monochrome mode 0 | paper | 44.2 | 42.1 | +2.1 |
| color mode 3 | black | 40.1 | 39.1 | +1.0 |
| color mode 3 | paper | 80.2 | 79.0 | +1.2 |
| color mode 4 | black | 673.8 | 696.8 | -23.0 |
| color mode 4 | paper | 487.8 | 497.8 | -10.0 |

The mode-0 and mode-3 differences are below one camera frame. The mode-4
differences are small relative to its roughly 0.5–0.7 second onset and are not
evidence of a regression at this sample count. The integrated boundary
therefore reproduces the Quill optical behavior within measurement resolution.

## Stateful controller cycle

One mode-3 paper transition at 256×256 and both at 512×512 entered a slower
vendor cycle. Their queue totals were 1,275–1,286 ms instead of 606–625 ms.
An isolated mode-3 run after reopening the firmware engine kept all twelve
queue totals between 606 and 630 ms.

This is not specific to the integrated boundary. The preserved Quill native
campaign exhibits the same sequence at the same global event numbers: paper
events 20, 22, and 24 take the slow path, while preceding mode-3 paper events
remain near 606–617 ms. The controller therefore carries update history and
periodically selects a quality/cleanup cycle. A median can hide this behavior,
so refresh policy must not treat queue duration as a fixed property of a mode.

## Preserved files

- `device_epaper_timing_probe.cpp` and `build-device-epaper-probe` in the bench
  root define the exact tablet stimulus and build.
- `device.csv` and `device.stderr` contain the synchronized full campaign.
- `camera.csv.zst`, `clock-before.csv`, `clock-after.csv`, and `region.csv`
  contain the optical input and clock model.
- `response.csv` and `summary.csv` contain per-transition and grouped results.
- `mode3-isolated.csv` and `mode3-isolated.stderr` isolate the controller's
  history-dependent slow cycle without a second camera capture.
- `calibration-before.jpg` and `calibration-black.jpg` preserve the framing and
  sampled patch.
- `partial-vs-global-showcase.mp4`, `showcase.log`, and `showcase.stderr`
  preserve the annotated stress test and its device-side evidence.
- `manifest.json` hashes every preserved artifact.

Decompress the camera samples with `zstd -d camera.csv.zst`.
