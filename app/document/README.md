# Remarque document exchange

This crate owns the device-independent document boundary: durable requests
between the Telegram service and the graphical process, plus flattened PDF
export. It does not know Telegram, systemd, Quill, or PDFium.

Incoming PDFs remain immutable. The tablet process renders one as a background
and stores Remarque strokes separately. Export deliberately flattens a page;
the original document is always available unchanged.

Requests and responses are atomically renamed into a mailbox directory. A
request is removed only after its response is durable, so restarting either
process cannot silently lose an operation.

Page export is independent of its background source: the same operation
flattens strokes over either a white page or a rendered PDF page.
