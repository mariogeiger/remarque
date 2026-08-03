use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

#[derive(Deserialize)]
struct TraceEvent {
    monotonic_ns: u64,
    kind: String,
    point_count: Option<u64>,
    scene_to_view_scale: Option<f64>,
    render_point: Option<RenderPoint>,
    native_point: Option<NativePoint>,
    image: Option<String>,
    stride: Option<usize>,
}

#[derive(Deserialize)]
struct RenderPoint {
    width: f64,
}

#[derive(Deserialize)]
struct NativePoint {
    width_quarters: u16,
}

#[derive(Deserialize)]
struct RawPenEvent {
    kernel_seconds: u64,
    kernel_microseconds: u64,
    #[serde(rename = "type")]
    event_type: u16,
    code: u16,
    value: i32,
}

#[derive(Deserialize)]
struct DrawingSurfaceMetadata {
    stride_pixels: usize,
    width: usize,
    height: usize,
}

#[derive(Default)]
struct CollectedStroke {
    begin_ns: u64,
    finish_ns: Option<u64>,
    last_event_ns: u64,
    native_point_count: Option<u64>,
    line_point_times: Vec<u64>,
    native_input_times: Vec<u64>,
    live_triangles: usize,
    finalization_triangles: usize,
    live_display_updates: usize,
    finalization_display_updates: usize,
    finish_ribbons: usize,
    scales: Vec<f64>,
    render_widths: Vec<f64>,
    stored_widths: Vec<f64>,
}

#[derive(Serialize)]
struct TraceSummary {
    event_counts: BTreeMap<String, usize>,
    event_count: usize,
    raw_pen_event_count: usize,
    display_update_intervals: IntervalSummary,
    triangle_destinations: Vec<TriangleDestinationSummary>,
    strokes: Vec<StrokeSummary>,
    drawing_difference: DrawingDifference,
}

#[derive(Serialize)]
struct StrokeSummary {
    duration_ms: f64,
    finalization_duration_ms: f64,
    native_point_count: u64,
    recorded_line_point_events: usize,
    raw_touch_frames: usize,
    input_to_native_delay: DistributionSummary,
    line_point_intervals: IntervalSummary,
    live_triangle_calls: usize,
    finalization_triangle_calls: usize,
    live_display_updates: usize,
    finalization_display_updates: usize,
    finish_ribbon_calls: usize,
    scene_to_view_scale: RangeSummary,
    render_width_pixels: RangeSummary,
    stored_width_quarters: RangeSummary,
}

#[derive(Serialize)]
struct TriangleDestinationSummary {
    image: String,
    stride_pixels: usize,
    call_count: usize,
}

#[derive(Serialize)]
struct IntervalSummary {
    sample_count: usize,
    interval_count: usize,
    mean_ms: f64,
    median_ms: f64,
    percentile_95_ms: f64,
    maximum_ms: f64,
}

#[derive(Serialize)]
struct DistributionSummary {
    sample_count: usize,
    mean_ms: f64,
    median_ms: f64,
    percentile_95_ms: f64,
    maximum_ms: f64,
}

#[derive(Serialize)]
struct RangeSummary {
    minimum: f64,
    maximum: f64,
}

#[derive(Serialize)]
struct DrawingDifference {
    changed_pixels: usize,
    changed_rectangle: Option<Rectangle>,
    mean_absolute_channel_change: f64,
    maximum_absolute_channel_change: u8,
}

#[derive(Serialize)]
struct Rectangle {
    x: usize,
    y: usize,
    width: usize,
    height: usize,
}

fn main() {
    if let Err(error) = summarize_trace() {
        eprintln!("summary_error={error}");
        std::process::exit(1);
    }
}

fn summarize_trace() -> Result<(), String> {
    let (directory, output_path) = parse_arguments()?;
    let events: Vec<TraceEvent> = read_json_lines(&directory.join("events.jsonl"))?;
    let raw_events: Vec<RawPenEvent> = read_json_lines(&directory.join("raw-pen-events.jsonl"))?;
    let metadata: DrawingSurfaceMetadata = read_json(&directory.join("drawing-surface.json"))?;

    let event_counts = count_event_kinds(&events);
    let triangle_destinations = count_triangle_destinations(&events);
    let display_update_times = events
        .iter()
        .filter(|event| event.kind == "display_update")
        .map(|event| event.monotonic_ns)
        .collect::<Vec<_>>();
    let mut strokes = collect_strokes(&events);
    let raw_touch_sessions = collect_raw_touch_sessions(&raw_events);
    let stroke_summaries = strokes
        .iter_mut()
        .enumerate()
        .map(|(index, stroke)| summarize_stroke(stroke, raw_touch_sessions.get(index)))
        .collect::<Vec<_>>();
    let drawing_difference = compare_drawing_surfaces(&directory, &metadata)?;
    let summary = TraceSummary {
        event_counts,
        event_count: events.len(),
        raw_pen_event_count: raw_events.len(),
        display_update_intervals: summarize_intervals(&display_update_times),
        triangle_destinations,
        strokes: stroke_summaries,
        drawing_difference,
    };
    let json = serde_json::to_vec_pretty(&summary).map_err(|error| error.to_string())?;
    if let Some(output_path) = output_path {
        fs::write(&output_path, &json)
            .map_err(|error| format!("cannot write {}: {error}", output_path.display()))?;
    }
    println!("{}", String::from_utf8(json).unwrap());
    Ok(())
}

fn parse_arguments() -> Result<(PathBuf, Option<PathBuf>), String> {
    let mut arguments = std::env::args().skip(1);
    let directory = arguments.next().map(PathBuf::from).ok_or_else(|| {
        "usage: summarize-native-stroke-trace TRACE-DIRECTORY [SUMMARY.json]".to_owned()
    })?;
    let output_path = arguments.next().map(PathBuf::from);
    if arguments.next().is_some() {
        return Err(
            "usage: summarize-native-stroke-trace TRACE-DIRECTORY [SUMMARY.json]".to_owned(),
        );
    }
    Ok((directory, output_path))
}

fn read_json_lines<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<Vec<T>, String> {
    let file =
        fs::File::open(path).map_err(|error| format!("cannot open {}: {error}", path.display()))?;
    BufReader::new(file)
        .lines()
        .enumerate()
        .map(|(index, line)| {
            let line = line.map_err(|error| error.to_string())?;
            serde_json::from_str(&line).map_err(|error| {
                format!("invalid JSON at {}:{}: {error}", path.display(), index + 1)
            })
        })
        .collect()
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T, String> {
    let bytes =
        fs::read(path).map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    serde_json::from_slice(&bytes)
        .map_err(|error| format!("invalid JSON in {}: {error}", path.display()))
}

fn count_event_kinds(events: &[TraceEvent]) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    for event in events {
        *counts.entry(event.kind.clone()).or_default() += 1;
    }
    counts
}

fn count_triangle_destinations(events: &[TraceEvent]) -> Vec<TriangleDestinationSummary> {
    let mut counts = BTreeMap::new();
    for event in events.iter().filter(|event| event.kind == "triangle") {
        if let (Some(image), Some(stride)) = (&event.image, event.stride) {
            *counts.entry((image.clone(), stride)).or_default() += 1;
        }
    }
    counts
        .into_iter()
        .map(
            |((image, stride_pixels), call_count)| TriangleDestinationSummary {
                image,
                stride_pixels,
                call_count,
            },
        )
        .collect()
}

fn collect_strokes(events: &[TraceEvent]) -> Vec<CollectedStroke> {
    let mut strokes = Vec::new();
    let mut active = None;
    for event in events {
        if event.kind == "begin_line" {
            strokes.push(CollectedStroke {
                begin_ns: event.monotonic_ns,
                last_event_ns: event.monotonic_ns,
                native_input_times: vec![event.monotonic_ns],
                ..CollectedStroke::default()
            });
            active = Some(strokes.len() - 1);
            continue;
        }
        let Some(stroke) = active.map(|index| &mut strokes[index]) else {
            continue;
        };
        stroke.last_event_ns = event.monotonic_ns;
        let finalizing = stroke.finish_ns.is_some();
        match event.kind.as_str() {
            "line_point" => {
                stroke.line_point_times.push(event.monotonic_ns);
                stroke.native_input_times.push(event.monotonic_ns);
                if let Some(scale) = event.scene_to_view_scale {
                    stroke.scales.push(scale);
                }
                if let Some(render_point) = &event.render_point {
                    stroke.render_widths.push(render_point.width);
                }
                if let Some(native_point) = &event.native_point {
                    stroke
                        .stored_widths
                        .push(f64::from(native_point.width_quarters));
                }
            }
            "triangle" if finalizing => stroke.finalization_triangles += 1,
            "triangle" => stroke.live_triangles += 1,
            "display_update" if finalizing => stroke.finalization_display_updates += 1,
            "display_update" => stroke.live_display_updates += 1,
            "finish_ribbon" => stroke.finish_ribbons += 1,
            "finish_line" => {
                stroke.finish_ns = Some(event.monotonic_ns);
                stroke.native_point_count = event.point_count;
            }
            _ => {}
        }
    }
    strokes
}

fn collect_raw_touch_sessions(events: &[RawPenEvent]) -> Vec<Vec<u64>> {
    let mut sessions = Vec::new();
    let mut active = None;
    for event in events {
        if event.event_type == 1 && event.code == 330 {
            if event.value != 0 {
                sessions.push(Vec::new());
                active = Some(sessions.len() - 1);
            } else {
                active = None;
            }
        }
        if event.event_type == 0
            && event.code == 0
            && let Some(index) = active
        {
            sessions[index]
                .push(event.kernel_seconds * 1_000_000_000 + event.kernel_microseconds * 1_000);
        }
    }
    sessions
}

fn summarize_stroke(
    stroke: &mut CollectedStroke,
    raw_touch_times: Option<&Vec<u64>>,
) -> StrokeSummary {
    let finish_ns = stroke.finish_ns.unwrap_or(stroke.last_event_ns);
    let raw_touch_times = raw_touch_times.map(Vec::as_slice).unwrap_or_default();
    let input_delays = stroke
        .native_input_times
        .iter()
        .zip(raw_touch_times)
        .map(|(native, raw)| native.saturating_sub(*raw))
        .collect::<Vec<_>>();
    StrokeSummary {
        duration_ms: nanoseconds_to_milliseconds(finish_ns - stroke.begin_ns),
        finalization_duration_ms: nanoseconds_to_milliseconds(
            stroke.last_event_ns.saturating_sub(finish_ns),
        ),
        native_point_count: stroke.native_point_count.unwrap_or_default(),
        recorded_line_point_events: stroke.line_point_times.len(),
        raw_touch_frames: raw_touch_times.len(),
        input_to_native_delay: summarize_distribution(&input_delays),
        line_point_intervals: summarize_intervals(&stroke.line_point_times),
        live_triangle_calls: stroke.live_triangles,
        finalization_triangle_calls: stroke.finalization_triangles,
        live_display_updates: stroke.live_display_updates,
        finalization_display_updates: stroke.finalization_display_updates,
        finish_ribbon_calls: stroke.finish_ribbons,
        scene_to_view_scale: summarize_range(&stroke.scales),
        render_width_pixels: summarize_range(&stroke.render_widths),
        stored_width_quarters: summarize_range(&stroke.stored_widths),
    }
}

fn summarize_intervals(timestamps: &[u64]) -> IntervalSummary {
    let intervals = timestamps
        .windows(2)
        .map(|pair| pair[1].saturating_sub(pair[0]))
        .collect::<Vec<_>>();
    let distribution = summarize_distribution(&intervals);
    IntervalSummary {
        sample_count: timestamps.len(),
        interval_count: intervals.len(),
        mean_ms: distribution.mean_ms,
        median_ms: distribution.median_ms,
        percentile_95_ms: distribution.percentile_95_ms,
        maximum_ms: distribution.maximum_ms,
    }
}

fn summarize_distribution(values: &[u64]) -> DistributionSummary {
    if values.is_empty() {
        return DistributionSummary {
            sample_count: 0,
            mean_ms: 0.0,
            median_ms: 0.0,
            percentile_95_ms: 0.0,
            maximum_ms: 0.0,
        };
    }
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    let mean = sorted.iter().map(|value| *value as f64).sum::<f64>() / sorted.len() as f64;
    DistributionSummary {
        sample_count: sorted.len(),
        mean_ms: nanoseconds_to_milliseconds(mean as u64),
        median_ms: nanoseconds_to_milliseconds(percentile(&sorted, 50)),
        percentile_95_ms: nanoseconds_to_milliseconds(percentile(&sorted, 95)),
        maximum_ms: nanoseconds_to_milliseconds(*sorted.last().unwrap()),
    }
}

fn percentile(sorted: &[u64], percentage: usize) -> u64 {
    let index = ((sorted.len() - 1) * percentage).div_ceil(100);
    sorted[index]
}

fn summarize_range(values: &[f64]) -> RangeSummary {
    RangeSummary {
        minimum: values.iter().copied().reduce(f64::min).unwrap_or(0.0),
        maximum: values.iter().copied().reduce(f64::max).unwrap_or(0.0),
    }
}

fn nanoseconds_to_milliseconds(nanoseconds: u64) -> f64 {
    nanoseconds as f64 / 1_000_000.0
}

fn compare_drawing_surfaces(
    directory: &Path,
    metadata: &DrawingSurfaceMetadata,
) -> Result<DrawingDifference, String> {
    let before = fs::read(directory.join("drawing-before.bgra"))
        .map_err(|error| format!("cannot read drawing-before.bgra: {error}"))?;
    let after = fs::read(directory.join("drawing-after.bgra"))
        .map_err(|error| format!("cannot read drawing-after.bgra: {error}"))?;
    let expected = metadata.stride_pixels * metadata.height * 4;
    if before.len() != expected || after.len() != expected {
        return Err(format!(
            "drawing surfaces have lengths {} and {}, expected {expected}",
            before.len(),
            after.len()
        ));
    }

    let mut changed_pixels = 0;
    let mut absolute_channel_change = 0_u64;
    let mut maximum_channel_change = 0_u8;
    let mut bounds: Option<(usize, usize, usize, usize)> = None;
    for y in 0..metadata.height {
        for x in 0..metadata.width {
            let offset = (y * metadata.stride_pixels + x) * 4;
            let before_pixel = &before[offset..offset + 4];
            let after_pixel = &after[offset..offset + 4];
            if before_pixel == after_pixel {
                continue;
            }
            changed_pixels += 1;
            bounds = Some(bounds.map_or((x, y, x, y), |(left, top, right, bottom)| {
                (left.min(x), top.min(y), right.max(x), bottom.max(y))
            }));
            for channel in 0..4 {
                let difference = before_pixel[channel].abs_diff(after_pixel[channel]);
                absolute_channel_change += u64::from(difference);
                maximum_channel_change = maximum_channel_change.max(difference);
            }
        }
    }
    let changed_rectangle = bounds.map(|(left, top, right, bottom)| Rectangle {
        x: left,
        y: top,
        width: right - left + 1,
        height: bottom - top + 1,
    });
    Ok(DrawingDifference {
        changed_pixels,
        changed_rectangle,
        mean_absolute_channel_change: if changed_pixels == 0 {
            0.0
        } else {
            absolute_channel_change as f64 / (changed_pixels * 4) as f64
        },
        maximum_absolute_channel_change: maximum_channel_change,
    })
}
