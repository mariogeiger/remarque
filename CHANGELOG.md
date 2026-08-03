# Changelog

All notable user-visible changes to this project are documented here.

## 0.4.0 - 2026-08-03

### Added

- A toolbar action and `/close` command that close the active PDF without
  deleting it, then return to the persistent blank page.
- `/page` export for both blank pages and PDF-backed pages.

### Changed

- Blank and PDF-backed pages now share one page model. A PDF is an optional
  immutable background beneath the same strokes, dimensions, transform, and
  export path.
- Reopening the last PDF restores its page number and annotations, while the
  independent blank page is restored when the PDF closes again.

## 0.3.0 - 2026-08-03

### Added

- A private, single-chat Telegram service that receives PDFs and can return the
  original document, current annotated page, or application status.
- Telegram page navigation with independently persisted annotations per page.
- One-finger edge swipes for discrete page turns, with the palm filter shared
  by page navigation and two-finger zoom.
- PDFium-backed multi-page display with an immutable source background and
  separately persisted Remarque strokes.
- Durable, atomic request/response exchange between the always-on transport and
  graphical tablet process.
- A device-independent, flattened one-page PDF exporter.

### Changed

- Product crates affected by document support are versioned at 0.3.0.
- Starting a document operation activates Remarque without requiring SSH; the
  Telegram daemon itself never owns the display.
- Minimum PDF zoom now fits page width exactly; tall pages pan vertically and
  out-of-page space is gray.

### Security

- The bot accepts updates from one configured chat only, bounds downloads to
  50 MiB, sanitizes filenames, and requires its credential file to be mode
  `0600`.
- Telegram update offsets and local operations become durable only after their
  effects succeed, avoiding silent loss across restarts.

## 0.2.0 - 2026-08-03

### Added

- A Rust tablet application with fineliner drawing, stroke erasing, anchored
  two-finger zoom and translation, viewport indicators, and a native-home tile.
- Palm rejection reconstructed from Xochitl's contact-area threshold and the
  Paper Pro touch controller's palm classification.
- A device-independent core, native replay fixtures, synchronized capture
  probes, and a readable semantic decompilation workflow.
- A floating color toolbar, explicit Quit control, antialiased label, eraser
  impact preview, and rounded fineliner endpoints.
- Browser streaming of Remarque's rendered screen from the tablet process.

### Changed

- Shared stroke colors, stroke data, BGRA images, rendering, notebook state,
  toolbar drawing, and hardware adapters now have separate modules.
- All workspace crates are versioned at 0.2.0 after the public core API cleanup.
- Repository documentation now describes the reconstruction system rather than
  only the original framebuffer streamer.
- The project mission now treats native behavior as a verified foundation for
  an independent, customizable application rather than as its ceiling.
- Product code now lives under `app/`; capture probes, replay fixtures, Ghidra
  tooling, and native findings live under `reverse-engineering/`.
- Public-tree safeguards now exclude private captures, environment files,
  Tailscale state, and generated decompiler output; connection examples use
  placeholders.

### Fixed

- Palm contacts no longer become accidental zoom or translation gestures.
- Returning to Xochitl no longer reports the native launcher tile as an
  unexpectedly closed document.
