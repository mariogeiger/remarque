# Remarque core

This crate contains Remarque's device-independent behavior. Some contracts
preserve useful Xochitl behavior; others are original product behavior. It is
clean Rust, never a translation of decompiler output.

## Evidence routes

Native-parity behavior enters this crate only when four layers agree:

1. the hashed native binary and captured inputs provide the primary evidence;
2. the semantic decompilation explains the relevant control flow and data;
3. a compact specification states the mathematics, invariants, and formats;
4. Rust tests reproduce native vectors and check general properties.

Original Remarque behavior instead requires a compact contract, explicit
invariants, and property-focused tests. It does not need a native precedent.

For parity claims, the binary and its observed outputs remain the oracle.
Decompiler output is only a fallible view of that oracle.

## Module order

1. `color`: the native scene palette and stable scene identifiers, independent
   of any brush or renderer.
2. `stroke`: shared pen samples, packed stroke points, and colored strokes.
3. `bgra_image`: owned pixel storage, rectangle composition, alpha blending,
   and brush-independent image primitives.
4. `view_transform`: coordinate mapping, anchored pinch scaling, translation,
   scene clamping, and viewport position indicators. Its equations and native
   vectors are recorded in `docs/view-transform.md`.
5. `fineliner`: constant-width presets and conversion from pen samples to
   shared stroke points. Its recovered equations and remaining capture boundary
   are in `docs/fineliner.md`.
6. `erase_strokes`: exact capsule/segment intersections and line splitting,
   preserving interpolated point metadata at new boundaries.
7. `render_fineliner`: incremental four-corner ribbons, joins, rounded endpoint
   caps, signed-distance triangle coverage, and exact BGRA composition.

Hardware input, DRM, waveform submission, and application UI do not belong in
this crate. They consume these operations from device-specific crates.
