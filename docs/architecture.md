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
| `app/document` | Durable document request/response protocol, flattened PDF writing | Telegram, PDF rendering, UI, hardware |
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

An incoming PDF is immutable. `remarque-tablet` rasterizes its current page as
the scene background and persists a separate Remarque stroke layer for every
page. Zoom changes only the view transform; strokes stay in scene coordinates.
`/page` renders the untransformed scene without the toolbar and flattens it into
a new one-page PDF. `/document` returns the untouched source.

Each PDF page is a separate scene. At minimum zoom its PDF width equals the
screen width; tall pages pan vertically, while a short or landscape page leaves
the out-of-page area gray. Two fingers pan and zoom within the page. A
one-finger inward swipe from the left or right edge changes page.

The Telegram daemon and graphical process have no shared memory or lifecycle.
They exchange atomically renamed JSON requests and responses. A request remains
durable until its response is durable, and the bot advances its Telegram offset
only after both the tablet effect and its reply succeed.

## Behavior rules

A native-parity claim requires a native observation, a readable explanation, a
small mathematical specification, and a regression test. An original Remarque
feature instead requires an explicit product contract and tests; it does not
need a native precedent. Hardware policy that exists only because of the Paper
Pro remains in `app/tablet`.
