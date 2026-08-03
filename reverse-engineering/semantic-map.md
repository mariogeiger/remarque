# Semantic map of Xochitl 3.27.3.0

The addresses below refer only to the verified Paper Pro binary with SHA-256
`9749880daa2f10844e77b560ec0ecddd1634d43eb328af637c7026edf3ef120e`.
Names describe operations established from control flow, Qt metadata, embedded
strings, or controlled device tests. They are not claimed original symbols.

## Recovered flows

```text
touch frames
  -> reject_touch_sequence_exceeding_common_limits
  -> accept_touch_scale_gesture
  -> update_accepted_touch_drag
  -> compute_touch_frame_centroid
  -> compute_touch_separation_scale
  -> retain_only_latest_touch_frame
  -> scale_scene_about_view_point / pan_scene_by_view_delta
  -> compute_pixel_aligned_viewport_origin
  -> build_view_to_scene_transform / build_scene_to_view_transform
  -> set_scene_focal_point_and_scale

raw pen samples
  -> begin_pen_line
  -> append_pen_sample_to_active_line
  -> quantize_pen_sample_to_line_point
  -> render_live_pen_line
  -> finish_pen_line | cancel_pen_line

eraser samples
  -> simplify_eraser_path
  -> apply_eraser_path_to_scene
  -> find_and_replace_erased_scene_items
  -> group_eraser_hits_by_scene_node
  -> split_line_outside_eraser_mask
  -> rebuild_render_cache_for_edited_items

BGRA update
  -> prepare_and_queue_epaper_update
  -> initialize_epaper_update_record / copy_epaper_update_record
  -> clip to the dirty rectangle
  -> split sufficiently tall rectangles into three conversion jobs
  -> convert_bgra_update_to_tone_levels
  -> subtract_update_rectangle_from_pending_updates
  -> quantize_bgra_pixels_with_tetrahedral_thresholds
  -> dispatch_waveform_frame_generation
  -> pack_eight_waveform_drive_codes
  -> DRM framebuffer flip
```

The common touch filter runs before gesture acceptance and again while an
accepted drag is updating. It rejects an entire sequence for too many contacts
or whenever any contact ellipse has area greater than 900 square pixels. This
is the native palm-rejection boundary, not a heuristic inferred from Remarque.

The display path is region-based: it allocates and converts only the clipped
dirty rectangle. Rectangles at least 29 rows tall are divided into three worker
jobs before waveform selection and queueing. This is separate from waveform
quality; a gesture can combine a rectangular update with a fast monochrome
mode, then request complete color once interaction ends.

Before queueing a new rectangle, Xochitl removes its area from pending updates.
An intersecting old rectangle is split into at most four remainders. Newer
pixels therefore supersede queued work spatially instead of waiting behind a
redundant full-screen refresh.

## Scene transform layout

The common state pointer used by the focal-point, scale, and pan operations has
this proven 64-byte prefix:

| Offset | Type | Field |
| ---: | --- | --- |
| `0x00` | `int32` | viewport width |
| `0x04` | `int32` | viewport height |
| `0x08` | `double` | focal x |
| `0x10` | `double` | focal y |
| `0x18` | `double` | scale |
| `0x20` | `double` | scene origin x |
| `0x28` | `double` | scene origin y |
| `0x30` | `double` | scene width |
| `0x38` | `double` | scene height |

Applying this structure changes raw expressions such as
`*(double *)(param + 0x18)` into `transform->scale` throughout the decompiler.
`PointF` captures the two-register `(x, y)` result shared by touch centroids and
the pixel-aligned viewport origin.

`AffineTransform` captures the 80-byte Qt matrix representation. The recovered
pair is exactly inverse: `scene = view / scale + origin` and
`view = (scene - origin) * scale`. Both use the same pixel-aligned origin.

## Line data layouts

`RawPenSample` is five consecutive floats: x, y, pressure, tilt x, and tilt y.
`PackedLinePoint` is the exact 14-byte representation documented in
`xochitl.md`. Both are installed in Ghidra's type
database by the reusable export pass.

## Confidence policy

- **High**: the operation follows directly from control flow and constants, or
  matches a controlled runtime artifact.
- **Medium**: the operation is supported by call structure and metadata but a
  field or branch remains unresolved.
- Uncertain candidates stay unnamed. A plausible but wrong symbol damages all
  downstream decompilation more than an honest address does.
