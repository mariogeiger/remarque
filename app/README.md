# Remarque application

This directory is the product. It contains the portable behavior, the tablet
process, its presentation assets, deployment units, and build adapters.

- `core/` defines device-independent drawing, geometry, image, erasure, and
  view-transform contracts.
- `document/` defines durable library requests, content IDs, and flattened
  single- or multi-page PDF export.
- `tablet/` adapts those contracts to Paper Pro input, display, interaction,
  PDFium rendering, and in-process screen streaming.
- `telegram-bot/` is the headless, single-chat library transport.
- `deploy/` contains explicit systemd units for switching between Remarque and
  Xochitl.
- `scripts/` contains target-linking adapters used to build the application.

Nothing under `app/` reads Xochitl memory or contains decompiler output. Native
evidence and capture programs live under `../reverse-engineering/`.
