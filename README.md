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
  `app/core`; document exchange and PDF export live in `app/document`; evdev,
  PDFium, the Paper Pro e-paper ABI, and systemd remain behind application
  adapters.
- **One source of truth per concept.** Colors, strokes, raster images, and view
  transforms are shared data, not properties of one brush or UI.
- **Differential confidence.** Recorded native fixtures test the same operations
  used by the tablet application.

Read the [project soul](SOUL.md), the [architecture](docs/architecture.md), and
the [documentation index](docs/README.md).

The private Telegram service can import a PDF, open any stored document, and
export either the current page or every annotated page. Its bot credential is
device configuration and never belongs in this repository.
