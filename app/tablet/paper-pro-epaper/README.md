# Paper Pro e-paper boundary

This directory owns the narrow C++ ABI boundary between Remarque and the
Paper Pro firmware's Qt-based e-paper engine. It exposes one framebuffer view,
regional update submission, queue synchronization for diagnostics, and Qt
event delivery through a stable C ABI. Display policy, dirty-region tracking,
waveform selection, and pixel access remain in Rust.

The implementation loads `libqsgepaper.so` explicitly at runtime from the
application's configured library paths. The proprietary library is neither
copied into this repository nor required as a build input. Compilation still
uses Qt headers and libraries from the firmware-matched developer SDK.

The source is adapted from the MIT-licensed Quill clean-room implementation.
Its exact origin and integration changes are recorded in `PROVENANCE.md`;
Quill's license is preserved in `LICENSE`.
