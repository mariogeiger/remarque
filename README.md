# Remarque

Remarque is an independent, customizable reMarkable Paper Pro application
written in Rust. It preserves the native application's best interactions where
they matter and provides a clean base for features designed specifically for
its owner.

The native application is a reference, not the product boundary. Remarque
distills useful native behavior into mathematics and fixtures without treating
decompiler output as source code, then combines those verified foundations with
its own interaction model and capabilities.

## Design principles

- **Independent product.** Native compatibility never prevents a simpler or
  more useful Remarque feature.
- **Evidence before imitation.** Every claim about native behavior is tied to a
  hashed binary, controlled capture, or explicit decompilation path.
- **Portable behavior, narrow adapters.** Drawing and geometry live in
  `app/core`; evdev, Quill, and systemd remain in `app/tablet`.
- **One source of truth per concept.** Colors, strokes, raster images, and view
  transforms are shared data, not properties of one brush or UI.
- **Differential confidence.** Recorded native fixtures test the same operations
  used by the tablet application.

Read the [project soul](SOUL.md), the [architecture](docs/architecture.md), and
the [documentation index](docs/README.md).
