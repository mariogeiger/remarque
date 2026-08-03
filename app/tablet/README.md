# Remarque tablet application

This crate runs the reconstructed notebook directly on a reMarkable Paper Pro
and returns cleanly to Xochitl through the library's explicit Quit action.

## Boundaries

- `input` converts Linux marker and touch events into typed frames.
- `filter_touch_sequences` rejects palm and excess-contact sequences.
- `touch_tap` recognizes stationary finger taps on the library and toolbar.
- `touch_gesture` makes taps, page swipes, and pinches mutually exclusive.
- `document_library` owns documents, ordered pages, selection, and persistence.
- `notebook` owns the active interaction, tools, erasure, and view transform.
- `pdfium` renders immutable PDF pages into notebook backgrounds.
- `page` represents geometry, strokes, and an optional PDF background.
- `render_page_view` maps page backgrounds and strokes through the view
  transform.
- `document_requests` applies durable import, list, open, and export requests.
- `edge_page_swipe` recognizes deliberate one-finger page turns from the left
  or right screen edge.
- `draw_document_library`, `draw_text`, `draw_toolbar`, and
  `draw_viewport_indicators` own presentation only.
- `display` copies BGRA rectangles and requests Quill waveform updates.
- `screen_stream` serves snapshots of that same display from this process; it
  is not a second tablet application.
- `remarque_tablet` only wires signals, devices, and the application loop.

Portable colors, strokes, images, geometry, erasure, and fineliner rendering
come from `remarque-core`; mailbox and PDF-export contracts come from
`remarque-document`. Native capture and decompiler code never enters this crate.

The library contains both imported PDFs and blank notebooks. Each document is
an ordered list of the same page type: a page has an optional immutable PDF
background and editable strokes. The toolbar can insert a blank page after the
current page in either kind of document.

The library is the home screen. Opening a document creates only a runtime view;
returning home drops that view, and startup never restores or highlights a
last-opened document. Per-document pages and annotations remain persistent.

Both the library and toolbar accept the stylus or a finger. Finger taps pass
through palm rejection and are cancelled when they turn into a drag or a
two-finger gesture. The stylus tip always draws; its opposite end always erases.

The stream listens on port `7432` without authentication. Expose it only on a
trusted local network or through a private network transport.

## Build

The `takeover` feature links against the Paper Pro Quill, PDFium, and e-paper
libraries and embeds a TrueType UI font. Set `REMARQUE_UI_FONT` when Noto Sans
or DejaVu Sans is not installed on the build host.
Use the firmware-matched SDK and `app/scripts/link-with-remarkable-sdk`; the
host workspace tests exercise the portable modules without linking those
libraries.

The service definitions under `../deploy/` make the transition mutually
exclusive with Xochitl. The native-tile launcher clears only Xochitl's stale
`LastOpen` recovery marker after Xochitl has stopped.
