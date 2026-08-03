# Remarque document exchange

This crate owns the device-independent document boundary: durable requests
between the Telegram service and the graphical process, plus flattened PDF
export. It does not know Telegram, systemd, Quill, or PDFium.

Incoming PDFs remain immutable. The tablet process renders their pages as
optional backgrounds and stores Remarque strokes separately. Export flattens
either one page or an entire mixed document into a new PDF.

Requests and responses are atomically renamed into a mailbox directory. A
request is removed only after its response is durable, so restarting either
process cannot silently lose an operation.

Export is independent of background source: the same operation flattens
strokes over white pages and rendered PDF pages. Whole-document writing
generates and compresses one raster page at a time, so memory use does not grow
with the document's total raster size.
