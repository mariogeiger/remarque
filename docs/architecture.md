# Architecture

Remarque separates product behavior, native observation, conformance, and
tablet integration. Native evidence can strengthen the application without
constraining what the application may become.

```text
Product intent                    Native behavior worth preserving
      |                                      |
      |                           Xochitl + controlled captures
      |                                      |
      |                        reverse-engineering/native-replay
      |                                      |
      +------------------+-------------------+
                         v
                      app/core
                  portable contracts
                         |        app/document
                         |     mailbox + PDF export
                         v              |
                     app/tablet <-------+
       interaction, PDF rendering, and hardware adapters
                         ^
                         |
                  app/telegram-bot
                private PDF transport
```

## Workspace boundaries

| Area | Owns | Must not own |
| --- | --- | --- |
| `app/core` | Colors, strokes, geometry, raster images, rendering, erasure, transforms | evdev, DRM, Quill, systemd, application UI |
| `app/document` | Durable library protocol, content IDs, flattened PDF writing | Telegram, PDF rendering, UI, hardware |
| `app/tablet` | Hardware adapters, PDFium rendering, notebook interaction, presentation, in-process screen streaming, UI takeover | Decompiled source or capture logic |
| `app/telegram-bot` | One-chat Telegram transport, private credentials, service activation | Drawing, rendering, graphical UI |
| `app/deploy` | Explicit service definitions for the tablet | Hidden installation logic |
| `reverse-engineering/native-observer` | Firmware-specific runtime capture probes | Product behavior or screen serving |
| `reverse-engineering/native-replay` | Immutable native fixtures and comparisons against core behavior | Tablet mutation or product state |
| `reverse-engineering/ghidra` | Evidence-backed semantic names and local readable exports | Production implementation |

## Core data direction

`Color` is a scene property, not a brush property. `Stroke` owns points and a
color. A brush converts `PenSample` values into shared `StrokePoint` values.
Renderers consume those shared values and a color; they do not own either.
`BgraImage` owns pixels and general drawing primitives independently of any
brush renderer.

This direction lets another brush reuse colors, stored strokes, images,
transforms, erasure, and display integration without depending on fineliner
internals.

## Document data direction

The persistent model is `Document { pages, current_page }` and
`Page { optional_background, strokes }`. A blank notebook and an imported PDF
therefore use identical navigation, insertion, drawing, erasure, and export
operations. An incoming PDF remains immutable; its pages are optional scene
backgrounds beneath separately persisted strokes. Zoom changes only the view
transform, while strokes stay in scene coordinates.

The library itself has no active-document field. It is the home state, while an
opened document is an ephemeral tablet view. Returning home or restarting the
application cannot implicitly reopen a file.

Current-page export flattens one page without the toolbar. Whole-document
export performs that operation page by page and writes a multi-page PDF while
holding only one rasterized page in memory. Distinct source page dimensions
are preserved.

Each PDF page is a separate scene. At minimum zoom its PDF width equals the
screen width; tall pages pan vertically, while a short or landscape page leaves
the out-of-page area gray. Two fingers pan and zoom within the page. A
one-finger inward swipe from the left or right edge changes page.

Stationary one-finger taps, edge swipes, and two-finger pinches are mutually
exclusive states in one gesture recognizer behind the palm-contact filter.
Tool choice does not add UI state: the stylus tip is the fineliner and the
opposite end is the eraser.

The physical power button is another tablet input, not an application exit.
The tablet adapter persists any active stroke, enters the firmware's systemd
suspend-then-hibernate path, and redraws the same runtime view after wake. A
timed transition wake lock closes the gap between releasing the application
lock and systemd disabling autosleep. The kernel's successful-suspend counter,
not the command exit alone, determines whether a retry is needed. Firmware
sleep hooks and wake-source configuration remain the authority for hardware
power transitions.

Battery and Wi-Fi readers expose small hardware facts to the library
presentation without putting sysfs or `wpa_cli` calls in drawing code. Sleep
measurement keeps raw before/after charge readings, sleep-inclusive monotonic
time, and kernel suspend counters; interpretation remains separate so an
overnight discharge is not confused with a charging cycle or a failed suspend.

The tablet library is available directly on screen. The Telegram surface keeps
only two commands: `/library` opens a document remotely and `/export` chooses a
current-page or whole-document export. Sending a PDF imports and opens it.

The Telegram daemon and graphical process have no shared memory or lifecycle.
They exchange atomically renamed JSON requests and responses through the same
durable-file primitive used by state, downloads, and PDF export. A request
remains durable until its response is durable, and the bot advances its
Telegram offset only after both the tablet effect and its reply succeed.

## Behavior rules

A native-parity claim requires a native observation, a readable explanation, a
small mathematical specification, and a regression test. An original Remarque
feature instead requires an explicit product contract and tests; it does not
need a native precedent. Hardware policy that exists only because of the Paper
Pro remains in `app/tablet`.
