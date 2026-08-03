# Scene/view transformation

Native evidence: Xochitl 3.27.3.0 functions `0x00dba970` through
`0x00dbb060`, touch functions `0x00791bf0` through `0x00791d40`, and the
controlled pinch capture described in `../../../reverse-engineering/xochitl.md`.

For viewport size `V`, scene focal point `c`, scale `z`, view point `q`, and
scene point `p`:

```text
p = c + (q - V/2) / z
q = (p - c) z + V/2
```

A pinch from previous centroid `q0` to current centroid `q1` with separation
ratio `r` uses `z' = z r`, preserves `p = view_to_scene(q0)`, and chooses:

```text
c' = p + (V/2 - q1) / z'
```

The focal coordinates are truncated to the device-pixel grid and clamped so
the viewport remains within scene bounds. If the scene is shorter than the
visible interval on an axis, that axis is centered instead.

The native capture measured separation ratios `1040.771 / 448.135` and
`229.706 / 1050.521`. They are retained as regression vectors in the module.
