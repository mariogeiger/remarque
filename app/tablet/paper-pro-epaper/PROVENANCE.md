# E-paper boundary provenance

The implementation is adapted from public Quill release `v0.1.0` at commit
`39262ee`, plus the display-timing changes preserved in
`../../../reverse-engineering/display-response/experiments/2026-08-04/quill-display-timing.patch.zst`.
The imported production boundary comprised Quill's `vendor_probe.cpp`,
`vendor_probe.h`, `quill_c.cpp`, and `clip_rect.h`.

Quill records that those files were implemented from a behavioral and ABI
specification without inspecting a former adapter implementation, `epfb-re`
source, or disassembly derived from `epfb-re`. Permitted references were Qt 6
headers from the matching reMarkable SDK, ELF metadata, dynamic-linking
behavior, vendor ABI symbol names, and black-box behavior of the owner's Paper
Pro. The proprietary `libqsgepaper.so` was not redistributed.

Remarque preserves that boundary while:

- reducing it to framebuffer discovery, regional update submission, queue
  synchronization, and Qt event delivery;
- replacing the Quill-specific C API with a product-owned stable C ABI;
- compiling the C++ source directly into the tablet executable; and
- loading the firmware's own `libqsgepaper.so` explicitly at runtime.

All production display policy, waveform selection, dirty-region tracking,
pixel copying, and update scheduling remain in Rust. The C++ boundary exposes
a blocking vendor-queue wait only to device diagnostics. No decompiled vendor
implementation is present in this directory. This record documents
implementation provenance; it is not legal advice about the proprietary
firmware library.
