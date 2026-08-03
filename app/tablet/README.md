# Remarque tablet application

This crate runs the reconstructed notebook directly on a reMarkable Paper Pro
and returns cleanly to Xochitl through an explicit Quit action.

## Boundaries

- `input` converts Linux marker and touch events into typed frames.
- `filter_touch_sequences` rejects palm and excess-contact sequences.
- `notebook` owns strokes, tool settings, erasure, and view-transform state.
- `pdfium` renders immutable PDF pages into notebook backgrounds.
- `page` represents geometry, strokes, and an optional PDF background.
- `document_requests` applies durable open, inspect, and export requests.
- `edge_page_swipe` recognizes deliberate one-finger page turns from the left
  or right screen edge.
- `draw_toolbar` and `draw_viewport_indicators` own presentation only.
- `display` copies BGRA rectangles and requests Quill waveform updates.
- `screen_stream` serves snapshots of that same display from this process; it
  is not a second tablet application.
- `remarque_tablet` only wires signals, devices, and the application loop.

Portable colors, strokes, images, geometry, erasure, and fineliner rendering
come from `remarque-core`; mailbox and PDF-export contracts come from
`remarque-document`. Native capture and decompiler code never enters this crate.

The document-close icon returns to a separately persisted blank page without
deleting the PDF or its annotations. Drawing, erasure, transforms, persistence,
and page export use the same path for blank and PDF-backed pages.

The stream listens on port `7432` without authentication. Expose it only on a
trusted local network or through a private network transport.

## Build

The `takeover` feature links against the Paper Pro Quill, PDFium, and e-paper
libraries.
Use the firmware-matched SDK and `app/scripts/link-with-remarkable-sdk`; the
host workspace tests exercise the portable modules without linking those
libraries.

The service definitions under `../deploy/` make the transition mutually
exclusive with Xochitl. The native-tile launcher clears only Xochitl's stale
`LastOpen` recovery marker after Xochitl has stopped.
