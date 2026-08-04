# Paper Pro display response — 2026-08-04

This campaign measured Quill requests on a reMarkable Paper Pro running
firmware 3.27.3.0 and compared them with a display request intercepted from the
same Xochitl binary. The panel waveform was
`GAL3_AAB0BV_ID3511_AC118TC1F2_AD1004-LHA_TC.eink`.

## Controlled variables

- Rectangle sizes: 64×64, 256×256, and 768×768 logical pixels, centered.
- Targets: black, paper, and red where the waveform supports color.
- Two repetitions per size and target pair. Paper after native-live black and
  red yields four paper observations per size.
- Camera: MX Brio, 1920×1080, 60 frames/s, HDR off, 1/120 s, ISO 500.
- Optical signal: mean Y-plane luminance in the calibrated 20×20 camera region.
- Tablet and camera clocks aligned immediately before and after each run.

The standard run has 50 transitions and 5,400 camera frames. The native-mode
run has 38 transitions and 6,000 camera frames. A separate native-input run
replayed 6,448 captured pen events while sampling 817 camera regions. Every
display request was accepted and every vendor queue drain completed.

## Results

The following values are medians across all three rectangle sizes. Times are
milliseconds from the software request except `submit` and `motion`.

| Request | Target | n | submit | queue empty | visible | settled | motion |
|---|---:|---:|---:|---:|---:|---:|---:|
| mono, mode 0, flags 0 | black | 6 | 2.2 | 346.9 | 37.1 | 81.9 | 46.0 |
| mono, mode 0, flags 0 | paper | 6 | 2.1 | 347.1 | 45.0 | 131.0 | 77.7 |
| color, mode 4, flags 0 | black | 6 | 10.1 | 1288.1 | 713.1 | 787.9 | 78.3 |
| color, mode 4, flags 0 | paper | 12 | 5.4 | 1276.0 | 508.1 | 649.8 | 133.0 |
| color, mode 4, flags 0 | red | 6 | 4.5 | 1274.9 | 366.3 | 1129.2 | 762.9 |
| color, mode 3, flags 0 | black | 6 | 4.9 | 617.1 | 47.4 | 82.3 | 30.8 |
| color, mode 3, flags 0 | paper | 6 | 5.7 | 617.1 | 83.5 | 155.3 | 63.6 |
| color, mode 3, flags 2 | black | 6 | 4.8 | 618.7 | 47.4 | 82.7 | 35.2 |
| color, mode 3, flags 2 | paper | 12 | 5.7 | 618.9 | 241.9 | 409.6 | 138.6 |
| color, mode 3, flags 2 | red | 6 | 4.9 | 617.0 | 41.6 | 538.1 | 497.5 |

Per-size medians and every individual transition remain in the summary and
response CSVs; the table above is not the source of truth.

## Native replay evidence

The native observer intercepted live Xochitl drawing requests at
`prepare_and_queue_epaper_update` (`0x00b9be80`). A representative raw request
was:

`[352, 713, 370, 732, 2, 1, 0, 3]`.

The first four words are rectangle bounds. Static control-flow recovery and
the `EPFramebuffer::swapBuffers(QRect, EPContentType, EPScreenMode, QFlags)`
ABI identify the remaining words as flags `2`, content `1`, temperature `0`,
and mode `3`. Replaying content `1`, mode `3`, flags `2` through Quill produced
the `native-live` rows above.

The observer also captured a request whose last word was `14`. Directly
passing mode 14 back to the vendor library produced six `Invalid screen mode`
warnings. Although all requests returned and visibly changed the panel, the
largest paper transition fell back to a much slower response: 414 ms to first
visibility and 527 ms to settle. Mode 14 is therefore evidence about an
internal Xochitl path, not a supported Quill primitive.

## End-to-end pen replay

The input replay recreated Xochitl's hard-coded marker device with `uinput`
and injected the original event sequence with its recorded timing. A 5.21 s
pen contact crossed 33 calibrated optical regions. The local visible-onset
median was 71.0 ms, with a 21.5 ms median absolute deviation. Clock uncertainty
was 2.9 ms and 60 Hz camera quantization contributed ±8.5 ms.

This is an end-to-end measurement from the write of the causally relevant
input event to physical ink. The causal coordinate includes the sum of the
3-pixel sampled-region radius and 3-pixel stroke radius. The contact rendered
continuously; lifting the pen finalized and cleaned the stroke but was not
required for first visibility.

The same input was then replayed through Remarque. Exact-frame submission,
batch coalescing, and 16 ms display pacing measured 109.8, 105.8, and 99.9 ms
median onset respectively. The shipped pacing rule therefore removes 9.9 ms
from the baseline while retaining every stroke sample. Xochitl remains 28.9 ms
ahead on this paired trace, so further work should measure the renderer-to-swap
boundary before changing waveform policy again.

## Findings

1. Software conversion is not the dominant interactive delay. It ranges from
   below 1 ms for small rectangles to roughly 48 ms for the largest requests.
2. Waveform choice dominates what the user sees. Mode-3 black begins in
   30–75 ms; the previous mode-4 color path begins hundreds of milliseconds
   later.
3. Pitch-black interaction feedback is structurally advantageous: both native
   mode-3 variants stabilize black in roughly 60–117 ms across the tested
   sizes.
4. Vendor queue drain is not optical settling. The measured region usually
   stabilizes 70–780 ms before the queue reports empty. Queue state remains
   useful for serialization, not as a glass-settled sensor.
5. Rectangle area mainly affects software submission and modestly shifts onset;
   it does not erase the order-of-magnitude difference between waveform modes.

These observations support a three-phase display policy: fast mono for live
geometry, supported color mode 3 for interactive color UI, and mode 4 or a
full refresh only for final-quality cleanup. Mode 14 is rejected because the
vendor library explicitly reports it as invalid.

## Limitations

- The camera signal is luminance only. Mode-3 chroma accuracy and long-term
  ghosting still require a color or spectrometric check.
- Sample counts are deliberately small. The conclusions are large effects,
  not precise population estimates.
- The optical metric covers the centered region, not every pixel on the panel.
- The pen result contains one long replayed contact. Its 33 spatial samples
  characterize that trace, not variation across writers, widths, and tools.

## Preserved files

- `calibration-camera.csv.zst`, `calibration-device.csv`: camera grid and tablet
  events used to locate the projected update center.
- `camera-calibration.jpg`, `camera-calibration-rectified.png`: raw and rectified
  views used to verify framing.
- `standard-*`: standard Quill waveform inputs, clocks, camera samples,
  per-transition response, and grouped summary.
- `native-mode-*`: replayed mode-3 inputs and corresponding outputs.
- `mode14-*`: bounded unsupported-mode experiment, including the vendor
  warnings that disqualify it from application use.
- `native-input-replay/`: raw source input, injection times, camera samples,
  calibration, photograph, derived response, and method notes.
- `quill-display-timing.patch.zst`: the exact Quill instrumentation patch
  against upstream commit `39262ee` used for the software-request and queue
  timestamps.

Decompress a camera CSV or the Quill patch with `zstd -d FILE.zst`.
`manifest.json` records the exact hashes and provenance.
