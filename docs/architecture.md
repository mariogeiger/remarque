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
                         |
                         v
                     app/tablet
       interaction, custom features, and hardware adapters
```

## Workspace boundaries

| Area | Owns | Must not own |
| --- | --- | --- |
| `app/core` | Colors, strokes, geometry, raster images, rendering, erasure, transforms | evdev, DRM, Quill, systemd, application UI |
| `app/tablet` | Hardware adapters, notebook interaction, presentation, in-process screen streaming, UI takeover | Decompiled source or capture logic |
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

## Behavior rules

A native-parity claim requires a native observation, a readable explanation, a
small mathematical specification, and a regression test. An original Remarque
feature instead requires an explicit product contract and tests; it does not
need a native precedent. Hardware policy that exists only because of the Paper
Pro remains in `app/tablet`.
