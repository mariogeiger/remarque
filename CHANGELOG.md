# Changelog

All notable user-visible changes to this project are documented here.

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
