# Optical display-response bench

This bench measures the path from a tablet e-paper update request to the change
seen on the physical panel. It keeps software submission, vendor-queue drain,
camera presentation, and optical settling as separate observations.

## Components

- `CameraBench.swift` captures a JPEG or samples named luminance regions from
  an MX Brio through AVFoundation.
- `MeasureClockOffset.swift` measures the tablet-minus-Mac monotonic-clock
  offset with an NTP-style UDP exchange.
- `measure_display_response.py` aligns tablet events with camera frames and
  derives one row of timing metrics per display transition.
- `measure_pen_response.py` aligns a replayed pen contact with a dense optical
  grid and measures local input-to-visible-ink latency.
- `write_manifest.py` hashes every preserved campaign artifact.
- `device_epaper_timing_probe.cpp` exercises the exact in-tree production
  e-paper boundary and records submission and vendor-queue timings.
- `build-device-epaper-probe` cross-compiles that probe with the firmware SDK.
- `device_epaper_partial_update_showcase.cpp` contrasts a sequence of 64×64
  partial updates with one complete-screen color restore.
- `build-device-epaper-showcase` cross-compiles that visual demonstration.
- `regions/` contains the calibrated camera-space regions.
- `experiments/` preserves immutable inputs, outputs, and conclusions from
  completed runs.

The historical `device_timing_probe` and `device_monotonic_server` remain in
the Quill repository because they exercise its ABI directly.

## Clock model

For one UDP exchange, let `m1` and `m4` be Mac host-clock times around the
request and let `d2` and `d3` be tablet `CLOCK_MONOTONIC` times around the
reply. The estimated tablet-minus-Mac offset is

`o = ((d2 - m1) + (d3 - m4)) / 2`.

Each capture has a clock sample before and after it. The analyzer takes the ten
lowest-round-trip samples in each set, uses their median offset, and linearly
interpolates the offset across tablet time. Its conservative clock uncertainty
is the larger of half the median round trip and half the selected offset range.

AVFoundation presentation timestamps and `CMClockGetHostTimeClock()` share the
Mac host-time domain. Camera callback time is retained for diagnosis but is not
used as the optical observation time.

## Response definitions

For luminance `L(t)`, the analyzer estimates baseline `B` before submission and
final level `F` after queue drain. Direction-independent progress is

`q(t) = (L(t) - B) / (F - B)`.

- `visible_onset_ms`: first sustained crossing of `q = 0.10`.
- `first_90_ms`: first sustained crossing of `q = 0.90`.
- `settled_ms`: first frame after which `|q - 1| <= 0.10` through the final
  observation window.
- `visible_motion_ms`: settled time minus visible onset.
- `settled_minus_drain_ms`: optical settling time minus vendor queue-drain
  time. A negative value means the queue remained busy after the measured
  region had stabilized.

Crossings require two consecutive frames. At 60 frames per second, camera
sampling contributes about ±8.5 ms of quantization uncertainty in addition to
the reported clock uncertainty.

For moving pen input, the causal input time is when the ideal stroke first
intersects a sampled camera region, not when its center reaches the region
center. Along the measured motion axis, a region of radius `r` and a stroke of
radius `s` first overlap when their centers are `r + s` apart. The pen analyzer
applies this Minkowski-sum offset before interpolating between consecutive
`SYN_REPORT` input samples. This avoids folding traversal time across the
sample window into panel latency.

## Camera setup

The validated setup is 1920×1080 at 60 frames per second, HDR disabled,
1/120-second exposure, and ISO 500. The camera must remain fixed after region
calibration. The 2026-08-04 calibration maps the center of the tablet probes'
centered rectangles to approximately `(1000, 575)` in the 1920×1080 camera
image.

On the Mac, build the camera application once and grant it camera access:

```sh
./build-camera-bench /tmp/RemarqueCameraProbe.app
xcrun swiftc MeasureClockOffset.swift -o /tmp/measure-clock-offset \
  -framework AVFoundation
```

An ad-hoc rebuild changes the executable signature and can make macOS ask for
camera permission again. Reuse the same built application for one campaign.

Run a region capture with:

```sh
open -n /tmp/RemarqueCameraProbe.app --args sample /tmp/camera.csv \
  90 2 /tmp/tablet-center.csv 1920 1080 60
```

Analyze a synchronized capture with:

```sh
python3 measure_display_response.py \
  --camera camera.csv \
  --device device.csv \
  --clock-before clock-before.csv \
  --clock-after clock-after.csv \
  --output response.csv \
  --summary summary.csv
```
