# Xochitl reverse engineering

This document records observations about the exact Xochitl build on the test
reMarkable Paper Pro. It separates measured facts from hypotheses so that later
probes remain reproducible.

## Target

- Device: reMarkable Paper Pro (`imx8mm-ferrari`, AArch64)
- Firmware: `3.27.3.0`
- Binary: `/usr/bin/xochitl`
- SHA-256: `9749880daa2f10844e77b560ec0ecddd1634d43eb328af637c7026edf3ef120e`
- ELF: 64-bit AArch64, dynamically linked, non-PIE, stripped
- Build ID: `c1093d373e2de94adf226821038ed4db706cf3d1`

The binary copied from the device is kept under `.build/`, which is ignored by
Git. No system file on the tablet has been changed or replaced. A standalone
Arm64 `strace` executable was added at `/home/root/strace` for temporary runtime
probes; it is not installed as a package or started as a service.

## Static observations

Xochitl is a Qt 6 application. Its dynamic dependencies include Qt Quick, QML,
GUI, DBus, Network, WebSockets, PDFium, libdrm, libudev, systemd, and several
reMarkable `libcsl*` hardware services.

The stripped binary still contains:

- Qt meta-object names and C++ RTTI;
- source paths from the original build;
- QML resource paths and substantial QML source text;
- logging strings that identify internal modules and operations;
- unwind information that gives Ghidra useful function boundaries.

The QML resource tree exposes modules for the home screen, documents, scene
view, pen input, power management, sleep screen, screen sharing, Wi-Fi, and the
other top-level features. This makes UI reconstruction a separate and easier
problem than reconstructing the native drawing engine.

## Input boundary

The running process opens all four input devices:

| Device | Kernel name |
| --- | --- |
| `/dev/input/event0` | `30370000.snvs:snvs-powerkey` |
| `/dev/input/event1` | `Hall effect sensors` |
| `/dev/input/event2` | `Elan marker input` |
| `/dev/input/event3` | `Elan touch input` |

This confirms that a replacement shell can obtain pen, touch, lid, and power
events from ordinary Linux evdev interfaces.

## Power-management boundary

The native binary names `powerButtonSuspend`, `goToSleep`, suspend delays,
slumber inhibitors, wake reasons, and the exact command target
`suspend-then-hibernate.target`. Its diagnostic strings identify inhibitors
for pending document stores and imports, sync, software updates, Wi-Fi
refresh, and screen sharing. These are application-level gates before the
firmware transition, not hidden kernel operations.

On this firmware, `suspend-then-hibernate.target` delegates to
`systemd-sleep suspend-then-hibernate`; its configured hibernation delay is
four hours. The shared system-sleep hooks inhibit the power key, choose wake
sources, force charger mode, unload and reload Wi-Fi/Bluetooth, record the
transition, and manage autosleep. Hibernate additionally selects the Falcon
boot flow and handles OP-TEE.

The kernel exposes completed and failed transitions under
`/sys/power/suspend_stats`. A systemd command returning is therefore
insufficient evidence that the device slept: the e-paper regulator can still
abort the transition while its post-update discharge timer is active. A
replacement application must confirm that the success counter advanced and
retry bounded failures.

### Replacement-path validation

On 2026-08-04, the Remarque path showed its sleep screen, waited for the
30-second panel discharge interval, and requested suspend-then-hibernate. The
first kernel transition returned `Resource temporarily unavailable`; the
bounded retry then advanced the successful-suspend counter from 3 to 4. After
the physical-button wake, the same graphical process ID was still running,
its durable document state had the same hash, autosleep and the application
wake lock were restored, and WPA reported a completed connection. This
confirms both the need for counter-based retry and the process-continuity
contract.

## Display boundary

The display backend is an internal class named `EPFramebuffer`. Its constructor
was identified at virtual address `0x009c5c20` in this build.

Measured initialization sequence:

1. Take the singleton lock `/tmp/epframebuffer.lock`.
2. Load waveform tables and panel calibration.
3. Open `/dev/dri/card0`.
4. Enable the DRM universal-planes client capability.
5. Create 16 DRM dumb buffers with ioctl `0xc02064b2`.
6. Register them with `drmModeAddFB`.
7. Map them with ioctl `0xc01064b3` and `mmap`.
8. Start real-time threads named `vsync-flip` and `framegen` at FIFO priorities
   99 and 98 respectively.

Each DRM buffer is `405 * 1084 * 4 = 1,756,080` bytes. The logical drawing
surface is 1620 by 2160. Its primary Qt image is 32-bit with a padded stride of
1632 pixels, or 6528 bytes per row. A second image uses an 8-bit format with a
1632-byte stride.

The dimensions and the framebuffer metadata reveal the packing structure:

- `405 = 1620 / 4`;
- `1080 = 2160 / 2`;
- each DRM pixel has a 32-bit pitch but depth 24 (`RGB888` plus one padding
  byte);
- `405 * 1080 * 3 * 8 = 1620 * 2160 * 3` bits.

Each 32-bit DRM word therefore contains eight packed three-bit logical-pixel
codes in its low 24 bits. Live buffer captures confirm this directly: the
fourth byte of every changed word remains zero, and splitting the other 24 bits
into eight consecutive three-bit fields produces uniform codes. The buffer has
four additional DRM rows, totalling 6480 bytes. Frame-generation code applies
a four-row offset when addressing the DRM buffer; the exact visible-coordinate
convention at that boundary still needs a controlled edge test.

The earlier four-bit inference counted the 32-bit storage padding as payload;
the DRM depth and captured bytes disprove it.

### Live DRM topology

A read-only probe enumerated the live DRM objects and atomic properties:

| Kind | Object ID | Relevant state |
| --- | ---: | --- |
| CRTC | 34 | `ACTIVE=0`, `MODE_ID=0` while idle |
| Connector | 36 | LVDS, `CRTC_ID=0` while idle |
| Primary plane | 32 | one plane, 405 by 1084 |

The plane exposes the ordinary atomic properties `FB_ID`, `CRTC_ID`, source
and destination rectangles, and fences. Every observed Xochitl atomic commit
had flags zero and changed exactly one property on plane 32: `FB_ID` (property
17). No reMarkable-specific atomic property or private display ioctl appeared.

Xochitl owns framebuffer IDs 37 through 52. Querying them through the DRM API
confirms that all 16 have geometry 405 by 1084, pitch 1620, bpp 32, and depth
24. Their Xochitl mappings are 1,756,080 bytes each.

## Hypotheses and predictions

### H1: QML can be recovered independently of the C++ decompilation

Prediction: observing Qt resource registration, or reading the live Qt resource
tree through a small injected probe, will map hashed QML cache files back to
their `qrc:` paths and recover most declarative UI code.

### H2: The essential display ABI is smaller than `EPFramebuffer`

Prediction: tracing only DRM ioctls during controlled full-screen changes will
show a small repeated protocol: buffer preparation, a commit or device-private
ioctl, and a completion event. The waveform and frame-generation code can then
be divided from the ABI needed by an independent application.

The runtime probes confirm this hypothesis. During updates, the display thread
emits only `DRM_IOCTL_MODE_SETCRTC` and repeated `DRM_IOCTL_MODE_ATOMIC` calls,
and every decoded atomic commit only changes the primary plane's standard
`FB_ID` property. Waveform selection, transition lookup, and three-bit packing
all occur in userspace before that commit.

### H3: Display updates are event-driven while idle

Prediction: an idle trace contains no display ioctl traffic. A first idle trace
over several seconds confirmed this: only an unrelated filesystem-label ioctl
was observed.

## Controlled runtime traces

The tracer attached to the existing Xochitl process and its threads. Each trace
was stopped with `SIGINT`, which detaches the tracer without stopping Xochitl.
The tablet and the framebuffer observer were checked after each experiment
and remained responsive.

Only `ioctl`, `poll`, `ppoll`, and timing metadata were collected. No document
or network contents were captured.

| Experiment | Atomic commits | `SETCRTC` | First-to-last display call |
| --- | ---: | ---: | ---: |
| Idle | 0 | 0 | n/a |
| One short black stroke | 92 | 2 | 3.089 s |
| One page change | 346 | 4 | 6.590 s |

All observed display ioctls came from thread 491, whose live kernel thread name
is `vsync-flip`. This agrees with the thread name recovered from the constructor
decompilation.

### One-stroke phases

The stroke produced:

1. one `SETCRTC` followed by 84 atomic commits over 1.005 s;
2. after a 0.995 s gap, eight atomic commits over 77 ms;
3. after a 1.011 s gap, one final `SETCRTC`.

The main phase is approximately 84 commits per second. The eight-commit tail is
not part of continuous pen sampling because it starts after a one-second gap.
It is likely a delayed stabilization or cleanup phase, but that interpretation
still needs a buffer-content comparison.

### Page-change phases

The page change produced:

1. one `SETCRTC` followed by 229 atomic commits over 2.693 s;
2. after a 1.012 s gap, one `SETCRTC`;
3. after 338 ms, one `SETCRTC` followed by 109 atomic commits over 1.277 s;
4. after 178 ms, eight atomic commits over 81 ms;
5. after 1.012 s, one final `SETCRTC`.

The main page transition is approximately 85 commits per second. The common
eight-commit tail and one-second delay in both experiments are evidence for a
shared finalization state. The extra 109-commit phase is specific to the larger
page transition in these two samples.

The exact cadence and phase lengths support a waveform state-machine model.
These timing-only traces did not establish the commit contents; the decoded
capture below does.

## Decoded atomic commits and buffer contents

A small ptrace probe now reads the pointer arrays passed to
`DRM_IOCTL_MODE_ATOMIC` at syscall entry. It does not change arguments or
Xochitl memory. A live stroke capture produced 145 commits over 1.752 seconds.

The main phase cycles through framebuffer IDs 37 through 51 in ascending order,
wrapping after 51. Framebuffer 52 is not part of this ring: Xochitl submits it
twice at the end. The final eight-commit tail had the sequence
`48, 49, 50, 51, 37, 38, 52, 52`. This identifies 15 temporal working buffers
and one final buffer.

Reading the shared DRM mappings after completion found all 16 buffers cleared
to zero. Capturing them at atomic-commit entry instead recovered nonzero drive
data. In the first live capture, exactly the same set of 854 logical pixels had
code 1 in framebuffers 43 and 45 and code 3 in framebuffers 44 and 46; all other
logical pixels had code zero. This proves that the three-bit fields are
per-logical-pixel temporal drive codes rather than grayscale pixels. The exact
electrical meaning of codes 0 through 7 still needs controlled transitions.

### Controlled short stroke

A later probe separates update events at idle gaps and snapshots each DRM
buffer on its first submission within an event. A controlled short black stroke
used 294 logical-pixel positions and 24 distinct temporal code sequences. The
dominant sequences use code 6 through most of the phase, sometimes with code 5
for one or two frames; shorter edge sequences use code 4. The final framebuffer
52 was zero. This is consistent with waveform-table output for solid and
antialiased parts of the stroke, but mapping the codes to electrical voltages
requires black-to-white and controlled-gray transitions too.

Copying one 1.76 MB buffer at each first submission stretched those particular
commits to about 35 ms. The captured bytes and ordering remain useful, but this
instrumented run must not be used for native cadence measurements.

### Frame-generation algorithm

The `framegen` thread entry is `0x009d6510`. Its generation dispatcher is
`0x009d6310`, with one-through-five-frame specializations. The simplest path at
`0x00ba4550` shows the core algorithm:

1. A 16-bit per-pixel transition index selects an entry in the active waveform
   table.
2. Each 16-bit waveform entry packs five consecutive three-bit drive codes
   (15 used bits).
3. The requested phase selects one of those three-bit fields.
4. Eight selected codes are packed into the low 24 bits of one 32-bit DRM word.
5. NEON processes groups of pixels and multiple output frames in parallel.

The update-coordinate conversion at `0x009cc250` doubles the horizontal
coordinate and halves the vertical coordinate before generation. Together with
the eight-code word, this is a swizzle of two logical display rows into one DRM
row: adjacent internal positions represent the two vertical pixels of a
logical column, and four logical columns fill one DRM word. This explains both
`405 = 1620 / 4` and `1080 = 2160 / 2` without padding bits being mistaken for
pixel data.

### Waveform and color tables

The active panel-specific waveform reported by Xochitl is:

`/usr/share/remarkable/GAL3_AAB0BV_ID3511_AC118TC1F2_AD1004-LHA_TC.eink`

It is 277,963 bytes with SHA-256
`5f36dce7a27610143d552a93a50193be1734e2364a515676ea1ad278221f8fdc`.
The firmware contains many such files for different panel lots; Xochitl selects
one using panel identity rather than assuming a universal waveform.

The waveform loader at `0x009d9ee0` delegates the vendor-file parsing to the
functions around `0x00da2710` through `0x00da2d68`, then converts the result to
the runtime representation used by `framegen`. The converted structure has:

- 32 tone states;
- a 32 by 32 source-to-target transition matrix (1024 entries) per mode and
  temperature range;
- a variable phase count for each mode and temperature;
- five three-bit phases packed into each 16-bit transition-table word.

Recovered mode names include `STANDARD`, `TEXT`, `TEXT_GLR`, `PMODE`, `TMODE`,
and `mode10` variants.

Xochitl also loads `ct33_{std,best,pen,fast}.bin` through the exact-size reader
at `0x009d14f0`. Each file is 287,496 bytes, exactly `33^3 * 8`: a 33 by 33 by
33 color cube with eight unsigned-byte thresholds at each lattice point. All
four files are byte-identical in this firmware. Every record is monotonically
nondecreasing. The values are cumulative quantization thresholds rather than
eight color components.

The conversion dispatcher at `0x009ca9f0` selects among these tables by update
mode. The core lookup at `0x00ba4e80` processes eight BGRA pixels in parallel.
For each color component `c`, it uses lattice coordinate `c >> 3` and residual
`(c & 7) + (c == 255)`. It orders the three residuals and selects four vertices
through the containing cube, so the interpolation is tetrahedral rather than
trilinear. The four integer weights sum to eight. Each interpolated threshold
therefore lies in `0..2040` without a division.

Those eight thresholds are compared with a spatial threshold from a 64 by 64
matrix embedded at `0x01632838`. Its rows have a padded stride of 72 16-bit
entries. The used entries cover every integer from 0 through 2039; 2024 values
occur twice and 16 occur three times. The eighth table threshold is always 255,
so its weighted value is always 2040 and can never pass. Counting the other
passed thresholds produces exactly one of eight output levels, 0 through 7.
Later conversion paths add their special-state flag bits; all-white BGRA, for
example, becomes `0x80` rather than level zero.

The firmware also contains byte-identical 524,288-byte `colortable_*` files.
The earlier `64^3 * 2` interpretation was only a size coincidence. Their actual
layout is 65,536 RGB565 entries times the same eight ordered byte thresholds:
the fastest axis is five-bit blue, the middle axis six-bit green, and the
slowest axis five-bit red, so the entry number is the native RGB565 value.
The files represent the same eight-level cumulative-quantization scheme as
`ct33`, apparently as a pre-expanded alternative. Their values are close to,
but not bit-identical to, resampling `ct33` with Xochitl's runtime tetrahedral
interpolator. No `colortable_*` filename or direct load was found in this
Xochitl binary, so this build uses `ct33`, not the RGB565 tables.

## Drawing pipeline

The drawing path is separate from the e-ink conversion. QML-facing
`ScenePenInputHandler` delegates ordinary line input to `PenInputLineHandler`.
The common begin, append, live-render, finish, and cancel routines are at
`0x00841ea0`, `0x008441a0`, `0x00f00830`, `0x00843eb0`, and `0x00844890`.
Finishing emits the completed line to the scene layer; cancelling clears the
temporary render mask and does not emit a line.

Each raw pen sample entering this path contains five 32-bit floats: `x`, `y`,
pressure, and two tilt components. `0x00ef7ac0` derives and appends a packed
14-byte line point:

| Offset | Type | Meaning |
| --- | --- | --- |
| 0 | `float` | x |
| 4 | `float` | y |
| 8 | `uint16` | speed-like value, quarter-unit fixed point |
| 10 | `uint16` | width, quarter-pixel fixed point |
| 12 | `uint8` | direction angle mapped onto `0..255` |
| 13 | `uint8` | pressure mapped onto `0..255` |

Tool IDs 4 and 17 are the legacy and v2 fineliner. Both use the simplest point
construction branch: position is unchanged, width is constant for the selected
thickness and quantized to quarter pixels, and pressure changes the stored
opacity value rather than width. `0x00ef7ac0` computes the effective width as

`w = max(2, 2 t)` pixels,

where `t` is the UI thickness enum value recovered at `0x0091be60`: thin `1`,
medium `2`, or thick `3`. It stores `4 w` as an unsigned quarter-pixel value.
The exact presets are therefore:

| Preset | Effective width | Stored width |
| --- | ---: | ---: |
| Thin | 2 px | 8 |
| Medium | 4 px | 16 |
| Thick | 6 px | 24 |

The first pressure value used for a segment is the clamped mean of the current
and previous raw pressures.

`0x00915c40` also constructs the v2 fineliner's tool-specific color model.
The RGB values passed to its color-record constructor are:

| Color | ID | RGB |
| --- | ---: | --- |
| Black | 0 | `#000000` |
| Gray | 1 | `#7a7776` |
| White | 2 | `#ffffff` |
| Blue | 6 | `#304ae0` |
| Red | 7 | `#c23132` |
| Green | 10 | `#91da71` |
| Cyan | 11 | `#74d2e8` |
| Magenta | 12 | `#c07fd2` |
| Yellow | 13 | `#fae719` |
| Orange | 14 | `#feb200` |

Live fineliner rendering selects the solid raster pipeline at `0x00f04740`.
The v2 path uses the antialiased solid implementation; the legacy path uses the
non-antialiased implementation. Each accepted point is transformed from scene
to display coordinates and sent immediately to the active rasterizer. The
damaged rectangle is accumulated, copied before modification for e-ink update
bookkeeping, and emitted again with the completed line at pen-up.

The solid geometry is explicit rather than a `QPainter` spline. For consecutive
points `p0` and `p1`, `append_point_to_antialiased_ribbon` at `0x00f08680`
computes `d = p1 - p0`, its length `L`, and a unit normal. It offsets `p0` and
`p1` by their respective half-widths and rasterizes the resulting four-corner
ribbon. `fill_antialiased_ribbon_join` fills direction changes and
`fill_antialiased_ribbon_start_cap` fills the cached first-segment cap. The
live point path exposes only that first cap. At pen-up,
`finish_antialiased_ribbon` at `0x00f089e0` invokes the same cap tessellator for
the other endpoint, producing the symmetric rounded end cap, then resets the
incremental ribbon state.

The live raster width differs from the packed scene width. The generic solid
path computes

`display_width = 0.75 + scene_to_view_scale * stored_width / 4`.

A controlled medium-fineline capture at scale `1` observed `4.75` at the
internal ribbon state for all 32 points, while each packed point retained width
`16`. The internal render coordinates matched the original view coordinates
within `3.5e-5` pixels. A breakpoint at the active virtual triangle sink
identified `fill_bgra_antialiased_triangle` at `0x00f169e0` and captured all six
floating-point coverage coordinates. Each vertex carried half-width `2.375`
and signed distance `±2.375`.

`rasterize_bgra_antialiased_triangle_scanlines` at `0x00f16270` interpolates
those two values. Coverage is opaque when
`abs(distance) < half_width - 0.75`, decreases linearly across the final
`0.75` pixel, and is zero outside the ribbon. A direct native capture around 32
triangle calls compared the triangle rasterizer independently of geometry and
display updates: all 158 changed pixels and every BGRA channel matched exactly.
In the older whole-stroke capture, the Rust reconstruction changes exactly the
native set of 52 pixels; three accumulated intensities differ, with mean channel
error `2.404` and maximum error `101`. The former capsule model changed 83
pixels and is rejected by the fixture.

A synchronized two-stroke trace links the kernel marker stream to all of these
boundaries. The two touch intervals contained 153 and 374 raw `SYN_REPORT`
frames; Xochitl stored exactly 153 and 374 points. Thus every touching marker
frame in this capture crossed the native point boundary. Consecutive native
point events had median intervals of `1.699` and `1.713` milliseconds. The
median raw-input-to-native-boundary delay was `1.042` and `0.978` milliseconds;
the first drawing-surface snapshot deliberately adds one larger instrumentation
outlier.

Live drawing queued 150 updates for 153 points and 366 updates for 374 points.
These are calls to `prepare_and_queue_epaper_update`, not claims that the panel
completed a physical refresh at the same cadence. Pen-up first emitted ten
triangles for the missing end cap and queued one final primary-surface update.
Subsequent triangle batches targeted separate stride-810 images, confirming a
second cache-render phase distinct from the latency-sensitive stride-1632
primary rendering.

The update rectangle itself is the minimal nonzero-coverage pixel rectangle.
For the first captured segment, center points near `(723, 361)`, width
`19.60505`, and the pixel-center rule `ceil(edge - 0.5)` reproduce the native
20-by-19 rectangle exactly. The Rust display path now uses the same rule rather
than expanding each segment by a heuristic margin.

## Eraser pipeline

The ordinary eraser is a scene edit, not a destructive paint operation on the
framebuffer. The input path first preprocesses the eraser line at `0x00df76f0`
with a `0.5` simplification parameter. It rejects paths over 30,000 points. The
scene mutation starts at `0x00e5baf0` and the authoritative search-and-edit
operation is `0x00dfc710`.

`0x00df8e10` traverses the target layer and groups hits by scene node. For a
line item, `0x00e4e090` performs the following stages:

1. reject disjoint line and eraser bounds;
2. find source-segment intersections with the polygonal eraser mask using a
   spatially bucketed edge index;
3. classify source points with a point-in-polygon test and insert interpolated
   boundary points at crossings;
4. collect the intervals outside the mask;
5. reconstruct zero or more line sections, interpolating packed point
   attributes at new endpoints.

The intersection expansion accounts for the source stroke's half-width and the
angle between the source and eraser edges, so testing only centerline distance
is not equivalent. A fully covered line becomes a deletion; a partial hit
replaces it with the surviving sections. All replacements are grouped into one
macro action for undo. `0x00debd10` then invalidates and rebuilds the affected
render-cache items; it is not the source-of-truth erasure.

## Two-finger zoom

### Palm rejection

All native drag recognizers first call the common filter at `0x00795a10`. It
rejects invalid timing, more contacts than the recognizer permits, and any
touch record satisfying

`contact_width contact_height > palmAreaThreshold`.

The drag-filter constructor at `0x00776ab0` initializes
`palmAreaThreshold = 900`. The Qt metacall at `0x00785a00` independently
confirms the property name and its storage at offset `0x10`. The accepted-drag
update at `0x00797f70` repeats the same test, so a palm arriving after a valid
pinch cancels the sequence instead of becoming a translation.

The Paper Pro Elan device exposes `ABS_MT_TOUCH_MAJOR` and
`ABS_MT_TOOL_TYPE`; the kernel uses tool type `2` for a palm. Because the
direct evdev interface exposes no minor diameter, Remarque estimates the
ellipse as `major * major`, applies the native 900-unit area boundary, rejects
the kernel's explicit palm classification, and keeps the sequence rejected
until every contact is released. Historical controlled captures separate the
classes cleanly: intended pinch contacts were 8--12 units wide, while palm
contacts were commonly 46--63 units wide or explicitly classified as palms.

The touch recognizer computes the centroid of every touch frame as

`c = (1 / n) sum_i p_i`.

For a candidate scale gesture it also computes the mean radial distance

`r = (1 / n) sum_i ||p_i - c||`.

`0x00791d40` reports `scale = r_current / r_initial` and a pixel displacement
`2 (r_current - r_initial)`. With two fingers, the scale is exactly the ratio
of their current and initial separation. The recognizer compares the two
finger-motion directions: a difference within 20 degrees of parallel is a pan;
the other branch is a scale candidate. The embedded `SceneViewGestures.qml`
configures both `panMinVelocity` and `scaleMinVelocity` to
`2.0 * DeviceScreenInfo.pixelsPerCm`. The constructor defaults both required
durations to 80 ms. The corresponding velocity must remain above its threshold
for that duration before `0x00796c30` accepts a scale gesture, changes its state
to 4, and emits the scale signals. Acceptance retains only the latest touch
frame, so delayed activation does not replay the pre-acceptance scale as a
jump. Native Xochitl therefore has no fixed pinch-distance threshold: the
perceived barrier is velocity plus time and direction classification.

`SceneTileManager::zoomOnPoint(viewPoint, factor)` is dispatched through the
Qt metacall at `0x007f3770` and implemented by `0x00dbaf00`. Let `q` be the
view-space anchor, `p = viewToScene(q)`, `z` the current scale, and `V` the
viewport size. It applies the multiplicative update

`z' = z factor`

and chooses the new scene-space focal point

`c' = p + (V / 2 - q) / z'`.

Thus the scene point under the fingers remains under the same view pixel. The
common focal-point setter (`0x00dbaab0`) rejects non-finite values, treats scales
below `1e-6` as invalid, snaps the focal point to the view-pixel grid with
`trunc(c' z') / z'`, and clamps it against the scene bounds before invalidating
tiles and emitting the transform change.

Parallel two-finger motion takes the pan branch. Its transform setter at
`0x00dbb060` subtracts the view displacement divided by the active scale from
the scene focal point. Combining scale and pan therefore maps the scene point
under the previous touch centroid to the current centroid. During an active
gesture, `ScreenDriver.gestureMode` selects the animation screen mode. Accepted
transform changes invalidate tiles and emit `transformChanged`; the scene-view
pipeline repaints dirty regions rather than requesting the whole screen.

`endDragAndZoom()` does not request an unconditional repaint or complete
refresh. It cancels obsolete tile jobs, requests only missing tiles, and clears
the gesture state. Leaving animation mode hides the scrollbars. A gesture whose
clamped transform never changes therefore creates neither transform damage nor
missing-tile work in this layer. Fast intermediate updates still need a final
quality update over their accumulated dirty region, even when the last rendered
pixels already equal the final image.

## Controlled drawing validation

A controlled capture on 2026-08-03 used the same `3.27.3.0` Xochitl binary as
the static analysis; the running and local binaries both had SHA-256
`9749880daa2f10844e77b560ec0ecddd1634d43eb328af637c7026edf3ef120e`.
The raw evdev streams and resulting page are preserved under
`.build/reverse/3.27.3.0/validation-20260803/`.

Two nearly equal horizontal paths were drawn with fineliner v2. The slow path
took 5.210 seconds for 3,996.5 marker units and the fast path took 1.234 seconds
for 3,999.2 units, a 4.23-times input-speed ratio. After erasing once across
their middles, the saved v6 scene contained four `FINELINER_2` line items: two
surviving sections per original line. Every stored point had width `8`, exactly
the predicted two-pixel minimum in quarter-pixel units. The mean stored speed
was about 2.52 for the slow line and 9.60 for the fast line, while pressure
varied independently. This validates both constant fineliner width and the
speed-like point field.

The two erased gaps measured 58.919 and 59.923 scene units. Their new boundary
points retained interpolated speed, direction, width, and pressure values.
This directly validates the scene-section replacement model rather than pixel
erasure.

The outward two-finger gesture changed raw separation from 448.135 to 1,040.771,
a ratio of 2.32245. The inward gesture changed it from 1,050.521 to 229.706, a
ratio of 0.218659. These are the quantities computed by the statically recovered
mean-radius ratio. The fingers' centroid moved during both gestures, exercising
the anchored focal-point correction. After returning to fit view, document
state recorded `zoomMode = bestFit` and `customZoomScale = 1`.

A second capture is preserved under
`.build/reverse/3.27.3.0/validation-medium-colors-20260803/`. Every intended
medium-fineline point had stored width `16`, confirming the statically derived
four-pixel preset. Light and strong black strokes had the same width while
their mean stored pressures were approximately 47 and 229. Saved line color
IDs matched black `0`, gray `1`, yellow `13`, green `10`, blue `6`, red `7`,
cyan `11`, and magenta `12`. No pink entry was present in the UI used for the
capture. Thick width and the exact RGB palette above come directly from the
decompiled program and require no additional gesture capture.

### Live fineliner conversion capture

A temporary ptrace breakpoint at `0x0084424c`, immediately after the call to
`quantize_pen_sample_to_line_point`, captured 24 consecutive medium-fineline
inputs and packed outputs without changing Xochitl or the document. The
effective scene-to-view scale was `0.75`. Every field is reproduced exactly by
the Rust fixture `fineliner-medium-samples-3.27.3.0.json`.

This experiment corrected two static-analysis ambiguities. AArch64 `fcvtau`
rounds distance, direction, width, and pressure to nearest rather than
truncating. More importantly, the distance multiplier at `0x01b1cbf0` is
initialized to `1.0` in the ELF but held `2.5` in the running process. Thus the
stored distance field for uniform view scale `z` is
`round(5 z (d_current + d_previous))`.

## Next experiments

1. Capture controlled white-to-black, black-to-white, and gray transitions to
   map the three-bit drive codes and temporal sequences.
2. Resolve the four-row DRM offset with changes at known top and bottom screen
   coordinates.
3. Test whether the four extra rows are padding or are used by another
   update mode.
4. Map output levels 0 through 7 to the 32 waveform tone states and reproduce
   the remaining output-level-to-transition-index step.
5. Extract the registered Qt resource tree to recover QML under stable `qrc:`
   names.
