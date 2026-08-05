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
                    ^             |
                    |        app/page-log
             app/telegram-bot     |
        private control plane     v
                            app/page-relay
                         durable page authority
                                  |
                     app/browser-page-renderer
                       Rust/Wasm canvas replica
```

## Workspace boundaries

| Area | Owns | Must not own |
| --- | --- | --- |
| `app/core` | Colors, strokes, geometry, raster images, rendering, erasure, transforms | evdev, DRM, e-paper hardware, systemd, application UI |
| `app/document` | Durable library protocol, content IDs, flattened PDF writing | Telegram, PDF rendering, UI, hardware |
| `app/tablet` | Hardware adapters, PDFium rendering, notebook interaction, presentation, in-process screen streaming, UI takeover | Decompiled source or capture logic |
| `app/telegram-bot` | One-chat Telegram transport, private credentials, service activation | Drawing, rendering, graphical UI |
| `app/page-log` | Ordered page operations, permissions, snapshots, binary protocol | Network transport, UI, persistence paths |
| `app/page-relay` | Authenticated 24-hour shares, durable authority, WebSocket fanout, background assets | Drawing input or canvas rendering |
| `app/browser-page-renderer` | Browser-side Rust/Wasm page replica and rasterization | Share authority or secret persistence |
| `app/deploy` | Explicit service definitions for the tablet and relay | Hidden installation logic |
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

The tablet library is available directly on screen. Telegram opens and exports
documents and acts as the sharing control plane through `/share`, `/shares`,
and `/revoke`. Sending a PDF imports and opens it.

A shared page is identified independently of the tablet's current view. The
relay is its ordered, durable authority for 24 hours, so browser participants
continue drawing while the tablet is offline. Clients exchange stroke points
and erasure replacements, not framebuffer tiles; each replica rasterizes the
same core stroke model. The relay assigns every guest a non-black color and
materializes that identity into operations. Guests may replace only their own
strokes; the black owner may replace any stroke.

The capability secret is delivered only in the URL fragment, redeemed once,
then removed from browser history and replaced by a Secure, HttpOnly,
SameSite cookie. The public origin is `https://remarque.geiger.ink`. Periodic
snapshot digests detect divergence, while snapshots establish and repair a
replica without making full-frame transfer the steady-state protocol.

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
